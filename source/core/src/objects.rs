//! Page content objects: list / delete / move / recolour / change font of text
//! runs and images, insert vector shapes, and TRUE redaction (glyphs and
//! images under the box are removed from the content stream, not covered).
//!
//! All edits rebuild the page content into a NEW stream and only rewrite the
//! page dictionary (undo-safe, like `textedit`). Object ids are positional
//! ("t<op>" for a text show op, "i<op>" for an XObject draw) and are only
//! valid until the next edit — the UI re-lists after every change.

use std::collections::{BTreeMap, HashMap};

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as PdfDoc, Object, ObjectId, Stream, StringFormat};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, ErrorCode};
use crate::textedit::{font_info, interpret_page, interpret_with, mul, FontInfo, Interpreted, Item, PathObj, Show, M};

#[derive(Debug, Clone, Serialize)]
pub struct PageObject {
    pub id: String,
    /// "text" | "image" | "graphic"
    pub kind: String,
    pub text: String,
    /// User-space bounding box [x0, y0, x1, y1].
    pub rect: [f64; 4],
    /// Text only: font name without subset prefix (e.g. "Times-Bold").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Text only: effective size on the page, in points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Text only: fill colour as "#rrggbb" (when it is gray/RGB/CMYK).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// "#rrggbb" for the fill operators in effect (g / rg / k).
fn fill_hex(fill: &[Operation]) -> Option<String> {
    let f = |o: &Object| match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    };
    let op = fill.iter().rev().find(|o| matches!(o.operator.as_str(), "g" | "rg" | "k" | "sc" | "scn"))?;
    let v: Vec<f64> = op.operands.iter().filter_map(f).collect();
    let rgb = match (op.operator.as_str(), v.len()) {
        ("g", 1) | (_, 1) => [v[0]; 3],
        (_, 3) => [v[0], v[1], v[2]],
        (_, 4) => [(1.0 - v[0]) * (1.0 - v[3]), (1.0 - v[1]) * (1.0 - v[3]), (1.0 - v[2]) * (1.0 - v[3])],
        _ => return None,
    };
    Some(format!("#{:02x}{:02x}{:02x}", (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8, (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8, (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8))
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RedactReport {
    pub glyphs_removed: usize,
    pub images_removed: usize,
    /// Images/graphics only partly under the box: covered, not removed.
    pub images_covered: usize,
    pub annotations_removed: usize,
}

fn r(v: f64) -> Object {
    Object::Real(v as f32)
}

fn apply(m: &M, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

fn bbox(m: &M, x0: f64, y0: f64, x1: f64, y1: f64) -> [f64; 4] {
    let pts = [apply(m, x0, y0), apply(m, x1, y0), apply(m, x0, y1), apply(m, x1, y1)];
    [
        pts.iter().map(|p| p.0).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.1).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.0).fold(f64::MIN, f64::max),
        pts.iter().map(|p| p.1).fold(f64::MIN, f64::max),
    ]
}

fn inv(m: &M) -> Option<M> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return None;
    }
    Some([m[3] / det, -m[1] / det, -m[2] / det, m[0] / det, (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det])
}

fn intersects(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn inside(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] >= b[0] - 0.5 && a[1] >= b[1] - 0.5 && a[2] <= b[2] + 0.5 && a[3] <= b[3] + 0.5
}

fn norm(rect: [f64; 4]) -> [f64; 4] {
    [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])]
}

/// User-space box of one glyph of a show op.
fn glyph_box(s: &Show, x: f64, adv: f64) -> [f64; 4] {
    // Text space has the font size folded into the advances; heights scale
    // by size directly (typical ascent 0.78, descent 0.22).
    bbox(&s.trm, x, s.rise - 0.22 * s.size, x + adv.max(0.0), s.rise + 0.78 * s.size)
}

fn show_box(s: &Show) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for it in &s.items {
        if let Item::Glyph { x, adv, .. } = it {
            let g = glyph_box(s, *x, *adv);
            b = Some(match b {
                None => g,
                Some(a) => [a[0].min(g[0]), a[1].min(g[1]), a[2].max(g[2]), a[3].max(g[3])],
            });
        }
    }
    b
}

struct PageContent {
    page: ObjectId,
    ops: Vec<Operation>,
    fonts: BTreeMap<Vec<u8>, FontInfo>,
    parsed: Interpreted,
    #[allow(dead_code)]
    xobjects: HashMap<Vec<u8>, bool>, // name → is image
}

fn load(doc: &PdfDoc, page_no: u32) -> Result<PageContent, EngineError> {
    let page = *doc.get_pages().get(&page_no).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {page_no}"))?;
    let ops = doc.get_and_decode_page_content(page)?.operations;
    let fonts: BTreeMap<Vec<u8>, FontInfo> = doc.get_page_fonts(page)?.into_iter().map(|(k, d)| (k, font_info(doc, d))).collect();
    let parsed = interpret_page(doc, &ops, &fonts);
    let mut xobjects = HashMap::new();
    if let Some(Object::Dictionary(res)) = crate::model::inherited(doc, page, b"Resources") {
        let xo = match res.get(b"XObject") {
            Ok(Object::Dictionary(d)) => Some(d.clone()),
            Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).ok().cloned(),
            _ => None,
        };
        for (k, v) in xo.iter().flat_map(|d| d.iter()) {
            let sub = match v {
                Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_stream().ok()).and_then(|s| s.dict.get(b"Subtype").ok().and_then(|n| n.as_name().ok()).map(|n| n.to_vec())),
                _ => None,
            };
            xobjects.insert(k.clone(), sub.as_deref() == Some(b"Image"));
        }
    }
    Ok(PageContent { page, ops, fonts, parsed, xobjects })
}

