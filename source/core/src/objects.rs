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
use crate::textedit::{font_info, interpret_page, mul, FontInfo, Interpreted, Item, Show, M};

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

pub fn list(doc: &PdfDoc, page_no: u32) -> Result<Vec<PageObject>, EngineError> {
    let pc = load(doc, page_no)?;
    let mut out = vec![];
    for s in &pc.parsed.shows {
        if let Some(b) = show_box(s) {
            if s.text.trim().is_empty() && s.decoded {
                continue;
            }
            let font = pc.fonts.get(&s.font).map(|f| {
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
            out.push(PageObject { id: format!("t{}", s.op), kind: "text".into(), text: s.text.clone(), rect: b, font, size: Some(size), color });
        }
    }
    // Vector paths (lines, rectangles, filled shapes) — not a full-page background.
    let pb = crate::docops::page_box(doc, pc.page);
    let page_area = (pb[2] - pb[0]) * (pb[3] - pb[1]);
    for p in &pc.parsed.paths {
        let b = p.bbox;
        if (b[2] - b[0]) * (b[3] - b[1]) > page_area * 0.9 {
            continue;
        }
        // Thin lines get at least a 3 pt target to click.
        let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        let (hw, hh) = (((b[2] - b[0]) / 2.0).max(1.5), ((b[3] - b[1]) / 2.0).max(1.5));
        out.push(PageObject { id: format!("p{}", p.op), kind: "path".into(), text: String::new(), rect: [cx - hw, cy - hh, cx + hw, cy + hh], font: None, size: None, color: None });
    }
    for d in &pc.parsed.draws {
        let image = pc.xobjects.get(&d.name).copied().unwrap_or(false);
        out.push(PageObject {
            id: format!("i{}", d.op),
            kind: if image { "image" } else { "graphic" }.into(),
            text: String::from_utf8_lossy(&d.name).into_owned(),
            rect: bbox(&d.ctm, 0.0, 0.0, 1.0, 1.0),
            font: None,
            size: None,
            color: None,
        });
    }
    Ok(out)
}

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
fn restore_tm(pc: &PageContent, s: &Show) -> Operation {
    let positioned = pc.ops[s.op + 1..]
        .iter()
        .take_while(|o| !matches!(o.operator.as_str(), "Tj" | "TJ" | "'" | "\"" | "ET"))
        .any(|o| matches!(o.operator.as_str(), "Td" | "TD" | "Tm" | "T*"));
    let next_is_show = pc.ops[s.op + 1..]
        .iter()
        .find(|o| matches!(o.operator.as_str(), "Tj" | "TJ" | "'" | "\"" | "ET"))
        .map(|o| o.operator != "ET")
        .unwrap_or(false);
    let m = if next_is_show && !positioned { s.tm_after } else { s.tlm };
    Operation::new("Tm", m.iter().map(|v| r(*v)).collect())
}

pub fn delete(doc: &mut PdfDoc, page_no: u32, ids: &[String]) -> Result<usize, EngineError> {
    let pc = load(doc, page_no)?;
    let mut repl = HashMap::new();
    for (k, i) in parse_ids(ids) {
        match k {
            't' => {
                if let Some(s) = pc.parsed.shows.iter().find(|s| s.op == i) {
                    let (arr, _) = filtered_array(s, |_| false);
                    repl.insert(i, as_tj(&pc.ops[i], arr));
                }
            }
            'i' => {
                repl.insert(i, vec![]);
            }
            'p' => {
                if let Some(p) = pc.parsed.paths.iter().find(|p| p.op == i) {
                    for j in p.start..=p.op {
                        repl.insert(j, vec![]);
                    }
                }
            }
            _ => {}
        }
    }
    let n = repl.len();
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "nothing selected"));
    }
    let ops = rebuild(&pc.ops, repl);
    store(doc, &pc, ops)?;
    Ok(n)
}

