//! True in-place text editing.
//!
//! The page's content stream is decoded and interpreted (graphics + text
//! state), every text-show operation is decoded to Unicode via its font, and
//! the operation(s) that drew the run the user clicked are located by text and
//! position. Then, in order of preference:
//!
//! 1. **InPlace** — the run is re-encoded in its ORIGINAL font (same size,
//!    position, colour, matrix). Used when the font can encode every new
//!    character and actually carries the glyphs (subset fonts are checked).
//! 2. **Replaced** — the original glyphs are removed from the content stream
//!    and the new text is drawn at the same text matrix in the closest
//!    standard-14 family (sans / serif / mono, bold / italic kept).
//! 3. **Overlay** — legacy fallback when the run cannot be found in the page
//!    content (e.g. text inside a form XObject): white patch + new text.
//!
//! Undo safety: existing stream objects are never modified. The rewritten
//! content goes into a NEW stream and only the page dictionary's `/Contents`
//! (and `/Resources` for 2/3) change — exactly what `oplog` snapshots.

use std::collections::{BTreeMap, HashSet};

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document as PdfDoc, Object, ObjectId, Stream, StringFormat};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, ErrorCode};

/// What the frontend knows about the run (all PDF user space, points).
#[derive(Debug, Clone, Deserialize)]
pub struct EditRequest {
    pub page: u32,
    /// Baseline origin of the run (pdf.js `item.transform[4..6]`).
    pub x: f64,
    pub y: f64,
    /// Run advance width and font size (pdf.js `item.width`, |transform[3]|).
    pub width: f64,
    pub font_size: f64,
    /// The run text as the renderer extracted it.
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditMethod {
    InPlace,
    Replaced,
    Overlay,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditOutcome {
    pub method: EditMethod,
    /// Human-readable detail for the status bar.
    pub note: String,
}

pub(crate) type M = [f64; 6];
pub(crate) const ID: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub(crate) fn mul(a: &M, b: &M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

fn num(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

fn nums<const N: usize>(ops: &[Object]) -> Option<[f64; N]> {
    if ops.len() < N {
        return None;
    }
    let mut out = [0.0; N];
    for (i, o) in ops[ops.len() - N..].iter().enumerate() {
        out[i] = num(o)?;
    }
    Some(out)
}

// ---------------------------------------------------------------- fonts

pub(crate) struct FontInfo {
    /// Two-byte codes (Type0 / Identity) vs one-byte simple fonts.
    pub(crate) two_byte: bool,
    /// Embedded program present (FontFile/2/3) => glyph set may be a subset.
    embedded: bool,
    pub(crate) base_font: String,
    first_char: i64,
    widths: Vec<f64>,
    cid_widths: BTreeMap<u32, f64>,
    default_width: f64,
    font_dict: Dictionary,
}

impl FontInfo {
    fn width(&self, code: u32) -> f64 {
        if self.two_byte {
            return self.cid_widths.get(&code).copied().unwrap_or(self.default_width) / 1000.0;
        }
        let i = code as i64 - self.first_char;
        if i >= 0 && (i as usize) < self.widths.len() && self.widths[i as usize] > 0.0 {
            return self.widths[i as usize] / 1000.0;
        }
        // No /Widths entry: a standard-14 font's built-in metrics.
        if self.widths.is_empty() {
            if let Some(w) = crate::std14::width(&self.base_font, code) {
                return w / 1000.0;
            }
        }
        self.default_width / 1000.0
    }

    fn codes(&self, bytes: &[u8]) -> Vec<u32> {
        if self.two_byte {
            bytes.chunks(2).map(|c| if c.len() == 2 { (c[0] as u32) << 8 | c[1] as u32 } else { c[0] as u32 }).collect()
        } else {
            bytes.iter().map(|&b| b as u32).collect()
        }
    }

    /// Unicode for the raw string bytes (None when the font has no usable map).
    pub(crate) fn decode(&self, doc: &PdfDoc, bytes: &[u8]) -> Option<String> {
        if self.two_byte {
            // Identity-H etc.: only ToUnicode gives text. Resolve it on a copy
            // of the dict without /Encoding so lopdf picks the ToUnicode map.
            let mut d = self.font_dict.clone();
            d.remove(b"Encoding");
            let enc = d.get_font_encoding(doc).ok()?;
            if !matches!(enc, lopdf::Encoding::UnicodeMapEncoding(_)) {
                return None;
            }
            return enc.bytes_to_string(bytes).ok();
        }
        match self.font_dict.get_font_encoding(doc) {
            Ok(enc) => enc.bytes_to_string(bytes).ok().or_else(|| Some(latin1(bytes))),
            Err(_) => Some(latin1(bytes)),
        }
    }

    /// Bytes for `text` in this font, or None if any character can't be encoded.
    fn encode(&self, doc: &PdfDoc, text: &str) -> Option<Vec<u8>> {
        let enc = if self.two_byte {
            let mut d = self.font_dict.clone();
            d.remove(b"Encoding");
            let e = d.get_font_encoding(doc).ok()?;
            if !matches!(e, lopdf::Encoding::UnicodeMapEncoding(_)) {
                return None;
            }
            let bytes = e.string_to_bytes(text);
            return (e.bytes_to_string(&bytes).ok()? == text).then_some(bytes);
        } else {
            self.font_dict.get_font_encoding(doc).ok()?
        };
        let bytes = enc.string_to_bytes(text);
        let back = enc.bytes_to_string(&bytes).ok()?;
        (back == text && bytes.len() == text.chars().count()).then_some(bytes)
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn resolve<'a>(doc: &'a PdfDoc, o: &'a Object) -> &'a Object {
    match o {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(o),
        _ => o,
    }
}

pub(crate) fn font_info(doc: &PdfDoc, font: &Dictionary) -> FontInfo {
    let subtype = font.get(b"Subtype").and_then(Object::as_name).unwrap_or(b"");
    let two_byte = subtype == b"Type0";
    let base_font = font
        .get(b"BaseFont")
        .and_then(Object::as_name)
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_default();

    let descriptor_of = |d: &Dictionary| -> Option<Dictionary> {
        resolve(doc, d.get(b"FontDescriptor").ok()?).as_dict().ok().cloned()
    };
    let mut embedded = false;
    let mut default_width = 500.0;
    let mut first_char = 0;
    let mut widths = Vec::new();
    let mut cid_widths = BTreeMap::new();

    if two_byte {
        default_width = 1000.0;
        let desc = font
            .get(b"DescendantFonts")
            .ok()
            .map(|o| resolve(doc, o))
            .and_then(|o| o.as_array().ok())
            .and_then(|a| a.first())
            .map(|o| resolve(doc, o))
            .and_then(|o| o.as_dict().ok());
        if let Some(cid) = desc {
            if let Some(dw) = cid.get(b"DW").ok().and_then(num) {
                default_width = dw;
            }
            if let Some(w) = cid.get(b"W").ok().map(|o| resolve(doc, o)).and_then(|o| o.as_array().ok()) {
                let mut i = 0;
                while i + 1 < w.len() {
                    let c0 = num(resolve(doc, &w[i])).unwrap_or(0.0) as u32;
                    match resolve(doc, &w[i + 1]) {
                        Object::Array(list) => {
                            for (k, v) in list.iter().enumerate() {
                                if let Some(v) = num(resolve(doc, v)) {
                                    cid_widths.insert(c0 + k as u32, v);
                                }
                            }
                            i += 2;
                        }
                        other => {
                            let c1 = num(other).unwrap_or(0.0) as u32;
                            let v = w.get(i + 2).map(|o| resolve(doc, o)).and_then(num).unwrap_or(default_width);
                            for c in c0..=c1.min(c0 + 65535) {
                                cid_widths.insert(c, v);
                            }
                            i += 3;
                        }
                    }
                }
            }
            if let Some(fd) = descriptor_of(cid) {
                embedded = fd.has(b"FontFile") || fd.has(b"FontFile2") || fd.has(b"FontFile3");
            }
        }
    } else {
        first_char = font.get(b"FirstChar").ok().and_then(num).unwrap_or(0.0) as i64;
        if let Some(w) = font.get(b"Widths").ok().map(|o| resolve(doc, o)).and_then(|o| o.as_array().ok()) {
            widths = w.iter().map(|o| num(resolve(doc, o)).unwrap_or(0.0)).collect();
        }
        if let Some(fd) = descriptor_of(font) {
            embedded = fd.has(b"FontFile") || fd.has(b"FontFile2") || fd.has(b"FontFile3");
            if let Some(mw) = fd.get(b"MissingWidth").ok().and_then(num) {
                if mw > 0.0 {
                    default_width = mw;
                }
            }
        }
        if subtype == b"Type3" {
            embedded = true;
        }
        if base_font.contains("Courier") {
            default_width = 600.0;
        }
    }
    FontInfo {
        two_byte,
        embedded,
        base_font,
        first_char,
        widths,
        cid_widths,
        default_width,
        font_dict: font.clone(),
    }
}

// ---------------------------------------------------- content interpretation

#[derive(Clone)]
struct TextState {
    tm: M,
    tlm: M,
    font: Vec<u8>,
    size: f64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    rise: f64,
}

impl Default for TextState {
    fn default() -> Self {
        Self { tm: ID, tlm: ID, font: Vec::new(), size: 0.0, tc: 0.0, tw: 0.0, th: 1.0, tl: 0.0, rise: 0.0 }
    }
}

/// One glyph (code) or TJ kerning number inside a show op, in order.
#[derive(Clone, Debug)]
pub(crate) enum Item {
    /// Raw code bytes, start x and advance (text space, before trm).
    Glyph { bytes: Vec<u8>, x: f64, adv: f64 },
    Kern(f64),
}

/// One text-show operation, with everything needed to match and rewrite it.
pub(crate) struct Show {
    pub(crate) op: usize,
    /// Text matrix x CTM at the start of the op.
    pub(crate) trm: M,
    /// Text matrix / line matrix before the op and text matrix after it.
    pub(crate) tm: M,
    pub(crate) tlm: M,
    pub(crate) tm_after: M,
    pub(crate) ctm: M,
    pub(crate) font: Vec<u8>,
    pub(crate) size: f64,
    pub(crate) th: f64,
    pub(crate) rise: f64,
    pub(crate) two_byte: bool,
    /// Decoded text ("" when the font has no usable map).
    pub(crate) text: String,
    pub(crate) decoded: bool,
    /// Per-char start x offsets (text space, before trm) for substring placement.
    pub(crate) char_x: Vec<f64>,
    pub(crate) items: Vec<Item>,
    /// Fill-colour operators in effect (colour space + colour), to restore.
    pub(crate) fill: Vec<Operation>,
    pub(crate) block: usize,
}

/// An XObject (image or form) drawn with `Do`.
pub(crate) struct Draw {
    pub(crate) op: usize,
    pub(crate) name: Vec<u8>,
    pub(crate) ctm: M,
}

pub(crate) struct Interpreted {
    pub(crate) shows: Vec<Show>,
    pub(crate) draws: Vec<Draw>,
}

impl Show {
    pub(crate) fn origin(&self) -> (f64, f64) {
        let p = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, self.rise], &self.trm);
        (p[4], p[5])
    }
    fn point_at(&self, tx: f64) -> (f64, f64) {
        let p = mul(&[1.0, 0.0, 0.0, 1.0, tx, self.rise], &self.trm);
        (p[4], p[5])
    }
}

fn string_bytes(o: &Object) -> Option<&[u8]> {
    match o {
        Object::String(b, _) => Some(b),
        _ => None,
    }
}

/// Walk the content, returning every text show op in drawing order.
fn interpret(doc: &PdfDoc, ops: &[Operation], fonts: &BTreeMap<Vec<u8>, FontInfo>) -> Vec<Show> {
    interpret_page(doc, ops, fonts).shows
}

/// Full walk: text shows (with glyph geometry) and XObject draws.
pub(crate) fn interpret_page(doc: &PdfDoc, ops: &[Operation], fonts: &BTreeMap<Vec<u8>, FontInfo>) -> Interpreted {
    let mut out = Vec::new();
    let mut draws = Vec::new();
    let mut ctm = ID;
    let mut fill: Vec<Operation> = Vec::new();
    let mut stack: Vec<(M, TextState, Vec<Operation>)> = Vec::new();
    let mut ts = TextState::default();
    let mut block = 0usize;

    for (i, op) in ops.iter().enumerate() {
        let a = &op.operands;
        match op.operator.as_str() {
            "q" => stack.push((ctm, ts.clone(), fill.clone())),
            "cs" => fill = vec![op.clone()],
            "rg" | "g" | "k" => fill = vec![op.clone()],
            "sc" | "scn" => {
                fill.retain(|o| o.operator == "cs");
                fill.push(op.clone());
            }
            "Do" => {
                if let Some(Object::Name(n)) = a.first() {
                    draws.push(Draw { op: i, name: n.clone(), ctm });
                }
            }
            "Q" => {
                if let Some((c, t, f)) = stack.pop() {
                    ctm = c;
                    fill = f;
                    // Text state params persist with the graphics state.
                    let (tm, tlm) = (ts.tm, ts.tlm);
                    ts = t;
                    ts.tm = tm;
                    ts.tlm = tlm;
                }
            }
            "cm" => {
                if let Some(m) = nums::<6>(a) {
                    ctm = mul(&m, &ctm);
                }
            }
            "BT" => {
                ts.tm = ID;
                ts.tlm = ID;
                block += 1;
            }
            "Tf" => {
                if let (Some(Object::Name(n)), Some(s)) = (a.first(), a.get(1).and_then(num)) {
                    ts.font = n.clone();
                    ts.size = s;
                }
            }
            "Tc" => ts.tc = a.first().and_then(num).unwrap_or(ts.tc),
            "Tw" => ts.tw = a.first().and_then(num).unwrap_or(ts.tw),
            "Tz" => ts.th = a.first().and_then(num).unwrap_or(100.0) / 100.0,
            "TL" => ts.tl = a.first().and_then(num).unwrap_or(ts.tl),
            "Ts" => ts.rise = a.first().and_then(num).unwrap_or(ts.rise),
            "Td" | "TD" => {
                if let Some([tx, ty]) = nums::<2>(a) {
                    if op.operator == "TD" {
                        ts.tl = -ty;
                    }
                    ts.tlm = mul(&[1.0, 0.0, 0.0, 1.0, tx, ty], &ts.tlm);
                    ts.tm = ts.tlm;
                }
            }
            "Tm" => {
                if let Some(m) = nums::<6>(a) {
                    ts.tlm = m;
                    ts.tm = m;
                }
            }
            "T*" => {
                ts.tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -ts.tl], &ts.tlm);
                ts.tm = ts.tlm;
            }
            "Tj" | "TJ" | "'" | "\"" => {
                if op.operator == "'" || op.operator == "\"" {
                    if op.operator == "\"" {
                        if let Some([aw, ac]) = a.get(..2).and_then(nums::<2>) {
                            ts.tw = aw;
                            ts.tc = ac;
                        }
                    }
                    ts.tlm = mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -ts.tl], &ts.tlm);
                    ts.tm = ts.tlm;
                }
                let Some(font) = fonts.get(&ts.font) else { continue };
                let (tm0, tlm0) = (ts.tm, ts.tlm);
                let mut items = Vec::new();
                // Flatten operands into (bytes | kerning) pieces.
                let pieces: Vec<Result<&[u8], f64>> = match op.operator.as_str() {
                    "TJ" => a
                        .first()
                        .and_then(|o| o.as_array().ok())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|o| string_bytes(o).map(Ok).or_else(|| num(o).map(Err)))
                                .collect()
                        })
                        .unwrap_or_default(),
                    _ => a.last().and_then(string_bytes).map(|b| vec![Ok(b)]).unwrap_or_default(),
                };
                let trm = mul(&ts.tm, &ctm);
                let mut text = String::new();
                let mut char_x = Vec::new();
                let mut tx = 0.0;
                let mut ok = true;
                for p in pieces {
                    match p {
                        Ok(bytes) => {
                            let codes = font.codes(bytes);
                            // Glyph geometry (always — redaction needs it even
                            // when the font can't be decoded to Unicode).
                            let mut gx = tx;
                            for (ci, &c) in codes.iter().enumerate() {
                                let space = !font.two_byte && c == 32;
                                let adv = (font.width(c) * ts.size + ts.tc + if space { ts.tw } else { 0.0 }) * ts.th;
                                let w = if font.two_byte { 2 } else { 1 };
                                items.push(Item::Glyph { bytes: bytes[ci * w..((ci + 1) * w).min(bytes.len())].to_vec(), x: gx, adv });
                                gx += adv;
                            }
                            let Some(s) = font.decode(doc, bytes).filter(|_| ok) else {
                                ok = false;
                                tx = gx;
                                continue;
                            };
                            // Map decoded chars onto codes 1:1 when counts agree;
                            // otherwise spread positions evenly (ligatures etc.).
                            let chars: Vec<char> = s.chars().collect();
                            let mut code_x = Vec::with_capacity(codes.len());
                            for &c in &codes {
                                code_x.push(tx);
                                let space = !font.two_byte && c == 32;
                                tx += (font.width(c) * ts.size + ts.tc + if space { ts.tw } else { 0.0 }) * ts.th;
                            }
                            for k in 0..chars.len() {
                                let idx = if chars.len() == codes.len() { k } else { k * codes.len() / chars.len().max(1) };
                                char_x.push(code_x.get(idx).copied().unwrap_or(tx));
                            }
                            text.push_str(&s);
                        }
                        Err(k) => {
                            items.push(Item::Kern(k));
                            tx -= k / 1000.0 * ts.size * ts.th;
                        }
                    }
                }
                ts.tm = mul(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], &ts.tm);
                out.push(Show {
                    op: i,
                    trm,
                    tm: tm0,
                    tlm: tlm0,
                    tm_after: ts.tm,
                    ctm,
                    font: ts.font.clone(),
                    size: ts.size,
                    th: ts.th,
                    rise: ts.rise,
                    two_byte: font.two_byte,
                    text: if ok { text } else { String::new() },
                    decoded: ok,
                    char_x,
                    items,
                    fill: fill.clone(),
                    block,
                });
            }
            _ => {}
        }
    }
    Interpreted { shows: out, draws }
}

