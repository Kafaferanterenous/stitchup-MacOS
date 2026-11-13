//! Whole-document operations: new blank document, watermark, crop boxes,
//! encrypted / compressed copies, embedded-file attachments.
//!
//! Like `textedit`, mutating ops never modify existing stream objects: they
//! add new objects and only rewrite page dictionaries (which `oplog` snapshots),
//! so undo/redo stays exact.

use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::content::{Content, Operation};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{dictionary, Dictionary, Document as PdfDoc, EncryptionState, EncryptionVersion, Object, ObjectId, Permissions, SaveOptions, Stream, StringFormat};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, ErrorCode};
use crate::model::inherited;

// ------------------------------------------------------------------ new

/// A one-page blank document of the given size (points).
pub fn new_blank(width_pt: f64, height_pt: f64) -> Result<PdfDoc, EngineError> {
    if !(width_pt > 0.0 && height_pt > 0.0) {
        return Err(crate::err!(ErrorCode::ValidationFailed, "page size must be positive"));
    }
    let mut doc = PdfDoc::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => Dictionary::new(),
        "MediaBox" => vec![0.into(), 0.into(), Object::Real(width_pt as f32), Object::Real(height_pt as f32)],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let info = doc.add_object(dictionary! { "Producer" => Object::string_literal("Stitchup") });
    doc.trailer.set("Info", info);
    Ok(doc)
}

// ------------------------------------------------------------ geometry

/// Visible box of a page: CropBox if present, else MediaBox (inherited ok).
pub fn page_box(doc: &PdfDoc, page: ObjectId) -> [f64; 4] {
    let arr = inherited(doc, page, b"CropBox")
        .or_else(|| inherited(doc, page, b"MediaBox"))
        .and_then(|o| o.as_array().ok().cloned())
        .unwrap_or_default();
    let n = |i: usize| arr.get(i).and_then(|o| o.as_float().ok()).map(|v| v as f64);
    match (n(0), n(1), n(2), n(3)) {
        (Some(a), Some(b), Some(c), Some(d)) => [a.min(c), b.min(d), a.max(c), b.max(d)],
        _ => [0.0, 0.0, 612.0, 792.0],
    }
}

fn rotation(doc: &PdfDoc, page: ObjectId) -> i64 {
    inherited(doc, page, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0).rem_euclid(360)
}

/// Displayed (rotated) width/height plus the `cm` matrix that maps displayed
/// coordinates (origin bottom-left of what the reader sees) to user space.
pub fn display_space(doc: &PdfDoc, page: ObjectId) -> (f64, f64, [f64; 6]) {
    let [x0, y0, x1, y1] = page_box(doc, page);
    let (w, h) = (x1 - x0, y1 - y0);
    match rotation(doc, page) {
        90 => (h, w, [0.0, 1.0, -1.0, 0.0, x0 + w, y0]),
        180 => (w, h, [-1.0, 0.0, 0.0, -1.0, x0 + w, y0 + h]),
        270 => (h, w, [0.0, -1.0, 1.0, 0.0, x0, y0 + h]),
        _ => (w, h, [1.0, 0.0, 0.0, 1.0, x0, y0]),
    }
}

// --------------------------------------------------------- page content

/// Unique resource name `<prefix>N` not yet used in `dict`.
fn unique_name(dict: &Dictionary, prefix: &str) -> Vec<u8> {
    (1..).map(|i| format!("{prefix}{i}").into_bytes()).find(|n| !dict.has(n)).unwrap()
}

/// Owned copy of a (possibly referenced) sub-dictionary of `res`.
fn sub_dict(doc: &PdfDoc, res: &Dictionary, key: &[u8]) -> Dictionary {
    match res.get(key) {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    }
}