/// Write `ops` back as the page's single (new) content stream.
fn store(doc: &mut PdfDoc, pc: &PageContent, ops: Vec<Operation>) -> Result<(), EngineError> {
    let data = Content { operations: ops }.encode()?;
    let mut s = Stream::new(dictionary! {}, data);
    let _ = s.compress();
    let id = doc.add_object(s);
    let mut page = doc.get_object(pc.page)?.as_dict()?.clone();
    page.set("Contents", Object::Reference(id));
    doc.objects.insert(pc.page, Object::Dictionary(page));
    Ok(())
}

// ------------------------------------------------- vector paths (page + forms)

fn path_delete(p: &PathObj, _ops: &[Operation], repl: &mut HashMap<usize, Vec<Operation>>) {
    for j in p.start..=p.op {
        repl.insert(j, vec![]);
    }
}

fn path_move(p: &PathObj, ops: &[Operation], repl: &mut HashMap<usize, Vec<Operation>>, dx: f64, dy: f64) {
    let Some(ci) = inv(&p.ctm) else { return };
    // CTM' = M × CTM = CTM × T  ⇒  M = CTM × T × CTM⁻¹ (a shift in page space).
    let m = mul(&mul(&p.ctm, &[1.0, 0.0, 0.0, 1.0, dx, dy]), &ci);
    repl.insert(p.start, vec![Operation::new("q", vec![]), Operation::new("cm", m.iter().map(|x| r(*x)).collect()), ops[p.start].clone()]);
    repl.insert(p.op, vec![ops[p.op].clone(), Operation::new("Q", vec![])]);
}

fn path_recolor(p: &PathObj, ops: &[Operation], repl: &mut HashMap<usize, Vec<Operation>>, rgb: [f64; 3]) {
    let c: Vec<Object> = rgb.iter().map(|v| r(*v)).collect();
    repl.insert(p.start, vec![Operation::new("q", vec![]), Operation::new("RG", c.clone()), Operation::new("rg", c), ops[p.start].clone()]);
    repl.insert(p.op, vec![ops[p.op].clone(), Operation::new("Q", vec![])]);
}

fn page_xobjects(doc: &PdfDoc, page: ObjectId) -> lopdf::Dictionary {
    match page_resources(doc, page).get(b"XObject") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => Default::default(),
    }
}

fn page_resources(doc: &PdfDoc, page: ObjectId) -> lopdf::Dictionary {
    match crate::model::inherited(doc, page, b"Resources") {
        Some(Object::Dictionary(d)) => d.clone(),
        Some(Object::Reference(r)) => doc.get_object(*r).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => Default::default(),
    }
}

fn sub_dict(doc: &PdfDoc, res: &lopdf::Dictionary, key: &[u8]) -> lopdf::Dictionary {
    match res.get(key) {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => Default::default(),
    }
}

/// Add XObject entries to the page's own /Resources.
fn add_xobjects(doc: &mut PdfDoc, page: ObjectId, entries: Vec<(Vec<u8>, ObjectId)>) -> Result<(), EngineError> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut res = page_resources(doc, page);
    let mut xo = page_xobjects(doc, page);
    for (k, id) in entries {
        xo.set(k, Object::Reference(id));
    }
    res.set("XObject", Object::Dictionary(xo));
    let mut p = doc.get_object(page)?.as_dict()?.clone();
    p.set("Resources", Object::Dictionary(res));
    doc.objects.insert(page, Object::Dictionary(p));
    Ok(())
}

/// Rebuild ops with per-index replacements.
fn rebuild(ops: &[Operation], repl: HashMap<usize, Vec<Operation>>) -> Vec<Operation> {
    let mut out = Vec::with_capacity(ops.len());
    for (i, op) in ops.iter().enumerate() {
        match repl.get(&i) {
            Some(v) => out.extend(v.iter().cloned()),
            None => out.push(op.clone()),
        }
    }
    out
}

// ------------------------------------------------- content scopes (page + groups)

/// Nesting depth of grouped drawings (form XObjects) opened for selection.
const MAX_DEPTH: usize = 4;

/// One content stream: the page itself (`chain` empty) or a form XObject
/// reached by the draw ops in `chain` (page op, then op inside that form, …).
struct Scope {
    chain: Vec<usize>,
    form: Option<ObjectId>,
    ops: Vec<Operation>,
    fonts: BTreeMap<Vec<u8>, FontInfo>,
    parsed: Interpreted,
    /// Resources in effect (a form without its own uses its parent's).
    resources: lopdf::Dictionary,
    /// True when `resources` is the form's own dictionary.
    own_resources: bool,
    /// CTM when this content starts drawing (page: identity).
    ctm0: M,
}

fn fonts_of(doc: &PdfDoc, res: &lopdf::Dictionary) -> BTreeMap<Vec<u8>, FontInfo> {
    sub_dict(doc, res, b"Font")
        .iter()
        .filter_map(|(k, v)| {
            let d = match v {
                Object::Reference(id) => doc.get_object(*id).and_then(Object::as_dict).ok()?,
                Object::Dictionary(d) => d,
                _ => return None,
            };
            Some((k.clone(), font_info(doc, d)))
        })
        .collect()
}