pub fn translate(doc: &mut PdfDoc, page_no: u32, ids: &[String], dx: f64, dy: f64) -> Result<usize, EngineError> {
    let pc = load(doc, page_no)?;
    let mut repl = HashMap::new();
    for (k, i) in parse_ids(ids) {
        match k {
            't' => {
                let Some(s) = pc.parsed.shows.iter().find(|s| s.op == i) else { continue };
                // Shift in user space → shift of the text matrix origin.
                let Some(ci) = inv(&s.ctm) else { continue };
                let (ux, uy) = (dx * ci[0] + dy * ci[2], dx * ci[1] + dy * ci[3]);
                let mut tm = s.tm;
                tm[4] += ux;
                tm[5] += uy;
                let mut v = vec![];
                if pc.ops[i].operator == "\"" {
                    v.push(Operation::new("Tw", vec![pc.ops[i].operands[0].clone()]));
                    v.push(Operation::new("Tc", vec![pc.ops[i].operands[1].clone()]));
                }
                v.push(Operation::new("Tm", tm.iter().map(|x| r(*x)).collect()));
                v.push(Operation::new("TJ", vec![Object::Array(original_array(&pc.ops[i]))]));
                v.push(restore_tm(&pc, s));
                repl.insert(i, v);
            }
            'i' => {
                let Some(d) = pc.parsed.draws.iter().find(|d| d.op == i) else { continue };
                let Some(ci) = inv(&d.ctm) else { continue };
                // CTM' = M × CTM = CTM × T  ⇒  M = CTM × T × CTM⁻¹
                let m = mul(&mul(&d.ctm, &[1.0, 0.0, 0.0, 1.0, dx, dy]), &ci);
                repl.insert(i, vec![
                    Operation::new("q", vec![]),
                    Operation::new("cm", m.iter().map(|x| r(*x)).collect()),
                    pc.ops[i].clone(),
                    Operation::new("Q", vec![]),
                ]);
            }
            'p' => {
                let Some(p) = pc.parsed.paths.iter().find(|p| p.op == i) else { continue };
                let Some(ci) = inv(&p.ctm) else { continue };
                let m = mul(&mul(&p.ctm, &[1.0, 0.0, 0.0, 1.0, dx, dy]), &ci);
                repl.insert(p.start, vec![Operation::new("q", vec![]), Operation::new("cm", m.iter().map(|x| r(*x)).collect()), pc.ops[p.start].clone()]);
                repl.insert(p.op, vec![pc.ops[p.op].clone(), Operation::new("Q", vec![])]);
            }
            _ => {}
        }
    }
    let n = repl.len();
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "nothing selected"));
    }
    let ops = rebuild(&pc.ops, repl);
    store(doc, &pc, ops)?;
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
    let pc = load(doc, page_no)?;
    let mut repl = HashMap::new();
    for (k, i) in parse_ids(ids) {
        if k == 'p' {
            // Shapes: outline and fill in the new colour.
            let Some(p) = pc.parsed.paths.iter().find(|p| p.op == i) else { continue };
            let c: Vec<Object> = rgb.iter().map(|v| r(*v)).collect();
            repl.insert(p.start, vec![Operation::new("q", vec![]), Operation::new("RG", c.clone()), Operation::new("rg", c), pc.ops[p.start].clone()]);
            repl.insert(p.op, vec![pc.ops[p.op].clone(), Operation::new("Q", vec![])]);
            continue;
        }
        if k != 't' {
            continue;
        }
        let Some(s) = pc.parsed.shows.iter().find(|s| s.op == i) else { continue };
        let mut v = vec![Operation::new("rg", vec![r(rgb[0]), r(rgb[1]), r(rgb[2])]), pc.ops[i].clone()];
        v.extend(fill_restore(s));
        repl.insert(i, v);
    }
    let n = repl.len();
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "select text or shapes to recolour (images keep their colours)"));
    }
    let ops = rebuild(&pc.ops, repl);
    store(doc, &pc, ops)?;
    Ok(n)
}