fn norm(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '\u{FB00}' => "ff".chars().collect::<Vec<_>>(),
            '\u{FB01}' => "fi".chars().collect(),
            '\u{FB02}' => "fl".chars().collect(),
            '\u{FB03}' => "ffi".chars().collect(),
            '\u{FB04}' => "ffl".chars().collect(),
            c if c.is_whitespace() || c == '\u{00A0}' => vec![],
            c => vec![c],
        })
        .collect()
}

/// Where the target run lives in the decoded content.
enum Hit {
    /// Ops `first..=last` (indices into `shows`) together draw exactly the run.
    Whole { first: usize, last: usize },
    /// The run is a substring `[start, end)` (char indices) of one op.
    Part { show: usize, start: usize, end: usize },
}

fn find_run(shows: &[Show], req: &EditRequest) -> Option<Hit> {
    let target = norm(&req.old_text);
    if target.is_empty() {
        return None;
    }
    let tol = (req.font_size * 0.6).max(2.0);
    let dist = |p: (f64, f64)| ((p.0 - req.x).powi(2) + (p.1 - req.y).powi(2)).sqrt();

    // 1) Sequence of whole ops starting near the run origin.
    let mut best: Option<(f64, Hit)> = None;
    for i in 0..shows.len() {
        if !shows[i].decoded {
            continue;
        }
        let d = dist(shows[i].origin());
        if d > tol {
            continue;
        }
        let mut acc = String::new();
        for j in i..shows.len() {
            if shows[j].block != shows[i].block || !shows[j].decoded {
                break;
            }
            acc.push_str(&norm(&shows[j].text));
            if acc == target {
                if best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                    best = Some((d, Hit::Whole { first: i, last: j }));
                }
                break;
            }
            if !target.starts_with(&acc) {
                break;
            }
        }
    }
    if let Some((_, h)) = best {
        return Some(h);
    }

    // 2) Substring of a single op on the same baseline (renderer split it).
    let old = req.old_text.trim();
    let mut best: Option<(f64, Hit)> = None;
    for (si, s) in shows.iter().enumerate() {
        if !s.decoded {
            continue;
        }
        let (_, oy) = s.origin();
        if (oy - req.y).abs() > tol {
            continue;
        }
        let chars: Vec<char> = s.text.chars().collect();
        let needle: Vec<char> = old.chars().collect();
        if needle.is_empty() || needle.len() > chars.len() {
            continue;
        }
        for st in 0..=chars.len() - needle.len() {
            if chars[st..st + needle.len()] != needle[..] {
                continue;
            }
            let p = s.point_at(s.char_x.get(st).copied().unwrap_or(0.0));
            let d = dist(p);
            if d <= tol * 2.0 && best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                best = Some((d, Hit::Part { show: si, start: st, end: st + needle.len() }));
            }
        }
    }
    best.map(|(_, h)| h)
}