/// Page resources as an owned dictionary (inline, referenced or inherited).
fn resources_of(doc: &PdfDoc, page: ObjectId) -> Dictionary {
    match inherited(doc, page, b"Resources") {
        Some(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    }
}

/// Draw `ops` on top of the page. The existing content is wrapped in q/Q so
/// any graphics state it leaves behind can't leak into the new drawing.
/// `fonts` / `ext_g` are (resource name, object) pairs merged into Resources.
pub fn append_to_page(
    doc: &mut PdfDoc,
    page: ObjectId,
    ops: Vec<Operation>,
    fonts: Vec<(Vec<u8>, Object)>,
    ext_g: Vec<(Vec<u8>, Object)>,
) -> Result<(), EngineError> {
    let mut res = resources_of(doc, page);
    if !fonts.is_empty() {
        let mut f = sub_dict(doc, &res, b"Font");
        for (k, v) in fonts {
            f.set(k, v);
        }
        res.set("Font", Object::Dictionary(f));
    }
    if !ext_g.is_empty() {
        let mut g = sub_dict(doc, &res, b"ExtGState");
        for (k, v) in ext_g {
            g.set(k, v);
        }
        res.set("ExtGState", Object::Dictionary(g));
    }
    let q = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let data = Content { operations: [vec![Operation::new("Q", vec![])], ops].concat() }.encode()?;
    let mut s = Stream::new(dictionary! {}, data);
    let _ = s.compress();
    let tail = doc.add_object(s);
    let page_dict = doc.get_object(page)?.as_dict()?.clone();
    let mut contents = vec![Object::Reference(q)];
    match page_dict.get(b"Contents") {
        Ok(Object::Reference(r)) => contents.push(Object::Reference(*r)),
        Ok(Object::Array(a)) => contents.extend(a.iter().cloned()),
        _ => {}
    }
    contents.push(Object::Reference(tail));
    let mut new_page = page_dict;
    new_page.set("Contents", Object::Array(contents));
    new_page.set("Resources", Object::Dictionary(res));
    doc.objects.insert(page, Object::Dictionary(new_page));
    Ok(())
}

/// Helvetica-Bold advance widths (1/1000 em) for ASCII 32..=126 (Adobe AFM).
const HELV_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556,
    556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278,
    556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584,
    556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556,
    333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Width in points of `text` set in Helvetica-Bold at `size`.
pub fn helv_bold_width(text: &str, size: f64) -> f64 {
    text.chars()
        .map(|c| {
            let i = c as u32;
            if (32..=126).contains(&i) { HELV_BOLD[(i - 32) as usize] } else { 556 }
        })
        .map(|w| w as f64)
        .sum::<f64>()
        * size
        / 1000.0
}

pub fn std_font(base: &str) -> Object {
    Object::Dictionary(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => Object::Name(base.as_bytes().to_vec()),
        "Encoding" => "WinAnsiEncoding",
    })
}

pub fn alpha_state(opacity: f64) -> Object {
    Object::Dictionary(dictionary! {
        "Type" => "ExtGState",
        "ca" => Object::Real(opacity as f32),
        "CA" => Object::Real(opacity as f32),
    })
}

fn r(v: f64) -> Object {
    Object::Real(v as f32)
}

// ------------------------------------------------------------- watermark

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Center,
    Top,
    Tiled,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatermarkSpec {
    pub text: String,
    pub size: f64,
    pub placement: Placement,
    /// 0..1
    pub opacity: f64,
    /// RGB 0..1
    pub color: [f64; 3],
    /// 1-based pages; empty = every page.
    #[serde(default)]
    pub pages: Vec<u32>,
}

/// Stamp `spec.text` on the pages, oriented to how each page is displayed.
pub fn watermark(doc: &mut PdfDoc, spec: &WatermarkSpec) -> Result<usize, EngineError> {
    let text = spec.text.trim();
    if text.is_empty() {
        return Err(crate::err!(ErrorCode::ValidationFailed, "watermark text is empty"));
    }
    let size = spec.size.clamp(4.0, 400.0);
    let enc = crate::textedit::std14_encode(text);
    let bytes = enc.bytes.clone();
    let tw = helv_bold_width(text, size);
    let all = doc.get_pages();
    let targets: Vec<ObjectId> = if spec.pages.is_empty() {
        all.values().copied().collect()
    } else {
        spec.pages.iter().filter_map(|n| all.get(n).copied()).collect()
    };
    for &page in &targets {
        let (w, h, cm) = display_space(doc, page);
        let res = resources_of(doc, page);
        let font = unique_name(&sub_dict(doc, &res, b"Font"), "StWmF");
        let gs = unique_name(&sub_dict(doc, &res, b"ExtGState"), "StWmG");
        let (c, s) = match spec.placement {
            Placement::Top => (1.0, 0.0),
            _ => (std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
        };
        // Text origins (display space) so the run is centred on each anchor.
        let at = |cx: f64, cy: f64| (cx - c * tw / 2.0 + s * size * 0.35, cy - s * tw / 2.0 - c * size * 0.35);
        let anchors: Vec<(f64, f64)> = match spec.placement {
            Placement::Center => vec![at(w / 2.0, h / 2.0)],
            Placement::Top => vec![(w / 2.0 - tw / 2.0, h - size * 1.6)],
            Placement::Tiled => {
                let step_x = (tw * 0.8).max(size * 4.0);
                let step_y = size * 4.0;
                let mut v = Vec::new();
                let mut row = 0;
                let mut y = step_y / 2.0;
                while y < h + step_y {
                    let mut x = if row % 2 == 0 { 0.0 } else { step_x / 2.0 };
                    while x < w + step_x {
                        v.push(at(x, y));
                        x += step_x;
                    }
                    y += step_y;
                    row += 1;
                }
                v
            }
        };
        let [cr, cg, cb] = spec.color;
        let mut ops = vec![
            Operation::new("q", vec![]),
            Operation::new("cm", cm.iter().map(|v| r(*v)).collect()),
            Operation::new("gs", vec![Object::Name(gs.clone())]),
            Operation::new("rg", vec![r(cr), r(cg), r(cb)]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(font.clone()), r(size)]),
        ];
        for (x, y) in anchors {
            ops.push(Operation::new("Tm", vec![r(c), r(s), r(-s), r(c), r(x), r(y)]));
            ops.push(Operation::new("Tj", vec![Object::String(bytes.clone(), StringFormat::Literal)]));
        }
        ops.extend([Operation::new("ET", vec![]), Operation::new("Q", vec![])]);
        append_to_page(
            doc,
            page,
            ops,
            vec![(font, crate::textedit::std14_font("Helvetica-Bold", &enc.diffs))],
            vec![(gs, alpha_state(spec.opacity.clamp(0.05, 1.0)))],
        )?;
    }
    Ok(targets.len())
}