/// Redraw text runs in a standard font family at `size` — the size as seen
/// on the page, in points (None = keep). `family` "" keeps each run's own
/// font and only changes its size.
pub fn set_font(doc: &mut PdfDoc, page_no: u32, ids: &[String], family: &str, size: Option<f64>) -> Result<usize, EngineError> {
    let pc = load(doc, page_no)?;
    // Page size → Tf operand (text may be scaled by its matrix, e.g. Tf 1 + Tm 12).
    let tf_size = |s: &crate::textedit::Show| match size {
        Some(v) => {
            let scale = (s.trm[2] * s.trm[2] + s.trm[3] * s.trm[3]).sqrt();
            if scale > 1e-6 { v / scale } else { v }
        }
        None => s.size,
    };
    if family.is_empty() {
        let Some(_) = size else {
            return Err(crate::err!(ErrorCode::ValidationFailed, "choose a font or a size"));
        };
        let mut repl = HashMap::new();
        for (k, i) in parse_ids(ids) {
            if k != 't' {
                continue;
            }
            let Some(s) = pc.parsed.shows.iter().find(|s| s.op == i) else { continue };
            let mut v = vec![];
            if pc.ops[i].operator == "'" || pc.ops[i].operator == "\"" {
                v.push(Operation::new("T*", vec![]));
            }
            v.push(Operation::new("Tf", vec![Object::Name(s.font.clone()), r(tf_size(s))]));
            let mut show = pc.ops[i].clone();
            if show.operator == "'" || show.operator == "\"" {
                let last = show.operands.last().cloned().unwrap_or(Object::Null);
                show = Operation::new("Tj", vec![last]);
            }
            v.push(show);
            v.push(Operation::new("Tf", vec![Object::Name(s.font.clone()), r(s.size)]));
            v.push(restore_tm(&pc, s));
            repl.insert(i, v);
        }
        let n = repl.len();
        if n == 0 {
            return Err(crate::err!(ErrorCode::ValidationFailed, "select text to change its size"));
        }
        let ops = rebuild(&pc.ops, repl);
        store(doc, &pc, ops)?;
        return Ok(n);
    }
    let std14 = matches!(
        family,
        "Helvetica" | "Helvetica-Bold" | "Helvetica-Oblique" | "Helvetica-BoldOblique" | "Times-Roman" | "Times-Bold"
            | "Times-Italic" | "Times-BoldItalic" | "Courier" | "Courier-Bold" | "Courier-Oblique" | "Courier-BoldOblique"
    );
    // Otherwise: an installed font, by PostScript name (embedded as a subset).
    let system = if std14 {
        None
    } else {
        Some(crate::fonts::find_face(family).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "font {family} is not installed"))?)
    };
    let base = family;
    let mut repl = HashMap::new();
    let mut new_fonts: Vec<(Vec<u8>, Object)> = vec![];
    let mut taken: std::collections::HashSet<Vec<u8>> = pc.fonts.keys().cloned().collect();
    for (k, i) in parse_ids(ids) {
        if k != 't' {
            continue;
        }
        let Some(s) = pc.parsed.shows.iter().find(|s| s.op == i && s.decoded) else { continue };
        let name = (1..).map(|n| format!("StFn{n}").into_bytes()).find(|n| !taken.contains(n)).unwrap();
        taken.insert(name.clone());
        let shown = match system {
            Some(face) => {
                let e = crate::fonts::embed_text(doc, face, &s.text)?;
                new_fonts.push((name.clone(), Object::Reference(e.font)));
                crate::fonts::hex_string(e.bytes)
            }
            None => {
                // Letters the standard font lacks (Cyrillic…) → an installed look-alike.
                let orig = pc.fonts.get(&s.font).map(|f| f.base_font.clone()).unwrap_or_default();
                let (font, shown, _, _) = crate::textedit::replacement_font(doc, &orig, base, &s.text);
                new_fonts.push((name.clone(), font));
                shown
            }
        };
        let mut v = vec![];
        if pc.ops[i].operator == "'" || pc.ops[i].operator == "\"" {
            v.push(Operation::new("T*", vec![]));
        }
        v.push(Operation::new("Tf", vec![Object::Name(name), r(tf_size(s))]));
        v.push(Operation::new("Tj", vec![shown]));
        v.push(Operation::new("Tf", vec![Object::Name(s.font.clone()), r(s.size)]));
        v.push(restore_tm(&pc, s));
        repl.insert(i, v);
    }
    let n = repl.len();
    if n == 0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "select text to change its font"));
    }
    let ops = rebuild(&pc.ops, repl);
    store(doc, &pc, ops)?;
    // Register the new fonts in the page resources.
    let page = doc.get_object(pc.page)?.as_dict()?.clone();
    let mut res = match page.get(b"Resources") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => lopdf::Dictionary::new(),
    };
    let mut f = match res.get(b"Font") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => lopdf::Dictionary::new(),
    };
    for (k, v) in new_fonts {
        f.set(k, v);
    }
    res.set("Font", Object::Dictionary(f));
    let mut page = doc.get_object(pc.page)?.as_dict()?.clone();
    page.set("Resources", Object::Dictionary(res));
    doc.objects.insert(pc.page, Object::Dictionary(page));
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