/// Glyphs we can be sure a (possibly subset) simple font carries: every code it
/// already draws on this page.
fn used_codes(shows: &[Show], ops: &[Operation], fonts: &BTreeMap<Vec<u8>, FontInfo>, font: &[u8]) -> HashSet<u32> {
    let mut set = HashSet::new();
    let Some(fi) = fonts.get(font) else { return set };
    for s in shows.iter().filter(|s| s.font == font) {
        for o in &ops[s.op].operands {
            let mut add = |b: &[u8]| set.extend(fi.codes(b));
            match o {
                Object::String(b, _) => add(b),
                Object::Array(arr) => arr.iter().filter_map(string_bytes).for_each(|b| add(b)),
                _ => {}
            }
        }
    }
    set
}

/// Encode `text` in the run's own font if every glyph is known to exist.
fn encode_in_font(
    doc: &PdfDoc,
    fi: &FontInfo,
    text: &str,
    available: &HashSet<u32>,
) -> Option<Vec<u8>> {
    let bytes = fi.encode(doc, text)?;
    if fi.embedded && !fi.two_byte {
        // Subset-safe: each code must already be drawn with this font, or be a
        // space (rendered as an advance even if the glyph is empty).
        for c in fi.codes(&bytes) {
            if c != 32 && !available.contains(&c) {
                return None;
            }
        }
    }
    Some(bytes)
}

