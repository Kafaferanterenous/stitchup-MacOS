//! Installed (system) fonts: find them, and embed a SUBSET of one into a PDF
//! as a Unicode (Type0 / Identity-H) font, so text can be set in any font on
//! this Mac and in any script the font covers (Cyrillic, Greek, CJK…).
//!
//! Only fonts whose licence allows embedding are offered (OS/2 fsType is not
//! "restricted"); colour/bitmap-only fonts and hidden system fonts are skipped.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use lopdf::{dictionary, Document as PdfDoc, Object, ObjectId, Stream, StringFormat};
use serde::Serialize;

use crate::error::{EngineError, ErrorCode};

#[derive(Debug, Clone, Serialize)]
pub struct SystemFace {
    pub family: String,
    pub style: String,
    /// PostScript name — the unique key used to pick a face.
    pub postscript: String,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    pub index: u32,
    pub bold: bool,
    pub italic: bool,
    pub weight: u16,
    pub monospace: bool,
    /// Has Cyrillic / Greek / CJK glyphs (for fallback choice and the UI).
    pub cyrillic: bool,
    pub greek: bool,
    pub cjk: bool,
}

/// Folders scanned for fonts (macOS); `STITCHUP_FONT_DIRS` (":"-separated) overrides.
pub fn font_dirs() -> Vec<PathBuf> {
    if let Ok(v) = std::env::var("STITCHUP_FONT_DIRS") {
        return v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    }
    let mut d = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/System/Library/Fonts/Supplemental"),
        PathBuf::from("/Library/Fonts"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        d.push(Path::new(&home).join("Library/Fonts"));
    }
    d
}

fn name_of(face: &ttf_parser::Face, ids: &[u16]) -> Option<String> {
    for id in ids {
        let mut best: Option<String> = None;
        for n in face.names() {
            if n.name_id == *id && n.is_unicode() {
                if let Some(s) = n.to_string() {
                    // Prefer English (Windows en-US 0x0409, or Mac/Unicode language 0).
                    if n.language_id == 0x0409 || n.language_id == 0 {
                        return Some(s);
                    }
                    best.get_or_insert(s);
                }
            }
        }
        if best.is_some() {
            return best;
        }
    }
    None
}

fn scan_file(path: &Path, out: &mut Vec<SystemFace>) {
    let Ok(data) = std::fs::read(path) else { return };
    let n = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    for index in 0..n {
        let Ok(face) = ttf_parser::Face::parse(&data, index) else { continue };
        if matches!(face.permissions(), Some(ttf_parser::Permissions::Restricted)) {
            continue;
        }
        let raw = face.raw_face();
        let outlines = raw.table(ttf_parser::Tag::from_bytes(b"glyf")).is_some() || raw.table(ttf_parser::Tag::from_bytes(b"CFF ")).is_some();
        if !outlines || face.glyph_index('A').is_none() && face.glyph_index('\u{4e00}').is_none() && face.glyph_index('\u{0410}').is_none() {
            continue;
        }
        let Some(postscript) = name_of(&face, &[6]) else { continue };
        let Some(family) = name_of(&face, &[16, 1]) else { continue };
        if family.starts_with('.') || postscript.starts_with('.') || family == "LastResort" {
            continue;
        }
        let style = name_of(&face, &[17, 2]).unwrap_or_else(|| "Regular".into());
        out.push(SystemFace {
            family,
            style,
            postscript,
            path: path.to_path_buf(),
            index,
            bold: face.is_bold() || face.weight().to_number() >= 600,
            italic: face.is_italic() || face.is_oblique(),
            weight: face.weight().to_number(),
            monospace: face.is_monospaced(),
            cyrillic: face.glyph_index('Ж').is_some(),
            greek: face.glyph_index('Ω').is_some() && face.glyph_index('λ').is_some(),
            cjk: face.glyph_index('中').is_some() || face.glyph_index('あ').is_some(),
        });
    }
}

fn scan() -> Vec<SystemFace> {
    let mut out = vec![];
    for dir in font_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                scan_file(&f, &mut out);
            }
        }
    }
    // One entry per PostScript name (the same font may be installed twice).
    out.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()).then(a.weight.cmp(&b.weight)).then(a.italic.cmp(&b.italic)));
    let mut seen = std::collections::HashSet::new();
    out.retain(|f| seen.insert(f.postscript.clone()));
    out
}

static FACES: OnceLock<Vec<SystemFace>> = OnceLock::new();

/// Every embeddable installed face (scanned once, ~1–2 s on first use).
pub fn system_faces() -> &'static [SystemFace] {
    FACES.get_or_init(scan)
}

pub fn find_face(postscript: &str) -> Option<&'static SystemFace> {
    system_faces().iter().find(|f| f.postscript == postscript)
}