fn form_matrix(s: &Stream) -> M {
    match s.dict.get(b"Matrix").and_then(Object::as_array) {
        Ok(a) if a.len() == 6 => {
            let v: Vec<f64> = a.iter().filter_map(|o| o.as_float().ok().map(|f| f as f64)).collect();
            if v.len() == 6 { [v[0], v[1], v[2], v[3], v[4], v[5]] } else { [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] }
        }
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

/// The page and every grouped drawing on it (to MAX_DEPTH), parents first.
fn scopes(doc: &PdfDoc, pc: &PageContent) -> Vec<Scope> {
    let page_res = page_resources(doc, pc.page);
    let mut out = vec![Scope {
        chain: vec![],
        form: None,
        ops: pc.ops.clone(),
        fonts: pc.fonts.clone(),
        parsed: interpret_page(doc, &pc.ops, &pc.fonts),
        resources: page_res,
        own_resources: true,
        ctm0: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }];
    let mut i = 0;
    while i < out.len() {
        if out[i].chain.len() < MAX_DEPTH {
            let xo = sub_dict(doc, &out[i].resources, b"XObject");
            let mut kids = vec![];
            for d in &out[i].parsed.draws {
                let Ok(Object::Reference(id)) = xo.get(&d.name) else { continue };
                let Ok(Object::Stream(s)) = doc.get_object(*id) else { continue };
                if s.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form".as_slice()) {
                    continue;
                }
                // A form drawing itself (directly or not) is not followed.
                if out[i].form == Some(*id) || out.iter().any(|sc| sc.form == Some(*id) && out[i].chain.starts_with(&sc.chain)) {
                    continue;
                }
                let Ok(ops) = s.decompressed_content().and_then(|c| Content::decode(&c)).map(|c| c.operations) else { continue };
                let (resources, own) = match s.dict.get(b"Resources") {
                    Ok(Object::Dictionary(r)) => (r.clone(), true),
                    Ok(Object::Reference(r)) => (doc.get_object(*r).and_then(Object::as_dict).cloned().unwrap_or_default(), true),
                    _ => (out[i].resources.clone(), false),
                };
                let fonts = if own && resources.get(b"Font").is_ok() { fonts_of(doc, &resources) } else { out[i].fonts.clone() };
                let ctm0 = mul(&form_matrix(s), &d.ctm);
                let parsed = interpret_with(doc, &ops, &fonts, ctm0);
                let mut chain = out[i].chain.clone();
                chain.push(d.op);
                kids.push(Scope { chain, form: Some(*id), ops, fonts, parsed, resources, own_resources: own, ctm0 });
            }
            out.extend(kids);
        }
        i += 1;
    }
    out
}

/// "t12" (page) or "f12.4:t7" (op 7 inside the form drawn at op 4 inside the form drawn at page op 12).
fn parse_id(id: &str) -> Option<(Vec<usize>, char, usize)> {
    if let Some(rest) = id.strip_prefix('f') {
        let (chain, obj) = rest.split_once(':')?;
        let chain: Option<Vec<usize>> = chain.split('.').map(|c| c.parse().ok()).collect();
        let k = obj.chars().next()?;
        return Some((chain?, k, obj[1..].parse().ok()?));
    }
    let k = id.chars().next()?;
    Some((vec![], k, id[1..].parse().ok()?))
}

fn make_id(chain: &[usize], k: char, op: usize) -> String {
    if chain.is_empty() {
        format!("{k}{op}")
    } else {
        format!("f{}:{k}{op}", chain.iter().map(|c| c.to_string()).collect::<Vec<_>>().join("."))
    }
}

/// Pending changes to one scope.
#[derive(Default)]
struct Edits {
    repl: HashMap<usize, Vec<Operation>>,
    /// Replace the whole content (text edit) instead of per-op replacements.
    whole: Option<Vec<Operation>>,
    fonts: Vec<(Vec<u8>, Object)>,
    xobjects: Vec<(Vec<u8>, ObjectId)>,
}

impl Edits {
    fn content(self_repl: HashMap<usize, Vec<Operation>>, whole: Option<Vec<Operation>>, ops: &[Operation]) -> Vec<Operation> {
        match whole {
            Some(w) => w,
            None => rebuild(ops, self_repl),
        }
    }
}

/// Edit a text run inside a grouped drawing on the page (see textedit).
pub(crate) fn edit_text_in_groups(doc: &mut PdfDoc, page_no: u32, req: &crate::textedit::EditRequest, new_text: &str) -> Result<Option<crate::textedit::EditOutcome>, EngineError> {
    let pc = load(doc, page_no)?;
    let all = scopes(doc, &pc);
    for sc in all.iter().filter(|s| !s.chain.is_empty()) {
        if let Some(ce) = crate::textedit::edit_ops(doc, sc.ops.clone(), &sc.fonts, sc.ctm0, req, new_text)? {
            let mut edits = BTreeMap::new();
            edits.insert(sc.chain.clone(), Edits { whole: Some(ce.ops), fonts: ce.fonts, ..Default::default() });
            write_edits(doc, &pc, &all, edits)?;
            return Ok(Some(ce.outcome));
        }
    }
    Ok(None)
}

/// Apply `act` to every selected object, in whatever scope it lives, then
/// write the results back: changed groups are COPIED (copy-on-write, deepest
/// first) under new names used only by their parent, so other pages or
/// placements of the same group keep the original. Returns how many objects
/// `act` changed.
fn edit_objects(
    doc: &mut PdfDoc,
    page_no: u32,
    ids: &[String],
    act: &mut dyn FnMut(&mut PdfDoc, &Scope, char, usize, &mut Edits) -> Result<bool, EngineError>,
) -> Result<usize, EngineError> {
    let pc = load(doc, page_no)?;
    let all = scopes(doc, &pc);
    let mut edits: BTreeMap<Vec<usize>, Edits> = BTreeMap::new();
    let mut n = 0;
    for id in ids {
        let Some((chain, k, op)) = parse_id(id) else { continue };
        let Some(sc) = all.iter().find(|s| s.chain == chain) else { continue };
        let e = edits.entry(chain.clone()).or_default();
        if act(doc, sc, k, op, e)? {
            n += 1;
        }
    }
    if n == 0 {
        return Ok(0);
    }
    write_edits(doc, &pc, &all, edits)?;
    Ok(n)
}

/// Write scope edits back, deepest first: each changed form becomes a new
/// object and its parent's Do is renamed to it; finally the page.
fn write_edits(doc: &mut PdfDoc, pc: &PageContent, all: &[Scope], mut edits: BTreeMap<Vec<usize>, Edits>) -> Result<(), EngineError> {
    let mut chains: Vec<Vec<usize>> = edits.keys().cloned().collect();
    while let Some(chain) = {
        chains.sort_by_key(|c| std::cmp::Reverse(c.len()));
        chains.first().filter(|c| !c.is_empty()).cloned()
    } {
        chains.remove(0);
        let e = edits.remove(&chain).unwrap_or_default();
        let sc = all.iter().find(|s| s.chain == chain).expect("scope");
        let Ok(Object::Stream(orig)) = doc.get_object(sc.form.expect("form")).cloned() else { continue };
        let mut dict = orig.dict.clone();
        dict.remove(b"Filter");
        dict.remove(b"DecodeParms");
        if !e.fonts.is_empty() || !e.xobjects.is_empty() {
            let mut res = sc.resources.clone();
            let mut f = sub_dict(doc, &res, b"Font");
            for (k, v) in e.fonts {
                f.set(k, v);
            }
            res.set("Font", Object::Dictionary(f));
            let mut xo = sub_dict(doc, &res, b"XObject");
            for (k, v) in e.xobjects {
                xo.set(k, Object::Reference(v));
            }
            res.set("XObject", Object::Dictionary(xo));
            dict.set("Resources", Object::Dictionary(res));
        } else if !sc.own_resources {
            dict.set("Resources", Object::Dictionary(sc.resources.clone()));
        }
        let mut ns = Stream::new(dict, Content { operations: Edits::content(e.repl, e.whole, &sc.ops) }.encode()?);
        let _ = ns.compress();
        let nid = doc.add_object(ns);
        // Rename the Do in the parent.
        let parent_chain = chain[..chain.len() - 1].to_vec();
        let parent = all.iter().find(|s| s.chain == parent_chain).expect("parent scope");
        let draw = *chain.last().unwrap();
        let taken = sub_dict(doc, &parent.resources, b"XObject");
        let pe = edits.entry(parent_chain.clone()).or_default();
        let name = (1..).map(|k| format!("StFx{k}").into_bytes()).find(|k| taken.get(k).is_err() && !pe.xobjects.iter().any(|(a, _)| a == k)).unwrap();
        let old = parent.ops[draw].clone();
        let renamed = Operation::new("Do", vec![Object::Name(name.clone())]);
        match pe.repl.get_mut(&draw) {
            Some(v) => {
                for o in v.iter_mut() {
                    if o.operator == "Do" && o.operands == old.operands {
                        *o = renamed.clone();
                    }
                }
            }
            None => {
                pe.repl.insert(draw, vec![renamed]);
            }
        }
        pe.xobjects.push((name, nid));
        if !chains.contains(&parent_chain) {
            chains.push(parent_chain);
        }
    }
    // The page.
    let e = edits.remove(&Vec::new()).unwrap_or_default();
    if !e.repl.is_empty() || e.whole.is_some() {
        let ops = Edits::content(e.repl, e.whole, &pc.ops);
        store(doc, pc, ops)?;
    }
    if !e.fonts.is_empty() {
        let mut res = page_resources(doc, pc.page);
        let mut f = sub_dict(doc, &res, b"Font");
        for (k, v) in e.fonts {
            f.set(k, v);
        }
        res.set("Font", Object::Dictionary(f));
        let mut page = doc.get_object(pc.page)?.as_dict()?.clone();
        page.set("Resources", Object::Dictionary(res));
        doc.objects.insert(pc.page, Object::Dictionary(page));
    }
    add_xobjects(doc, pc.page, e.xobjects)?;
    Ok(())
}

fn object_entry(doc: &PdfDoc, sc: &Scope, out: &mut Vec<PageObject>, page_area: f64) {
    for s in &sc.parsed.shows {
        if let Some(b) = show_box(s) {
            if s.text.trim().is_empty() && s.decoded {
                continue;
            }
            let font = sc.fonts.get(&s.font).map(|f| {
                let n = f.base_font.as_str();
                // Drop a subset prefix such as "ABCDEF+".
                match n.split_once('+') {
                    Some((p, rest)) if p.len() == 6 && p.chars().all(|c| c.is_ascii_uppercase()) => rest.to_string(),
                    _ => n.to_string(),
                }
            });
            let scale = (s.trm[2] * s.trm[2] + s.trm[3] * s.trm[3]).sqrt();
            let size = ((s.size * scale * 10.0).round() / 10.0).abs();
            let color = if s.fill.is_empty() { Some("#000000".into()) } else { fill_hex(&s.fill) };
            out.push(PageObject { id: make_id(&sc.chain, 't', s.op), kind: "text".into(), text: s.text.clone(), rect: b, font, size: Some(size), color });
        }
    }
    // Vector paths (lines, rectangles, filled shapes) — not a full-page background.
    for p in &sc.parsed.paths {
        let b = p.bbox;
        if (b[2] - b[0]) * (b[3] - b[1]) > page_area * 0.9 {
            continue;
        }
        // Thin lines get at least a 3 pt target to click.
        let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        let (hw, hh) = (((b[2] - b[0]) / 2.0).max(1.5), ((b[3] - b[1]) / 2.0).max(1.5));
        out.push(PageObject { id: make_id(&sc.chain, 'p', p.op), kind: "path".into(), text: String::new(), rect: [cx - hw, cy - hh, cx + hw, cy + hh], font: None, size: None, color: None });
    }
    let xo = sub_dict(doc, &sc.resources, b"XObject");
    for d in &sc.parsed.draws {
        let image = match xo.get(&d.name) {
            Ok(Object::Reference(id)) => doc.get_object(*id).ok().and_then(|o| o.as_stream().ok()).and_then(|s| s.dict.get(b"Subtype").ok()).and_then(|n| n.as_name().ok()) == Some(b"Image".as_slice()),
            _ => false,
        };
        out.push(PageObject {
            id: make_id(&sc.chain, 'i', d.op),
            kind: if image { "image" } else { "graphic" }.into(),
            text: String::from_utf8_lossy(&d.name).into_owned(),
            rect: bbox(&d.ctm, 0.0, 0.0, 1.0, 1.0),
            font: None,
            size: None,
            color: None,
        });
    }
}

pub fn list(doc: &PdfDoc, page_no: u32) -> Result<Vec<PageObject>, EngineError> {
    let pc = load(doc, page_no)?;
    let pb = crate::docops::page_box(doc, pc.page);
    let page_area = (pb[2] - pb[0]) * (pb[3] - pb[1]);
    let mut out = vec![];
    for sc in scopes(doc, &pc) {
        object_entry(doc, &sc, &mut out, page_area);
    }
    Ok(out)
}

#[allow(dead_code)]
fn parse_ids(ids: &[String]) -> Vec<(char, usize)> {
    ids.iter().filter_map(|id| Some((id.chars().next()?, id[1..].parse().ok()?))).collect()
}

/// Ops that redraw a show op's operand as a plain TJ (keeping `'`/`"` semantics).
fn as_tj(op: &Operation, array: Vec<Object>) -> Vec<Operation> {
    let mut out = vec![];
    match op.operator.as_str() {
        "\"" => {
            out.push(Operation::new("Tw", vec![op.operands[0].clone()]));
            out.push(Operation::new("Tc", vec![op.operands[1].clone()]));
            out.push(Operation::new("T*", vec![]));
        }
        "'" => out.push(Operation::new("T*", vec![])),
        _ => {}
    }
    out.push(Operation::new("TJ", vec![Object::Array(array)]));
    out
}

/// The op's original strings/kerning as a TJ array.
fn original_array(op: &Operation) -> Vec<Object> {
    match op.operator.as_str() {
        "TJ" => op.operands.first().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default(),
        _ => op.operands.last().cloned().into_iter().collect(),
    }
}

/// TJ array keeping only glyphs for which `keep` is true; removed glyphs
/// become kerning of exactly their advance, so nothing else moves.
fn filtered_array(s: &Show, keep: impl Fn(&Item) -> bool) -> (Vec<Object>, usize) {
    let mut arr = vec![];
    let mut cur: Vec<u8> = vec![];
    let mut removed = 0;
    let fmt = if s.two_byte { StringFormat::Hexadecimal } else { StringFormat::Literal };
    let unit = s.size * s.th;
    let flush = |cur: &mut Vec<u8>, arr: &mut Vec<Object>| {
        if !cur.is_empty() {
            arr.push(Object::String(std::mem::take(cur), fmt));
        }
    };
    for it in &s.items {
        match it {
            Item::Glyph { bytes, adv, .. } => {
                if keep(it) {
                    cur.extend(bytes);
                } else {
                    flush(&mut cur, &mut arr);
                    removed += 1;
                    if unit.abs() > 1e-9 {
                        arr.push(r(-adv / unit * 1000.0));
                    }
                }
            }
            Item::Kern(k) => {
                flush(&mut cur, &mut arr);
                arr.push(r(*k));
            }
        }
    }
    flush(&mut cur, &mut arr);
    (arr, removed)
}

/// After rewriting a show op, put the text matrix back where the following op
/// expects it.
fn restore_tm(ops: &[Operation], s: &Show) -> Operation {
    let positioned = ops[s.op + 1..]
        .iter()
        .take_while(|o| !matches!(o.operator.as_str(), "Tj" | "TJ" | "'" | "\"" | "ET"))
        .any(|o| matches!(o.operator.as_str(), "Td" | "TD" | "Tm" | "T*"));
    let next_is_show = ops[s.op + 1..]
        .iter()
        .find(|o| matches!(o.operator.as_str(), "Tj" | "TJ" | "'" | "\"" | "ET"))
        .map(|o| o.operator != "ET")
        .unwrap_or(false);
    let m = if next_is_show && !positioned { s.tm_after } else { s.tlm };
    Operation::new("Tm", m.iter().map(|v| r(*v)).collect())
}

pub fn delete(doc: &mut PdfDoc, page_no: u32, ids: &[String]) -> Result<usize, EngineError> {
    let n = edit_objects(doc, page_no, ids, &mut |_, sc, k, i, e| {
        Ok(match k {
            't' => match sc.parsed.shows.iter().find(|s| s.op == i) {
                Some(s) => {
                    let (arr, _) = filtered_array(s, |_| false);
                    e.repl.insert(i, as_tj(&sc.ops[i], arr));
                    true
                }
                None => false,
            },
            'i' => sc.parsed.draws.iter().any(|d| d.op == i) && e.repl.insert(i, vec![]).is_none(),
            'p' => match sc.parsed.paths.iter().find(|p| p.op == i) {
                Some(p) => {
                    path_delete(p, &sc.ops, &mut e.repl);
                    true
                }
                None => false,
            },
            _ => false,
        })
    })?;
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "nothing selected"));
    }
    Ok(n)
}