pub(crate) fn set_text(op: &mut Operation, bytes: Vec<u8>, hex: bool) {
    let s = Object::String(bytes, if hex { StringFormat::Hexadecimal } else { StringFormat::Literal });
    match op.operator.as_str() {
        "'" | "\"" => {
            if let Some(last) = op.operands.last_mut() {
                *last = s;
            }
        }
        _ => {
            op.operator = "Tj".into();
            op.operands = vec![s];
        }
    }
}

/// Standard-14 replacement family that best resembles the original font.
pub(crate) fn standard_font(base: &str) -> &'static str {
    let b = base.to_lowercase();
    let bold = b.contains("bold") || b.contains("black") || b.contains("heavy") || b.contains("semibold");
    let italic = b.contains("italic") || b.contains("oblique");
    let mono = ["courier", "mono", "consol", "menlo", "cmtt"].iter().any(|k| b.contains(k));
    let serif = ["times", "serif", "roman", "georgia", "garamond", "cmr", "minion", "cambria", "book", "palatino"]
        .iter()
        .any(|k| b.contains(k))
        && !b.contains("sans");
    match (mono, serif, bold, italic) {
        (true, _, true, true) => "Courier-BoldOblique",
        (true, _, true, false) => "Courier-Bold",
        (true, _, false, true) => "Courier-Oblique",
        (true, _, false, false) => "Courier",
        (false, true, true, true) => "Times-BoldItalic",
        (false, true, true, false) => "Times-Bold",
        (false, true, false, true) => "Times-Italic",
        (false, true, false, false) => "Times-Roman",
        (false, false, true, true) => "Helvetica-BoldOblique",
        (false, false, true, false) => "Helvetica-Bold",
        (false, false, false, true) => "Helvetica-Oblique",
        (false, false, false, false) => "Helvetica",
    }
}