/// A face of `family` closest to the requested weight / slant.
pub fn face_in_family(family: &str, bold: bool, italic: bool) -> Option<&'static SystemFace> {
    let want = if bold { 700 } else { 400 };
    system_faces()
        .iter()
        .filter(|f| f.family.eq_ignore_ascii_case(family))
        .min_by_key(|f| (f.italic != italic) as i32 * 10_000 + (f.weight as i32 - want).abs() + (f.style.contains("Condensed") || f.style.contains("Narrow")) as i32 * 500)
}

/// Installed face to stand in when `text` has letters a standard font lacks
/// (Cyrillic, Greek, CJK…): same look (serif / sans / mono, bold, italic) where possible.
pub fn fallback_face(text: &str, serif: bool, mono: bool, bold: bool, italic: bool) -> Option<&'static SystemFace> {
    let families: &[&str] = if mono {
        &["Menlo", "Courier New", "PT Mono", "Andale Mono"]
    } else if serif {
        &["Times New Roman", "Georgia", "PT Serif", "Times"]
    } else {
        &["Helvetica Neue", "Arial", "Helvetica", "PT Sans", "Lucida Grande"]
    };
    let covers = |f: &SystemFace| -> bool {
        let Ok(data) = std::fs::read(&f.path) else { return false };
        let Ok(face) = ttf_parser::Face::parse(&data, f.index) else { return false };
        text.chars().all(|c| c.is_whitespace() || face.glyph_index(c).is_some())
    };
    for fam in families {
        if let Some(f) = face_in_family(fam, bold, italic) {
            if covers(f) {
                return Some(f);
            }
        }
    }
    // Any face that covers the text (CJK: Hiragino / PingFang / Arial Unicode…).
    let cjk = text.chars().any(|c| ('\u{2e80}'..='\u{9fff}').contains(&c) || ('\u{ac00}'..='\u{d7af}').contains(&c));
    let preferred: &[&str] = if cjk { &["Hiragino Sans", "Hiragino Sans GB", "PingFang SC", "Apple SD Gothic Neo", "Arial Unicode MS"] } else { &["Arial Unicode MS"] };
    for fam in preferred {
        if let Some(f) = face_in_family(fam, bold, italic) {
            if covers(f) {
                return Some(f);
            }
        }
    }
    system_faces().iter().filter(|f| !f.bold && !f.italic).find(|f| covers(f))
}

/// Text set in an embedded subset font.
pub struct EmbeddedText {
    /// Font dictionary (Type0) — put it in the page /Font resources.
    pub font: ObjectId,
    /// 2-byte codes (CIDs) to show with Tj.
    pub bytes: Vec<u8>,
    /// Advance of the whole text at size 1, in text space (sum of widths / 1000).
    pub advance: f64,
    /// Characters the font has no glyph for (shown as the font's blank glyph).
    pub missing: Vec<char>,
}

/// Measure `text` in `face` (advance at size 1, in text space) and which chars it lacks.
pub fn measure(face: &SystemFace, text: &str) -> Result<(f64, Vec<char>), EngineError> {
    let data = std::fs::read(&face.path)?;
    let f = ttf_parser::Face::parse(&data, face.index).map_err(|e| crate::err!(ErrorCode::IoError, "font {}: {e}", face.postscript))?;
    let upm = f.units_per_em() as f64;
    let mut adv = 0.0;
    let mut missing = vec![];
    for c in text.chars() {
        match f.glyph_index(c) {
            Some(g) => adv += f.glyph_hor_advance(g).unwrap_or(0) as f64 / upm,
            None => {
                if !missing.contains(&c) {
                    missing.push(c);
                }
            }
        }
    }
    Ok((adv, missing))
}

fn subset_tag(seed: &[u8]) -> String {
    let h = crate::hash::sha256_hex(seed);
    h.bytes().take(6).map(|b| (b'A' + b % 26) as char).collect()
}