pub fn translate(doc: &mut PdfDoc, page_no: u32, ids: &[String], dx: f64, dy: f64) -> Result<usize, EngineError> {
    let n = edit_objects(doc, page_no, ids, &mut |_, sc, k, i, e| {
        Ok(match k {
            't' => {
                let Some(s) = sc.parsed.shows.iter().find(|s| s.op == i) else { return Ok(false) };
                // Shift in user space → shift of the text matrix origin.
                let Some(ci) = inv(&s.ctm) else { return Ok(false) };
                let (ux, uy) = (dx * ci[0] + dy * ci[2], dx * ci[1] + dy * ci[3]);
                let mut tm = s.tm;
                tm[4] += ux;
                tm[5] += uy;
                let mut v = vec![];
                if sc.ops[i].operator == "\"" {
                    v.push(Operation::new("Tw", vec![sc.ops[i].operands[0].clone()]));
                    v.push(Operation::new("Tc", vec![sc.ops[i].operands[1].clone()]));
                }
                v.push(Operation::new("Tm", tm.iter().map(|x| r(*x)).collect()));
                v.push(Operation::new("TJ", vec![Object::Array(original_array(&sc.ops[i]))]));
                v.push(restore_tm(&sc.ops, s));
                e.repl.insert(i, v);
                true
            }
            'i' => {
                let Some(d) = sc.parsed.draws.iter().find(|d| d.op == i) else { return Ok(false) };
                let Some(ci) = inv(&d.ctm) else { return Ok(false) };
                // CTM' = M × CTM = CTM × T  ⇒  M = CTM × T × CTM⁻¹
                let m = mul(&mul(&d.ctm, &[1.0, 0.0, 0.0, 1.0, dx, dy]), &ci);
                e.repl.insert(i, vec![
                    Operation::new("q", vec![]),
                    Operation::new("cm", m.iter().map(|x| r(*x)).collect()),
                    sc.ops[i].clone(),
                    Operation::new("Q", vec![]),
                ]);
                true
            }
            'p' => match sc.parsed.paths.iter().find(|p| p.op == i) {
                Some(p) => {
                    path_move(p, &sc.ops, &mut e.repl, dx, dy);
                    true
                }
                None => false,
            },
            _ => false,
        })
    })?;
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "nothing selected"));
    }
    Ok(n)
}