// -------------------------------------------------------------- cropping

/// Set the visible area of a page (user-space rectangle, points).
pub fn set_crop_box(doc: &mut PdfDoc, page_no: u32, rect: [f64; 4]) -> Result<(), EngineError> {
    let page = *doc
        .get_pages()
        .get(&page_no)
        .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {page_no}"))?;
    let [x0, y0, x1, y1] = rect;
    let media = inherited(doc, page, b"MediaBox")
        .and_then(|o| o.as_array().ok().cloned())
        .and_then(|a| {
            let v: Vec<f64> = a.iter().filter_map(|o| o.as_float().ok().map(|f| f as f64)).collect();
            (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
        })
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let (a, b, c, d) = (x0.min(x1).max(media[0]), y0.min(y1).max(media[1]), x0.max(x1).min(media[2]), y0.max(y1).min(media[3]));
    if c - a < 4.0 || d - b < 4.0 {
        return Err(crate::err!(ErrorCode::ValidationFailed, "crop box too small"));
    }
    let dict = doc.get_object_mut(page)?.as_dict_mut()?;
    dict.set("CropBox", vec![r(a), r(b), r(c), r(d)]);
    Ok(())
}

// ------------------------------------------------------- protected copies

/// Serialize an AES-256 (PDF 2.0, R6) encrypted copy. `doc` is not modified.
pub fn encrypted_copy(doc: &PdfDoc, user_password: &str, owner_password: &str) -> Result<PdfDoc, EngineError> {
    if user_password.is_empty() && owner_password.is_empty() {
        return Err(crate::err!(ErrorCode::ValidationFailed, "a password is required"));
    }
    let owner = if owner_password.is_empty() { user_password } else { owner_password };
    let mut copy = doc.clone();
    copy.trailer.remove(b"Encrypt");
    if !copy.trailer.has(b"ID") {
        let id = random_bytes::<16>()?.to_vec();
        copy.trailer.set("ID", vec![Object::String(id.clone(), StringFormat::Hexadecimal), Object::String(id, StringFormat::Hexadecimal)]);
    }
    let key = random_bytes::<32>()?;
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: owner,
        user_password,
        permissions: Permissions::all(),
    })
    .map_err(|e| crate::err!(ErrorCode::Unsupported, "encryption setup failed: {e}"))?;
    copy.encrypt(&state).map_err(|e| crate::err!(ErrorCode::Unsupported, "encryption failed: {e}"))?;
    Ok(copy)
}

fn random_bytes<const N: usize>() -> Result<[u8; N], EngineError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| crate::err!(ErrorCode::IoError, "no randomness: {e}"))?;
    Ok(b)
}

/// Smallest faithful serialization: unused objects dropped, every stream
/// Flate-compressed, objects packed into object streams.
pub fn compressed_bytes(doc: &PdfDoc) -> Result<Vec<u8>, EngineError> {
    let mut copy = doc.clone();
    copy.trailer.remove(b"Encrypt");
    copy.prune_objects();
    copy.compress();
    let mut out = Vec::new();
    copy.save_with_options(
        &mut out,
        SaveOptions::builder().use_object_streams(true).use_xref_streams(true).compression_level(9).build(),
    )?;
    Ok(out)
}

// ------------------------------------------------------------ attachments

#[derive(Debug, Clone, Serialize)]
pub struct Attachment {
    pub index: usize,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub description: String,
}