/// Embed the glyphs `text` needs from `face` and encode the text for them.
pub fn embed_text(doc: &mut PdfDoc, face: &SystemFace, text: &str) -> Result<EmbeddedText, EngineError> {
    let data = std::fs::read(&face.path)?;
    let f = ttf_parser::Face::parse(&data, face.index).map_err(|e| crate::err!(ErrorCode::IoError, "font {}: {e}", face.postscript))?;
    let upm = f.units_per_em() as f64;
    let scale = 1000.0 / upm;

    let mut remap = subsetter::GlyphRemapper::new();
    remap.remap(0); // .notdef stays glyph 0
    let mut glyphs: Vec<(char, u16)> = vec![];
    let mut missing = vec![];
    for c in text.chars() {
        let g = match f.glyph_index(c) {
            Some(g) => g.0,
            None => {
                if !missing.contains(&c) {
                    missing.push(c);
                }
                0
            }
        };
        remap.remap(g);
        glyphs.push((c, g));
    }
    let sub = subsetter::subset(&data, face.index, &remap).map_err(|e| crate::err!(ErrorCode::Unsupported, "can't embed {}: {e:?}", face.postscript))?;

    let mut bytes = Vec::with_capacity(glyphs.len() * 2);
    let mut advance = 0.0;
    let mut widths: std::collections::BTreeMap<u16, f64> = Default::default();
    let mut to_unicode: std::collections::BTreeMap<u16, char> = Default::default();
    for (c, g) in &glyphs {
        let cid = remap.get(*g).unwrap_or(0);
        bytes.extend_from_slice(&cid.to_be_bytes());
        let w = f.glyph_hor_advance(ttf_parser::GlyphId(*g)).unwrap_or(0) as f64 * scale;
        advance += w / 1000.0;
        widths.insert(cid, w);
        if *g != 0 {
            to_unicode.entry(cid).or_insert(*c);
        }
    }

    // Font program: TrueType outlines → FontFile2; CFF → the CFF table as FontFile3.
    let raw_sub = ttf_parser::RawFace::parse(&sub, 0).map_err(|e| crate::err!(ErrorCode::IoError, "subset: {e}"))?;
    let (file_key, program, cid_subtype) = match raw_sub.table(ttf_parser::Tag::from_bytes(b"CFF ")) {
        Some(cff) => {
            let mut s = Stream::new(dictionary! { "Subtype" => "CIDFontType0C" }, cff.to_vec());
            let _ = s.compress();
            ("FontFile3", s, "CIDFontType0")
        }
        None => {
            let mut s = Stream::new(dictionary! { "Length1" => sub.len() as i64 }, sub.clone());
            let _ = s.compress();
            ("FontFile2", s, "CIDFontType2")
        }
    };
    let program_id = doc.add_object(program);

    let base = format!("{}+{}", subset_tag(&[face.postscript.as_bytes(), text.as_bytes()].concat()), face.postscript.replace(' ', ""));
    let bb = f.global_bounding_box();
    let serif = !face.monospace && (face.family.contains("Times") || face.family.contains("Georgia") || face.family.contains("Serif") || face.family.contains("Garamond") || face.family.contains("Baskerville") || face.family.contains("Palatino"));
    let mut flags = 32; // nonsymbolic
    if face.monospace {
        flags |= 1;
    }
    if serif {
        flags |= 2;
    }
    if face.italic {
        flags |= 64;
    }
    let s = |v: f64| Object::Real(v as f32);
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(base.clone().into_bytes()),
        "Flags" => flags,
        "FontBBox" => vec![s(bb.x_min as f64 * scale), s(bb.y_min as f64 * scale), s(bb.x_max as f64 * scale), s(bb.y_max as f64 * scale)],
        "ItalicAngle" => s(f.italic_angle() as f64),
        "Ascent" => s(f.ascender() as f64 * scale),
        "Descent" => s(f.descender() as f64 * scale),
        "CapHeight" => s(f.capital_height().unwrap_or(f.ascender()) as f64 * scale),
        "StemV" => if face.bold { 120 } else { 80 },
        file_key => program_id,
    });
    let mut w = vec![];
    for (cid, width) in &widths {
        w.push(Object::Integer(*cid as i64));
        w.push(Object::Array(vec![s(*width)]));
    }
    let mut cid_font = dictionary! {
        "Type" => "Font",
        "Subtype" => cid_subtype,
        "BaseFont" => Object::Name(base.clone().into_bytes()),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "FontDescriptor" => descriptor,
        "DW" => 1000,
        "W" => w,
    };
    if cid_subtype == "CIDFontType2" {
        cid_font.set("CIDToGIDMap", "Identity");
    }
    let cid_id = doc.add_object(cid_font);

    // ToUnicode CMap so the text stays searchable / copyable.
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<_> = to_unicode.iter().collect();
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, c) in chunk {
            let mut u = [0u16; 2];
            let hex: String = c.encode_utf16(&mut u).iter().map(|x| format!("{x:04X}")).collect();
            cmap.push_str(&format!("<{cid:04X}> <{hex}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let mut tu = Stream::new(dictionary! {}, cmap.into_bytes());
    let _ = tu.compress();
    let tu_id = doc.add_object(tu);

    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => Object::Name(base.into_bytes()),
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(cid_id)],
        "ToUnicode" => tu_id,
    });
    Ok(EmbeddedText { font, bytes, advance, missing })
}

/// The `Tj` operand for embedded text.
pub fn hex_string(bytes: Vec<u8>) -> Object {
    Object::String(bytes, StringFormat::Hexadecimal)
}