/// Fill-colour ops to restore after a recoloured run (default black).
fn fill_restore(s: &Show) -> Vec<Operation> {
    if s.fill.is_empty() {
        vec![Operation::new("g", vec![0.into()])]
    } else {
        s.fill.clone()
    }
}

pub fn recolor(doc: &mut PdfDoc, page_no: u32, ids: &[String], rgb: [f64; 3]) -> Result<usize, EngineError> {
    let n = edit_objects(doc, page_no, ids, &mut |_, sc, k, i, e| {
        Ok(match k {
            // Shapes: outline and fill in the new colour.
            'p' => match sc.parsed.paths.iter().find(|p| p.op == i) {
                Some(p) => {
                    path_recolor(p, &sc.ops, &mut e.repl, rgb);
                    true
                }
                None => false,
            },
            't' => match sc.parsed.shows.iter().find(|s| s.op == i) {
                Some(s) => {
                    let mut v = vec![Operation::new("rg", vec![r(rgb[0]), r(rgb[1]), r(rgb[2])]), sc.ops[i].clone()];
                    v.extend(fill_restore(s));
                    e.repl.insert(i, v);
                    true
                }
                None => false,
            },
            _ => false,
        })
    })?;
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "select text or shapes to recolour (images keep their colours)"));
    }
    Ok(n)
}