/// WinAnsi bytes for `text`; unencodable chars become '?' (returned separately).
pub fn winansi(text: &str) -> (Vec<u8>, Vec<char>) {
    let enc = lopdf::Encoding::SimpleEncoding(b"WinAnsiEncoding");
    let mut out = Vec::new();
    let mut bad = Vec::new();
    for c in text.chars() {
        let b = enc.string_to_bytes(&c.to_string());
        if b.len() == 1 && enc.bytes_to_string(&b).ok().as_deref() == Some(c.to_string().as_str()) {
            out.extend(b);
        } else {
            out.push(b'?');
            if !bad.contains(&c) {
                bad.push(c);
            }
        }
    }
    (out, bad)
}

/// Glyph names (Adobe Glyph List) for characters outside WinAnsi that the
/// standard-14 Latin fonts carry: Central European Latin Extended-A (Polish,
/// Czech, Slovak, Hungarian, Romanian, Turkish, Baltic…) and a few symbols.
const EXTRA_GLYPHS: &[(char, &str)] = &[
    ('\u{0100}', "Amacron"), ('\u{0101}', "amacron"), ('\u{0102}', "Abreve"), ('\u{0103}', "abreve"),
    ('\u{0104}', "Aogonek"), ('\u{0105}', "aogonek"), ('\u{0106}', "Cacute"), ('\u{0107}', "cacute"),
    ('\u{010C}', "Ccaron"), ('\u{010D}', "ccaron"), ('\u{010E}', "Dcaron"), ('\u{010F}', "dcaron"),
    ('\u{0110}', "Dcroat"), ('\u{0111}', "dcroat"), ('\u{0112}', "Emacron"), ('\u{0113}', "emacron"),
    ('\u{0116}', "Edotaccent"), ('\u{0117}', "edotaccent"), ('\u{0118}', "Eogonek"), ('\u{0119}', "eogonek"),
    ('\u{011A}', "Ecaron"), ('\u{011B}', "ecaron"), ('\u{011E}', "Gbreve"), ('\u{011F}', "gbreve"),
    ('\u{0122}', "Gcommaaccent"), ('\u{0123}', "gcommaaccent"), ('\u{012A}', "Imacron"), ('\u{012B}', "imacron"),
    ('\u{012E}', "Iogonek"), ('\u{012F}', "iogonek"), ('\u{0130}', "Idotaccent"), ('\u{0131}', "dotlessi"),
    ('\u{0136}', "Kcommaaccent"), ('\u{0137}', "kcommaaccent"), ('\u{0139}', "Lacute"), ('\u{013A}', "lacute"),
    ('\u{013B}', "Lcommaaccent"), ('\u{013C}', "lcommaaccent"), ('\u{013D}', "Lcaron"), ('\u{013E}', "lcaron"),
    ('\u{0141}', "Lslash"), ('\u{0142}', "lslash"), ('\u{0143}', "Nacute"), ('\u{0144}', "nacute"),
    ('\u{0145}', "Ncommaaccent"), ('\u{0146}', "ncommaaccent"), ('\u{0147}', "Ncaron"), ('\u{0148}', "ncaron"),
    ('\u{014C}', "Omacron"), ('\u{014D}', "omacron"), ('\u{0150}', "Ohungarumlaut"), ('\u{0151}', "ohungarumlaut"),
    ('\u{0154}', "Racute"), ('\u{0155}', "racute"), ('\u{0156}', "Rcommaaccent"), ('\u{0157}', "rcommaaccent"),
    ('\u{0158}', "Rcaron"), ('\u{0159}', "rcaron"), ('\u{015A}', "Sacute"), ('\u{015B}', "sacute"),
    ('\u{015E}', "Scedilla"), ('\u{015F}', "scedilla"), ('\u{0162}', "Tcommaaccent"), ('\u{0163}', "tcommaaccent"),
    ('\u{0164}', "Tcaron"), ('\u{0165}', "tcaron"), ('\u{016A}', "Umacron"), ('\u{016B}', "umacron"),
    ('\u{016E}', "Uring"), ('\u{016F}', "uring"), ('\u{0170}', "Uhungarumlaut"), ('\u{0171}', "uhungarumlaut"),
    ('\u{0172}', "Uogonek"), ('\u{0173}', "uogonek"), ('\u{0179}', "Zacute"), ('\u{017A}', "zacute"),
    ('\u{017B}', "Zdotaccent"), ('\u{017C}', "zdotaccent"), ('\u{0218}', "Scommaaccent"), ('\u{0219}', "scommaaccent"),
    ('\u{2212}', "minus"), ('\u{2260}', "notequal"), ('\u{2264}', "lessequal"), ('\u{2265}', "greaterequal"),
];