/// File specifications from the catalog's EmbeddedFiles name tree and from
/// FileAttachment annotations, in document order.
fn filespecs(doc: &PdfDoc) -> Vec<Dictionary> {
    fn walk(doc: &PdfDoc, node: &Dictionary, out: &mut Vec<Dictionary>, depth: u32) {
        if depth > 32 {
            return;
        }
        if let Ok(names) = node.get(b"Names").and_then(Object::as_array) {
            for pair in names.chunks(2) {
                if let Some(spec) = pair.get(1).and_then(|o| deref(doc, o)).and_then(|o| o.as_dict().ok()) {
                    out.push(spec.clone());
                }
            }
        }
        if let Ok(kids) = node.get(b"Kids").and_then(Object::as_array) {
            for k in kids {
                if let Some(d) = deref(doc, k).and_then(|o| o.as_dict().ok()) {
                    walk(doc, d, out, depth + 1);
                }
            }
        }
    }
    let mut out = Vec::new();
    let tree = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .and_then(|n| n.get(b"EmbeddedFiles").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok());
    if let Some(t) = tree {
        walk(doc, t, &mut out, 0);
    }
    for (_, page) in doc.get_pages() {
        let annots = doc.get_object(page).ok().and_then(|p| p.as_dict().ok()).and_then(|d| d.get(b"Annots").ok()).and_then(|o| deref(doc, o));
        for a in annots.and_then(|o| o.as_array().ok()).into_iter().flatten() {
            if let Some(ad) = deref(doc, a).and_then(|o| o.as_dict().ok()) {
                if ad.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"FileAttachment") {
                    if let Some(fs) = ad.get(b"FS").ok().and_then(|o| deref(doc, o)).and_then(|o| o.as_dict().ok()) {
                        out.push(fs.clone());
                    }
                }
            }
        }
    }
    out
}

fn deref<'a>(doc: &'a PdfDoc, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok(),
        o => Some(o),
    }
}

fn embedded_stream<'a>(doc: &'a PdfDoc, spec: &'a Dictionary) -> Option<&'a Stream> {
    let ef = spec.get(b"EF").ok().and_then(|o| deref(doc, o))?.as_dict().ok()?;
    let s = ef.get(b"UF").or_else(|_| ef.get(b"F")).ok().and_then(|o| deref(doc, o))?;
    s.as_stream().ok()
}

pub fn list_attachments(doc: &PdfDoc) -> Vec<Attachment> {
    filespecs(doc)
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let text = |k: &[u8]| spec.get(k).ok().and_then(|o| lopdf::decode_text_string(o).ok()).unwrap_or_default();
            let name = Some(text(b"UF")).filter(|s| !s.is_empty()).unwrap_or_else(|| text(b"F"));
            let stream = embedded_stream(doc, spec);
            let size = stream
                .and_then(|s| s.dict.get(b"Params").ok())
                .and_then(|o| deref(doc, o))
                .and_then(|o| o.as_dict().ok())
                .and_then(|p| p.get(b"Size").ok())
                .and_then(|o| o.as_i64().ok())
                .map(|v| v as u64)
                .or_else(|| stream.and_then(|s| s.decompressed_content().ok()).map(|b| b.len() as u64))
                .unwrap_or(0);
            let mime = stream
                .and_then(|s| s.dict.get(b"Subtype").ok())
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).replace("#2F", "/"))
                .unwrap_or_default();
            Attachment { index, name: if name.is_empty() { format!("attachment-{}", index + 1) } else { name }, size, mime, description: text(b"Desc") }
        })
        .collect()
}

pub fn attachment_bytes(doc: &PdfDoc, index: usize) -> Result<Vec<u8>, EngineError> {
    let specs = filespecs(doc);
    let spec = specs.get(index).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no attachment {index}"))?;
    let s = embedded_stream(doc, spec).ok_or_else(|| crate::err!(ErrorCode::Malformed, "attachment has no embedded data"))?;
    s.decompressed_content().or_else(|_| Ok(s.content.clone()))
}

/// Partial names (/T) of all AcroForm fields, recursively.
pub fn list_field_names(doc: &PdfDoc) -> Vec<String> {
    fn walk(doc: &PdfDoc, arr: &[Object], out: &mut Vec<String>, depth: u32) {
        if depth > 16 {
            return;
        }
        for f in arr {
            if let Some(d) = deref(doc, f).and_then(|o| o.as_dict().ok()) {
                if let Ok(t) = d.get(b"T").and_then(lopdf::decode_text_string) {
                    out.push(t);
                }
                if let Some(kids) = d.get(b"Kids").ok().and_then(|o| deref(doc, o)).and_then(|o| o.as_array().ok()) {
                    walk(doc, kids, out, depth + 1);
                }
            }
        }
    }
    let mut out = vec![];
    let fields = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .and_then(|f| f.get(b"Fields").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok());
    if let Some(f) = fields {
        walk(doc, f, &mut out, 0);
    }
    out
}