/// Redraw text runs in a standard or installed font at `size` — the size as
/// seen on the page, in points (None = keep). `family` "" keeps each run's
/// own font and only changes its size.
pub fn set_font(doc: &mut PdfDoc, page_no: u32, ids: &[String], family: &str, size: Option<f64>) -> Result<usize, EngineError> {
    if family.is_empty() && size.is_none() {
        return Err(crate::err!(ErrorCode::ValidationFailed, "choose a font or a size"));
    }
    // Page size → Tf operand (text may be scaled by its matrix, e.g. Tf 1 + Tm 12).
    let tf_size = |s: &Show| match size {
        Some(v) => {
            let scale = (s.trm[2] * s.trm[2] + s.trm[3] * s.trm[3]).sqrt();
            if scale > 1e-6 { v / scale } else { v }
        }
        None => s.size,
    };
    let std14 = matches!(
        family,
        "Helvetica" | "Helvetica-Bold" | "Helvetica-Oblique" | "Helvetica-BoldOblique" | "Times-Roman" | "Times-Bold"
            | "Times-Italic" | "Times-BoldItalic" | "Courier" | "Courier-Bold" | "Courier-Oblique" | "Courier-BoldOblique"
    );
    // Otherwise: an installed font, by PostScript name (embedded as a subset).
    let system = if std14 || family.is_empty() {
        None
    } else {
        Some(crate::fonts::find_face(family).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "font {family} is not installed"))?)
    };
    let mut taken: HashMap<Vec<usize>, std::collections::HashSet<Vec<u8>>> = HashMap::new();
    let n = edit_objects(doc, page_no, ids, &mut |doc, sc, k, i, e| {
        if k != 't' {
            return Ok(false);
        }
        let mut v = vec![];
        if sc.ops[i].operator == "'" || sc.ops[i].operator == "\"" {
            v.push(Operation::new("T*", vec![]));
        }
        if family.is_empty() {
            // Size only: same font, same bytes.
            let Some(s) = sc.parsed.shows.iter().find(|s| s.op == i) else { return Ok(false) };
            v.push(Operation::new("Tf", vec![Object::Name(s.font.clone()), r(tf_size(s))]));
            let mut show = sc.ops[i].clone();
            if show.operator == "'" || show.operator == "\"" {
                let last = show.operands.last().cloned().unwrap_or(Object::Null);
                show = Operation::new("Tj", vec![last]);
            }
            v.push(show);
            v.push(Operation::new("Tf", vec![Object::Name(s.font.clone()), r(s.size)]));
            v.push(restore_tm(&sc.ops, s));
            e.repl.insert(i, v);
            return Ok(true);
        }
        let Some(s) = sc.parsed.shows.iter().find(|s| s.op == i && s.decoded) else { return Ok(false) };
        let used = taken.entry(sc.chain.clone()).or_insert_with(|| sub_dict(doc, &sc.resources, b"Font").iter().map(|(k, _)| k.clone()).collect());
        let mut name = || {
            let n = (1..).map(|n| format!("StFn{n}").into_bytes()).find(|n| !used.contains(n)).unwrap();
            used.insert(n.clone());
            n
        };
        match system {
            Some(face) => {
                // The chosen installed font; letters it lacks from a fallback.
                let lower = face.family.to_ascii_lowercase();
                let serif = lower.contains("times") || lower.contains("serif") || lower.contains("georgia");
                for (f, piece) in crate::fonts::visual_font_runs(&s.text, face, serif, face.monospace, face.bold, face.italic) {
                    let em = crate::fonts::embed_text(doc, f, &piece, tf_size(s), s.rise)?;
                    let n = name();
                    e.fonts.push((n.clone(), Object::Reference(em.font)));
                    v.push(Operation::new("Tf", vec![Object::Name(n), r(tf_size(s))]));
                    v.extend(em.ops);
                }
            }
            None => {
                // Letters the standard font lacks (Cyrillic…) → installed look-alikes.
                let orig = sc.fonts.get(&s.font).map(|f| f.base_font.clone()).unwrap_or_default();
                let rep = crate::textedit::replacement_text(doc, &orig, family, &s.text, tf_size(s), s.rise, &mut name);
                e.fonts.extend(rep.fonts);
                v.extend(rep.ops);
            }
        }
        v.push(Operation::new("Tf", vec![Object::Name(s.font.clone()), r(s.size)]));
        v.push(restore_tm(&sc.ops, s));
        e.repl.insert(i, v);
        Ok(true)
    })?;
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "select text to change its font"));
    }
    Ok(n)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShapeSpec {
    /// "rect" | "oval" | "line" | "arrow"
    pub kind: String,
    /// User space; for lines: start = (x0,y0), end = (x1,y1).
    pub rect: [f64; 4],
    pub stroke: [f64; 3],
    pub fill: Option<[f64; 3]>,
    pub width: f64,
}