/// Text encoded for a standard-14 font: WinAnsi codes plus, for characters
/// outside WinAnsi, spare codes 1..31 remapped by a /Differences array.
pub struct Std14Text {
    pub bytes: Vec<u8>,
    pub diffs: Vec<(u8, &'static str)>,
    /// Characters no standard-14 Latin font has (shown as '?').
    pub unsupported: Vec<char>,
}

pub fn std14_encode(text: &str) -> Std14Text {
    let (win, _) = winansi(text);
    let mut bytes = Vec::with_capacity(win.len());
    let mut diffs: Vec<(u8, &'static str)> = Vec::new();
    let mut unsupported = Vec::new();
    for (c, b) in text.chars().zip(win) {
        if b != b'?' || c == '?' {
            bytes.push(b);
            continue;
        }
        match EXTRA_GLYPHS.iter().find(|(u, _)| *u == c) {
            Some((_, name)) => {
                let code = match diffs.iter().find(|(_, n)| n == name) {
                    Some((code, _)) => *code,
                    None if diffs.len() < 31 => {
                        let code = diffs.len() as u8 + 1;
                        diffs.push((code, name));
                        code
                    }
                    None => b'?',
                };
                bytes.push(code);
            }
            None => {
                bytes.push(b'?');
                if !unsupported.contains(&c) {
                    unsupported.push(c);
                }
            }
        }
    }
    Std14Text { bytes, diffs, unsupported }
}

/// A standard-14 font dictionary whose encoding covers `diffs`.
pub fn std14_font(base: &str, diffs: &[(u8, &'static str)]) -> Object {
    let encoding = if diffs.is_empty() {
        Object::Name(b"WinAnsiEncoding".to_vec())
    } else {
        let mut d = vec![];
        for (code, name) in diffs {
            d.push(Object::Integer(*code as i64));
            d.push(Object::Name(name.as_bytes().to_vec()));
        }
        Object::Dictionary(dictionary! {
            "Type" => "Encoding",
            "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => d,
        })
    };
    Object::Dictionary(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => Object::Name(base.as_bytes().to_vec()),
        "Encoding" => encoding,
    })
}

pub(crate) fn unique_font_name(fonts: &BTreeMap<Vec<u8>, FontInfo>) -> Vec<u8> {
    (1..)
        .map(|i| format!("StEd{i}").into_bytes())
        .find(|n| !fonts.contains_key(n))
        .unwrap()
}

/// Page `/Resources` as an owned dictionary (inline or referenced).
fn page_resources(doc: &PdfDoc, page: &Dictionary) -> Dictionary {
    match page.get(b"Resources") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    }
}

pub(crate) fn add_font_resource(doc: &PdfDoc, page: &Dictionary, name: &[u8], font: Object) -> Dictionary {
    let mut res = page_resources(doc, page);
    let mut fonts = match res.get(b"Font") {
        Ok(Object::Dictionary(d)) => d.clone(),
        Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
        _ => Dictionary::new(),
    };
    fonts.set(name.to_vec(), font);
    res.set("Font", Object::Dictionary(fonts));
    res
}

/// Edit the run described by `req` on its page. See the module docs.
pub fn edit_text(doc: &mut PdfDoc, req: &EditRequest) -> Result<EditOutcome, EngineError> {
    // An empty replacement deletes the run.
    let new_text: String = req.new_text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let page_id: ObjectId = *doc
        .get_pages()
        .get(&req.page)
        .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {}", req.page))?;

    match edit_in_content(doc, page_id, req, &new_text) {
        Ok(Some(outcome)) => Ok(outcome),
        // Run not found in the page content: legacy white-patch overlay.
        Ok(None) | Err(_) => overlay(doc, page_id, req, &new_text),
    }
}

fn edit_in_content(
    doc: &mut PdfDoc,
    page_id: ObjectId,
    req: &EditRequest,
    new_text: &str,
) -> Result<Option<EditOutcome>, EngineError> {
    let content = doc.get_and_decode_page_content(page_id)?;
    let mut ops = content.operations;
    let fonts: BTreeMap<Vec<u8>, FontInfo> = doc
        .get_page_fonts(page_id)?
        .into_iter()
        .map(|(k, d)| (k, font_info(doc, d)))
        .collect();
    let shows = interpret(doc, &ops, &fonts);
    let Some(hit) = find_run(&shows, req) else { return Ok(None) };

    let (first, last, prefix, suffix, start_tx) = match hit {
        Hit::Whole { first, last } => (first, last, String::new(), String::new(), 0.0),
        Hit::Part { show, start, end } => {
            let chars: Vec<char> = shows[show].text.chars().collect();
            let tx = shows[show].char_x.get(start).copied().unwrap_or(0.0);
            (show, show, chars[..start].iter().collect(), chars[end..].iter().collect(), tx)
        }
    };
    let lead = &shows[first];
    let fi = &fonts[&lead.font];
    let available = used_codes(&shows, &ops, &fonts, &lead.font);
    let full = format!("{prefix}{new_text}{suffix}");

    // Clear every matched op but the first.
    for s in &shows[first + 1..=last] {
        set_text(&mut ops[s.op], Vec::new(), fi.two_byte);
    }

    let page = doc.get_object(page_id)?.as_dict()?.clone();
    let mut new_page = page.clone();
    let outcome = if let Some(bytes) = encode_in_font(doc, fi, &full, &available) {
        set_text(&mut ops[lead.op], bytes, fi.two_byte);
        EditOutcome { method: EditMethod::InPlace, note: format!("edited in the original font ({})", short(&fi.base_font)) }
    } else {
        // Keep any prefix/suffix in the original font; remove the run itself and
        // draw the new text at the run's own text matrix in a standard font.
        let keep = format!("{prefix}{suffix}");
        let kept = if prefix.is_empty() && suffix.is_empty() {
            Some(Vec::new())
        } else {
            encode_in_font(doc, fi, &keep, &available).or_else(|| fi.encode(doc, &keep))
        };
        match (kept, suffix.is_empty()) {
            // Suffix shifts left when the run is removed — only safe without one.
            (Some(bytes), true) => set_text(&mut ops[lead.op], bytes, fi.two_byte),
            _ => return Ok(None),
        }
        let family = standard_font(&fi.base_font);
        let name = unique_font_name(&fonts);
        let enc = std14_encode(new_text);
        let (bytes, bad) = (enc.bytes.clone(), enc.unsupported.clone());
        let m = mul(&[1.0, 0.0, 0.0, 1.0, start_tx, 0.0], &lead.trm);
        ops.insert(0, Operation::new("q", vec![]));
        ops.push(Operation::new("Q", vec![]));
        let r = |v: f64| Object::Real(v as f32);
        ops.extend([
            Operation::new("q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(name.clone()), r(lead.size)]),
            Operation::new("Ts", vec![r(lead.rise)]),
            Operation::new("Tm", m.iter().map(|v| r(*v)).collect()),
            Operation::new("Tj", vec![Object::String(bytes, StringFormat::Literal)]),
            Operation::new("ET", vec![]),
            Operation::new("Q", vec![]),
        ]);
        new_page.set("Resources", Object::Dictionary(add_font_resource(doc, &page, &name, std14_font(family, &enc.diffs))));
        let mut note = format!(
            "{} lacks glyphs for the new text; replaced with {family}",
            short(&fi.base_font)
        );
        if !bad.is_empty() {
            note.push_str(&format!(" (unsupported: {})", bad.iter().collect::<String>()));
        }
        EditOutcome { method: EditMethod::Replaced, note }
    };

    let data = Content { operations: ops }.encode()?;
    let mut stream = Stream::new(dictionary! {}, data);
    let _ = stream.compress();
    let sid = doc.add_object(stream);
    new_page.set("Contents", Object::Reference(sid));
    doc.objects.insert(page_id, Object::Dictionary(new_page));
    Ok(Some(outcome))
}

fn short(base: &str) -> String {
    let b = base.split_once('+').map(|(_, r)| r).unwrap_or(base);
    if b.is_empty() { "font".into() } else { b.to_string() }
}

/// Legacy fallback: white patch over the run's box + Helvetica text.
fn overlay(doc: &mut PdfDoc, page_id: ObjectId, req: &EditRequest, new_text: &str) -> Result<EditOutcome, EngineError> {
    let size = req.font_size.max(1.0);
    let enc = std14_encode(new_text);
    let (bytes, bad) = (enc.bytes.clone(), enc.unsupported.clone());
    let page = doc.get_object(page_id)?.as_dict()?.clone();
    let fonts: BTreeMap<Vec<u8>, FontInfo> = doc
        .get_page_fonts(page_id)
        .map(|f| f.into_iter().map(|(k, d)| (k, font_info(doc, d))).collect())
        .unwrap_or_default();
    let name = unique_font_name(&fonts);
    let ops = vec![
        Operation::new("q", vec![]),
        Operation::new("rg", vec![1.into(), 1.into(), 1.into()]),
        Operation::new(
            "re",
            vec![
                Object::Real((req.x - 1.0) as f32),
                Object::Real((req.y - size * 0.25) as f32),
                Object::Real((req.width.max(size) + 2.0) as f32),
                Object::Real((size * 1.2) as f32),
            ],
        ),
        Operation::new("f", vec![]),
        Operation::new("Q", vec![]),
        Operation::new("q", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(name.clone()), Object::Real(size as f32)]),
        Operation::new("Td", vec![Object::Real(req.x as f32), Object::Real(req.y as f32)]),
        Operation::new("Tj", vec![Object::String(bytes, StringFormat::Literal)]),
        Operation::new("ET", vec![]),
        Operation::new("Q", vec![]),
    ];
    let data = Content { operations: ops }.encode()?;
    let sid = doc.add_object(Stream::new(dictionary! {}, data));
    let contents = match page.get(b"Contents") {
        Ok(Object::Reference(r)) => Object::Array(vec![Object::Reference(*r), Object::Reference(sid)]),
        Ok(Object::Array(arr)) => {
            let mut arr = arr.clone();
            arr.push(Object::Reference(sid));
            Object::Array(arr)
        }
        _ => Object::Reference(sid),
    };
    let mut new_page = page.clone();
    new_page.set("Resources", Object::Dictionary(add_font_resource(doc, &page, &name, std14_font("Helvetica", &enc.diffs))));
    new_page.set("Contents", contents);
    doc.objects.insert(page_id, Object::Dictionary(new_page));
    let mut note = "text not found in the page content; covered and redrawn in Helvetica".to_string();
    if !bad.is_empty() {
        note.push_str(&format!(" (unsupported: {})", bad.iter().collect::<String>()));
    }
    Ok(EditOutcome { method: EditMethod::Overlay, note })
}