/// Draw a vector shape into the page content (not an annotation).
pub fn insert_shape(doc: &mut PdfDoc, page_no: u32, spec: &ShapeSpec) -> Result<(), EngineError> {
    let page = *doc.get_pages().get(&page_no).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {page_no}"))?;
    let w = spec.width.clamp(0.25, 24.0);
    let mut ops = vec![
        Operation::new("q", vec![]),
        Operation::new("w", vec![r(w)]),
        Operation::new("J", vec![1.into()]),
        Operation::new("j", vec![1.into()]),
        Operation::new("RG", spec.stroke.iter().map(|v| r(*v)).collect()),
    ];
    if let Some(f) = spec.fill {
        ops.push(Operation::new("rg", f.iter().map(|v| r(*v)).collect()));
    }
    let paint = if spec.fill.is_some() { "B" } else { "S" };
    match spec.kind.as_str() {
        "rect" | "oval" => {
            let [x0, y0, x1, y1] = norm(spec.rect);
            if spec.kind == "rect" {
                ops.push(Operation::new("re", vec![r(x0), r(y0), r(x1 - x0), r(y1 - y0)]));
            } else {
                let (rx, ry, cx, cy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0, (x0 + x1) / 2.0, (y0 + y1) / 2.0);
                let k = 0.552_284_75;
                ops.extend([
                    Operation::new("m", vec![r(cx + rx), r(cy)]),
                    Operation::new("c", vec![r(cx + rx), r(cy + k * ry), r(cx + k * rx), r(cy + ry), r(cx), r(cy + ry)]),
                    Operation::new("c", vec![r(cx - k * rx), r(cy + ry), r(cx - rx), r(cy + k * ry), r(cx - rx), r(cy)]),
                    Operation::new("c", vec![r(cx - rx), r(cy - k * ry), r(cx - k * rx), r(cy - ry), r(cx), r(cy - ry)]),
                    Operation::new("c", vec![r(cx + k * rx), r(cy - ry), r(cx + rx), r(cy - k * ry), r(cx + rx), r(cy)]),
                    Operation::new("h", vec![]),
                ]);
            }
            ops.push(Operation::new(paint, vec![]));
        }
        "line" | "arrow" => {
            let [x0, y0, x1, y1] = spec.rect;
            ops.extend([Operation::new("m", vec![r(x0), r(y0)]), Operation::new("l", vec![r(x1), r(y1)]), Operation::new("S", vec![])]);
            if spec.kind == "arrow" {
                let ang = (y1 - y0).atan2(x1 - x0);
                let len = w * 3.0 + 6.0;
                ops.extend([
                    Operation::new("m", vec![r(x1 + len * (ang + 2.6).cos()), r(y1 + len * (ang + 2.6).sin())]),
                    Operation::new("l", vec![r(x1), r(y1)]),
                    Operation::new("l", vec![r(x1 + len * (ang - 2.6).cos()), r(y1 + len * (ang - 2.6).sin())]),
                    Operation::new("S", vec![]),
                ]);
            }
        }
        other => return Err(crate::err!(ErrorCode::ValidationFailed, "unknown shape {other}")),
    }
    ops.push(Operation::new("Q", vec![]));
    crate::docops::append_to_page(doc, page, ops, vec![], vec![])
}

/// TRUE redaction of `rect` (user space) on a page.
pub fn redact(doc: &mut PdfDoc, page_no: u32, rect: [f64; 4]) -> Result<RedactReport, EngineError> {
    let rect = norm(rect);
    if rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "drag a larger area to redact"));
    }
    let pc = load(doc, page_no)?;
    let mut report = RedactReport::default();
    let mut repl = HashMap::new();
    for s in &pc.parsed.shows {
        let hit = |it: &Item| match it {
            Item::Glyph { x, adv, .. } => intersects(&glyph_box(s, *x, *adv), &rect),
            Item::Kern(_) => false,
        };
        if !s.items.iter().any(&hit) {
            continue;
        }
        let (arr, removed) = filtered_array(s, |it| !hit(it));
        report.glyphs_removed += removed;
        repl.insert(s.op, as_tj(&pc.ops[s.op], arr));
    }
    for d in &pc.parsed.draws {
        let b = bbox(&d.ctm, 0.0, 0.0, 1.0, 1.0);
        if !intersects(&b, &rect) {
            continue;
        }
        if inside(&b, &rect) {
            repl.insert(d.op, vec![]);
            report.images_removed += 1;
        } else {
            report.images_covered += 1;
        }
    }
    let ops = rebuild(&pc.ops, repl);
    store(doc, &pc, ops)?;
    // Annotations touching the area go too (their text could reveal content).
    for (id, _, a) in crate::annots::rects(doc, page_no) {
        if intersects(&a, &rect) {
            crate::annots::delete(doc, page_no, id)?;
            report.annotations_removed += 1;
        }
    }
    let page = pc.page;
    crate::docops::append_to_page(
        doc,
        page,
        vec![
            Operation::new("q", vec![]),
            Operation::new("g", vec![0.into()]),
            Operation::new("re", vec![r(rect[0]), r(rect[1]), r(rect[2] - rect[0]), r(rect[3] - rect[1])]),
            Operation::new("f", vec![]),
            Operation::new("Q", vec![]),
        ],
        vec![],
        vec![],
    )?;
    Ok(report)
}

// ------------------------------------------------------------------ images

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum ImageData {
    /// Original JPEG bytes (embedded as-is: DCTDecode, no re-compression).
    Jpeg { bytes: Vec<u8> },
    /// Decoded pixels, 8-bit RGBA, row-major.
    Rgba { width: u32, height: u32, pixels: Vec<u8> },
}

/// (width, height, components) from a JPEG's SOF marker.
fn jpeg_info(b: &[u8]) -> Option<(u32, u32, u8)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h, b[i + 9]));
        }
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        i += 2 + len;
    }
    None
}

/// Place an image on a page inside `rect` (user space). Returns its size in pixels.
pub fn insert_image(doc: &mut PdfDoc, page_no: u32, rect: [f64; 4], img: &ImageData) -> Result<(u32, u32), EngineError> {
    let page = *doc.get_pages().get(&page_no).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {page_no}"))?;
    let rect = norm(rect);
    if rect[2] - rect[0] < 1.0 || rect[3] - rect[1] < 1.0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "image area too small"));
    }
    let (xobj, w, h) = match img {
        ImageData::Jpeg { bytes } => {
            let (w, h, comps) = jpeg_info(bytes).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "not a readable JPEG"))?;
            let cs = match comps {
                1 => "DeviceGray",
                4 => "DeviceCMYK",
                _ => "DeviceRGB",
            };
            let mut d = dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
                "ColorSpace" => Object::Name(cs.as_bytes().to_vec()), "BitsPerComponent" => 8, "Filter" => "DCTDecode",
            };
            if comps == 4 {
                // Adobe CMYK JPEGs are stored inverted.
                d.set("Decode", vec![1.into(), 0.into(), 1.into(), 0.into(), 1.into(), 0.into(), 1.into(), 0.into()]);
            }
            (doc.add_object(Stream::new(d, bytes.clone()).with_compression(false)), w, h)
        }
        ImageData::Rgba { width, height, pixels } => {
            let (w, h) = (*width, *height);
            if pixels.len() != (w * h * 4) as usize || w == 0 || h == 0 {
                return Err(crate::err!(ErrorCode::ValidationFailed, "image data size mismatch"));
            }
            let rgb: Vec<u8> = pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            let alpha: Vec<u8> = pixels.chunks(4).map(|p| p[3]).collect();
            let mut d = dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
                "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
            };
            if alpha.iter().any(|&a| a != 255) {
                let mut sm = Stream::new(
                    dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
                    alpha,
                );
                let _ = sm.compress();
                let smask = doc.add_object(sm);
                d.set("SMask", smask);
            }
            let mut s = Stream::new(d, rgb);
            let _ = s.compress();
            (doc.add_object(s), w, h)
        }
    };
    // Register the XObject in the page resources, then draw it.
    let mut res = match crate::model::inherited(doc, page, b"Resources") {
        Some(Object::Dictionary(d)) => d.clone(),
        _ => lopdf::Dictionary::new(),
    };
    let mut xo = match res.get(b"XObject") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => lopdf::Dictionary::new(),
    };
    let name = (1..).map(|n| format!("StIm{n}").into_bytes()).find(|n| !xo.has(n)).unwrap();
    xo.set(name.clone(), xobj);
    res.set("XObject", Object::Dictionary(xo));
    let mut pd = doc.get_object(page)?.as_dict()?.clone();
    pd.set("Resources", Object::Dictionary(res));
    doc.objects.insert(page, Object::Dictionary(pd));
    crate::docops::append_to_page(
        doc,
        page,
        vec![
            Operation::new("q", vec![]),
            Operation::new("cm", vec![r(rect[2] - rect[0]), r(0.0), r(0.0), r(rect[3] - rect[1]), r(rect[0]), r(rect[1])]),
            Operation::new("Do", vec![Object::Name(name)]),
            Operation::new("Q", vec![]),
        ],
        vec![],
        vec![],
    )?;
    Ok((w, h))
}
