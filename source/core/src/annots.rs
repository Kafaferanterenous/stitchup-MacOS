//! Annotations (PDF 32000 §12.5): create, move, delete — each created
//! annotation carries its own appearance stream so every viewer (Preview,
//! Acrobat, pdf.js) draws it identically.
//!
//! Undo safety: existing annotation objects are never modified. Creating adds
//! new objects; moving clones the annotation (shifted) under a new id; both
//! only rewrite the page's /Annots (kept inline in the page dictionary, which
//! the op log snapshots).

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document as PdfDoc, Object, ObjectId, Stream, StringFormat};
use serde::Deserialize;

use crate::error::{EngineError, ErrorCode};

type Rgb = [f64; 3];

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnnotSpec {
    /// Text markup over quads (x0,y0,x1,y1 per box).
    Highlight { boxes: Vec<[f64; 4]>, color: Rgb },
    Underline { boxes: Vec<[f64; 4]>, color: Rgb },
    StrikeOut { boxes: Vec<[f64; 4]>, color: Rgb },
    /// Sticky note at a point (top-left of the icon).
    Note { at: [f64; 2], text: String, color: Rgb },
    TextBox { rect: [f64; 4], text: String, size: f64, color: Rgb },
    Square { rect: [f64; 4], color: Rgb, fill: Option<Rgb>, width: f64 },
    Circle { rect: [f64; 4], color: Rgb, fill: Option<Rgb>, width: f64 },
    Line { from: [f64; 2], to: [f64; 2], color: Rgb, width: f64, arrow: bool },
    Ink { strokes: Vec<Vec<[f64; 2]>>, color: Rgb, width: f64 },
    Stamp { rect: [f64; 4], label: String, color: Rgb },
    /// Web link (`uri`) or jump to a page (`page`, 1-based).
    Link { rect: [f64; 4], uri: Option<String>, page: Option<u32> },
}

fn r(v: f64) -> Object {
    Object::Real(v as f32)
}

fn arr(v: &[f64]) -> Object {
    Object::Array(v.iter().map(|x| r(*x)).collect())
}

fn norm(rect: [f64; 4]) -> [f64; 4] {
    [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])]
}

fn union(boxes: &[[f64; 4]]) -> [f64; 4] {
    boxes.iter().map(|b| norm(*b)).fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, b| {
        [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
    })
}

fn now_date() -> Object {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Same civil-date algorithm as sign.rs (UTC).
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    Object::string_literal(format!("D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60))
}

/// Form XObject appearance for an annotation whose /Rect is `rect`: `ops`
/// are drawn in rect-local coordinates (origin = rect lower-left).
fn appearance(doc: &mut PdfDoc, rect: [f64; 4], ops: Vec<Operation>, resources: Dictionary) -> Result<ObjectId, EngineError> {
    let [x0, y0, x1, y1] = norm(rect);
    let data = Content { operations: ops }.encode()?;
    let mut s = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => arr(&[0.0, 0.0, x1 - x0, y1 - y0]),
            "Resources" => resources,
        },
        data,
    );
    let _ = s.compress();
    Ok(doc.add_object(s))
}

fn op(name: &str, args: Vec<Object>) -> Operation {
    Operation::new(name, args)
}

fn color_ops(c: Rgb, stroke: bool) -> Operation {
    op(if stroke { "RG" } else { "rg" }, vec![r(c[0]), r(c[1]), r(c[2])])
}

/// Ellipse inscribed in (0,0,w,h) as four Béziers.
fn ellipse(w: f64, h: f64, inset: f64) -> Vec<Operation> {
    let (rx, ry, cx, cy) = (w / 2.0 - inset, h / 2.0 - inset, w / 2.0, h / 2.0);
    let k = 0.552_284_75;
    vec![
        op("m", vec![r(cx + rx), r(cy)]),
        op("c", vec![r(cx + rx), r(cy + k * ry), r(cx + k * rx), r(cy + ry), r(cx), r(cy + ry)]),
        op("c", vec![r(cx - k * rx), r(cy + ry), r(cx - rx), r(cy + k * ry), r(cx - rx), r(cy)]),
        op("c", vec![r(cx - rx), r(cy - k * ry), r(cx - k * rx), r(cy - ry), r(cx), r(cy - ry)]),
        op("c", vec![r(cx + k * rx), r(cy - ry), r(cx + rx), r(cy - k * ry), r(cx + rx), r(cy)]),
    ]
}

/// Greedy word wrap using Helvetica-Bold advances (slightly generous for regular).
fn wrap(text: &str, size: f64, width: f64) -> Vec<String> {
    let mut lines = vec![];
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if !cur.is_empty() && crate::docops::helv_bold_width(&trial, size) * 0.93 > width {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else {
                cur = trial;
            }
        }
        lines.push(cur);
    }
    lines
}

/// Text lines drawn with one standard font; returns (ops, font dict).
fn text_block(lines: &[String], font: &str, size: f64, x: f64, top: f64, color: Rgb) -> (Vec<Operation>, Object) {
    let all = lines.join("\n");
    let enc = crate::textedit::std14_encode(&all);
    let mut ops = vec![op("BT", vec![]), color_ops(color, false), op("Tf", vec![Object::Name(b"F1".to_vec()), r(size)])];
    let mut y = top - size;
    let mut pos = 0;
    for l in lines {
        let n = l.chars().count();
        let bytes = enc.bytes[pos..pos + n].to_vec();
        pos += n + 1; // skip the joined '\n'
        ops.push(op("Tm", vec![r(1.0), r(0.0), r(0.0), r(1.0), r(x), r(y)]));
        ops.push(op("Tj", vec![Object::String(bytes, StringFormat::Literal)]));
        y -= size * 1.2;
    }
    ops.push(op("ET", vec![]));
    (ops, crate::textedit::std14_font(font, &enc.diffs))
}

fn base(subtype: &str, rect: [f64; 4], color: Option<Rgb>) -> Dictionary {
    let mut d = dictionary! {
        "Type" => "Annot",
        "Subtype" => Object::Name(subtype.as_bytes().to_vec()),
        "Rect" => arr(&norm(rect)),
        "F" => 4, // Print
        "M" => now_date(),
        "NM" => Object::string_literal(format!("stitchup-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0))),
    };
    if let Some(c) = color {
        d.set("C", arr(&c));
    }
    d
}

fn quads(boxes: &[[f64; 4]]) -> Object {
    let mut v = vec![];
    for b in boxes {
        let [x0, y0, x1, y1] = norm(*b);
        // Upper-left, upper-right, lower-left, lower-right (Acrobat order).
        v.extend([x0, y1, x1, y1, x0, y0, x1, y0]);
    }
    arr(&v)
}

/// Build the annotation dictionary (+ appearance objects) for `spec`.
fn build(doc: &mut PdfDoc, page_id: ObjectId, spec: &AnnotSpec) -> Result<Dictionary, EngineError> {
    let page_ref = |d: &mut Dictionary| d.set("P", page_id);
    let mut d = match spec {
        AnnotSpec::Highlight { boxes, color } | AnnotSpec::Underline { boxes, color } | AnnotSpec::StrikeOut { boxes, color } => {
            if boxes.is_empty() {
                return Err(crate::err!(ErrorCode::ValidationFailed, "nothing to mark up"));
            }
            let rect = union(boxes);
            let kind = match spec {
                AnnotSpec::Highlight { .. } => "Highlight",
                AnnotSpec::Underline { .. } => "Underline",
                _ => "StrikeOut",
            };
            let mut ops = vec![op("q", vec![])];
            let mut res = Dictionary::new();
            if kind == "Highlight" {
                res.set("ExtGState", dictionary! { "GM" => dictionary! { "Type" => "ExtGState", "BM" => "Multiply", "ca" => 1.0, "CA" => 1.0 } });
                ops.push(op("gs", vec![Object::Name(b"GM".to_vec())]));
                ops.push(color_ops(*color, false));
            } else {
                ops.push(color_ops(*color, true));
            }
            for b in boxes {
                let [x0, y0, x1, y1] = norm(*b);
                let (lx, ly, w, h) = (x0 - rect[0], y0 - rect[1], x1 - x0, y1 - y0);
                match kind {
                    "Highlight" => ops.extend([op("re", vec![r(lx), r(ly), r(w), r(h)]), op("f", vec![])]),
                    _ => {
                        let y = if kind == "Underline" { ly + h * 0.08 } else { ly + h * 0.42 };
                        ops.extend([
                            op("w", vec![r((h * 0.07).max(0.6))]),
                            op("m", vec![r(lx), r(y)]),
                            op("l", vec![r(lx + w), r(y)]),
                            op("S", vec![]),
                        ]);
                    }
                }
            }
            ops.push(op("Q", vec![]));
            let ap = appearance(doc, rect, ops, res)?;
            let mut d = base(kind, rect, Some(*color));
            d.set("QuadPoints", quads(boxes));
            if kind == "Highlight" {
                d.set("CA", r(1.0));
            }
            d.set("AP", dictionary! { "N" => ap });
            d
        }
        AnnotSpec::Note { at, text, color } => {
            let rect = [at[0], at[1] - 22.0, at[0] + 22.0, at[1]];
            let ops = vec![
                op("q", vec![]),
                color_ops(*color, false),
                op("RG", vec![r(0.35), r(0.3), r(0.0)]),
                op("w", vec![r(1.0)]),
                op("m", vec![r(1.0), r(21.0)]),
                op("l", vec![r(21.0), r(21.0)]),
                op("l", vec![r(21.0), r(7.0)]),
                op("l", vec![r(15.0), r(1.0)]),
                op("l", vec![r(1.0), r(1.0)]),
                op("h", vec![]),
                op("B", vec![]),
                op("m", vec![r(5.0), r(15.5)]),
                op("l", vec![r(17.0), r(15.5)]),
                op("m", vec![r(5.0), r(11.5)]),
                op("l", vec![r(17.0), r(11.5)]),
                op("m", vec![r(5.0), r(7.5)]),
                op("l", vec![r(12.0), r(7.5)]),
                op("S", vec![]),
                op("Q", vec![]),
            ];
            let ap = appearance(doc, rect, ops, Dictionary::new())?;
            let mut d = base("Text", rect, Some(*color));
            d.set("Contents", lopdf::text_string(text));
            d.set("Name", "Comment");
            d.set("Open", false);
            d.set("F", 28); // Print | NoZoom | NoRotate
            d.set("AP", dictionary! { "N" => ap });
            d
        }
        AnnotSpec::TextBox { rect, text, size, color } => {
            let rect = norm(*rect);
            let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
            let size = size.clamp(4.0, 144.0);
            let lines = wrap(text, size, w - 6.0);
            let (mut tops, font) = text_block(&lines, "Helvetica", size, 3.0, h - 3.0, *color);
            let mut ops = vec![op("q", vec![]), op("re", vec![r(0.0), r(0.0), r(w), r(h)]), op("W", vec![]), op("n", vec![])];
            ops.append(&mut tops);
            ops.push(op("Q", vec![]));
            let ap = appearance(doc, rect, ops, dictionary! { "Font" => dictionary! { "F1" => font } })?;
            let mut d = base("FreeText", rect, None);
            d.set("Contents", lopdf::text_string(text));
            d.set("DA", Object::string_literal(format!("/Helv {size:.1} Tf {:.3} {:.3} {:.3} rg", color[0], color[1], color[2])));
            d.set("Border", vec![0.into(), 0.into(), 0.into()]);
            d.set("AP", dictionary! { "N" => ap });
            d
        }
        AnnotSpec::Square { rect, color, fill, width } | AnnotSpec::Circle { rect, color, fill, width } => {
            let rect = norm(*rect);
            let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
            let lw = width.clamp(0.25, 24.0);
            let mut ops = vec![op("q", vec![]), op("w", vec![r(lw)]), color_ops(*color, true)];
            if let Some(f) = fill {
                ops.push(color_ops(*f, false));
            }
            let circle = matches!(spec, AnnotSpec::Circle { .. });
            if circle {
                ops.extend(ellipse(w, h, lw / 2.0));
            } else {
                ops.push(op("re", vec![r(lw / 2.0), r(lw / 2.0), r(w - lw), r(h - lw)]));
            }
            ops.push(op(if fill.is_some() { "B" } else { "S" }, vec![]));
            ops.push(op("Q", vec![]));
            let ap = appearance(doc, rect, ops, Dictionary::new())?;
            let mut d = base(if circle { "Circle" } else { "Square" }, rect, Some(*color));
            if let Some(f) = fill {
                d.set("IC", arr(f));
            }
            d.set("BS", dictionary! { "W" => r(lw), "S" => "S" });
            d.set("AP", dictionary! { "N" => ap });
            d
        }
        AnnotSpec::Line { from, to, color, width, arrow } => {
            let lw = width.clamp(0.25, 24.0);
            let pad = lw * 4.0 + 4.0;
            let rect = [from[0].min(to[0]) - pad, from[1].min(to[1]) - pad, from[0].max(to[0]) + pad, from[1].max(to[1]) + pad];
            let (fx, fy, tx, ty) = (from[0] - rect[0], from[1] - rect[1], to[0] - rect[0], to[1] - rect[1]);
            let mut ops = vec![
                op("q", vec![]),
                op("w", vec![r(lw)]),
                op("J", vec![1.into()]),
                op("j", vec![1.into()]),
                color_ops(*color, true),
                op("m", vec![r(fx), r(fy)]),
                op("l", vec![r(tx), r(ty)]),
                op("S", vec![]),
            ];
            if *arrow {
                let ang = (ty - fy).atan2(tx - fx);
                let len = lw * 3.0 + 6.0;
                let a1 = ang + 2.6;
                let a2 = ang - 2.6;
                ops.extend([
                    op("m", vec![r(tx + len * a1.cos()), r(ty + len * a1.sin())]),
                    op("l", vec![r(tx), r(ty)]),
                    op("l", vec![r(tx + len * a2.cos()), r(ty + len * a2.sin())]),
                    op("S", vec![]),
                ]);
            }
            ops.push(op("Q", vec![]));
            let ap = appearance(doc, rect, ops, Dictionary::new())?;
            let mut d = base("Line", rect, Some(*color));
            d.set("L", arr(&[from[0], from[1], to[0], to[1]]));
            d.set("BS", dictionary! { "W" => r(lw) });
            if *arrow {
                d.set("LE", vec![Object::Name(b"None".to_vec()), Object::Name(b"OpenArrow".to_vec())]);
            }
            d.set("AP", dictionary! { "N" => ap });
            d
        }
        AnnotSpec::Ink { strokes, color, width } => {
            let pts: Vec<[f64; 2]> = strokes.iter().flatten().copied().collect();
            if pts.len() < 2 {
                return Err(crate::err!(ErrorCode::ValidationFailed, "draw a longer stroke"));
            }
            let lw = width.clamp(0.25, 24.0);
            let rect = [
                pts.iter().map(|p| p[0]).fold(f64::MAX, f64::min) - lw,
                pts.iter().map(|p| p[1]).fold(f64::MAX, f64::min) - lw,
                pts.iter().map(|p| p[0]).fold(f64::MIN, f64::max) + lw,
                pts.iter().map(|p| p[1]).fold(f64::MIN, f64::max) + lw,
            ];
            let mut ops = vec![op("q", vec![]), op("w", vec![r(lw)]), op("J", vec![1.into()]), op("j", vec![1.into()]), color_ops(*color, true)];
            for s in strokes.iter().filter(|s| !s.is_empty()) {
                ops.push(op("m", vec![r(s[0][0] - rect[0]), r(s[0][1] - rect[1])]));
                for p in &s[1..] {
                    ops.push(op("l", vec![r(p[0] - rect[0]), r(p[1] - rect[1])]));
                }
                ops.push(op("S", vec![]));
            }
            ops.push(op("Q", vec![]));
            let ap = appearance(doc, rect, ops, Dictionary::new())?;
            let mut d = base("Ink", rect, Some(*color));
            d.set(
                "InkList",
                Object::Array(strokes.iter().map(|s| arr(&s.iter().flat_map(|p| [p[0], p[1]]).collect::<Vec<_>>())).collect()),
            );
            d.set("BS", dictionary! { "W" => r(lw) });
            d.set("AP", dictionary! { "N" => ap });
            d
        }
        AnnotSpec::Stamp { rect, label, color } => {
            let rect = norm(*rect);
            let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
            let text = label.trim().to_uppercase();
            let size = (h * 0.55).min((w - 10.0) / (crate::docops::helv_bold_width(&text, 1.0).max(0.1))).max(4.0);
            let tw = crate::docops::helv_bold_width(&text, size);
            let (mut tops, font) = text_block(&[text.clone()], "Helvetica-Bold", size, (w - tw) / 2.0, (h + size * 0.72) / 2.0 + size * 0.28, *color);
            let lw = (h * 0.06).clamp(1.0, 4.0);
            let mut ops = vec![
                op("q", vec![]),
                op("gs", vec![Object::Name(b"GA".to_vec())]),
                op("w", vec![r(lw)]),
                color_ops(*color, true),
                op("re", vec![r(lw), r(lw), r(w - lw * 2.0), r(h - lw * 2.0)]),
                op("S", vec![]),
            ];
            ops.append(&mut tops);
            ops.push(op("Q", vec![]));
            let res = dictionary! {
                "Font" => dictionary! { "F1" => font },
                "ExtGState" => dictionary! { "GA" => dictionary! { "Type" => "ExtGState", "ca" => 0.85, "CA" => 0.85 } },
            };
            let ap = appearance(doc, rect, ops, res)?;
            let mut d = base("Stamp", rect, Some(*color));
            let name: String = text.split_whitespace().map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default()
            }).collect();
            d.set("Name", Object::Name(name.into_bytes()));
            d.set("Contents", lopdf::text_string(label.trim()));
            d.set("AP", dictionary! { "N" => ap });
            d
        }
        AnnotSpec::Link { rect, uri, page } => {
            let mut d = base("Link", *rect, None);
            d.set("Border", vec![0.into(), 0.into(), 0.into()]);
            if let Some(u) = uri.as_ref().filter(|u| !u.trim().is_empty()) {
                let u = u.trim();
                let u = if u.contains("://") || u.starts_with("mailto:") { u.to_string() } else { format!("https://{u}") };
                d.set("A", dictionary! { "S" => "URI", "URI" => Object::string_literal(u) });
            } else if let Some(p) = page {
                let target = *doc.get_pages().get(p).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {p}"))?;
                d.set("Dest", vec![Object::Reference(target), Object::Name(b"Fit".to_vec())]);
            } else {
                return Err(crate::err!(ErrorCode::ValidationFailed, "a link needs a web address or a page"));
            }
            d
        }
    };
    page_ref(&mut d);
    Ok(d)
}

/// The page's /Annots as an owned list (inline or referenced array).
fn annots_of(doc: &PdfDoc, page: &Dictionary) -> Vec<Object> {
    match page.get(b"Annots") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => doc.get_object(*r).and_then(Object::as_array).cloned().unwrap_or_default(),
        _ => vec![],
    }
}

fn page_id(doc: &PdfDoc, page_no: u32) -> Result<ObjectId, EngineError> {
    doc.get_pages().get(&page_no).copied().ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {page_no}"))
}

fn set_annots(doc: &mut PdfDoc, page: ObjectId, annots: Vec<Object>) -> Result<(), EngineError> {
    let mut p = doc.get_object(page)?.as_dict()?.clone();
    if annots.is_empty() {
        p.remove(b"Annots");
    } else {
        p.set("Annots", annots);
    }
    doc.objects.insert(page, Object::Dictionary(p));
    Ok(())
}

/// Add an annotation to a page; returns the new object id.
pub fn add(doc: &mut PdfDoc, page_no: u32, spec: &AnnotSpec) -> Result<ObjectId, EngineError> {
    let page = page_id(doc, page_no)?;
    let dict = build(doc, page, spec)?;
    let id = doc.add_object(dict);
    let mut annots = annots_of(doc, doc.get_object(page)?.as_dict()?);
    annots.push(Object::Reference(id));
    set_annots(doc, page, annots)?;
    Ok(id)
}

/// Remove an annotation (by object number) from a page.
pub fn delete(doc: &mut PdfDoc, page_no: u32, obj: u32) -> Result<(), EngineError> {
    let page = page_id(doc, page_no)?;
    let annots = annots_of(doc, doc.get_object(page)?.as_dict()?);
    let before = annots.len();
    let kept: Vec<Object> = annots.into_iter().filter(|a| a.as_reference().map(|r| r.0 != obj).unwrap_or(true)).collect();
    if kept.len() == before {
        return Err(crate::err!(ErrorCode::ValidationFailed, "annotation {obj} is not on page {page_no}"));
    }
    set_annots(doc, page, kept)
}

fn shift_nums(o: &Object, dx: f64, dy: f64) -> Object {
    match o {
        Object::Array(a) => Object::Array(
            a.iter()
                .enumerate()
                .map(|(i, v)| match v.as_float() {
                    Ok(f) => r(f as f64 + if i % 2 == 0 { dx } else { dy }),
                    Err(_) => shift_nums(v, dx, dy),
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Move an annotation by (dx, dy) points. The original object is left as is
/// (undo restores it); a shifted copy takes its place in /Annots.
pub fn translate(doc: &mut PdfDoc, page_no: u32, obj: u32, dx: f64, dy: f64) -> Result<u32, EngineError> {
    let page = page_id(doc, page_no)?;
    let mut annots = annots_of(doc, doc.get_object(page)?.as_dict()?);
    let pos = annots
        .iter()
        .position(|a| a.as_reference().map(|r| r.0 == obj).unwrap_or(false))
        .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "annotation {obj} is not on page {page_no}"))?;
    let old_id = annots[pos].as_reference()?;
    let mut d = doc.get_object(old_id)?.as_dict()?.clone();
    for key in [&b"Rect"[..], b"QuadPoints", b"L", b"Vertices", b"CL"] {
        if let Ok(v) = d.get(key) {
            let v = shift_nums(v, dx, dy);
            d.set(key.to_vec(), v);
        }
    }
    if let Ok(Object::Array(ink)) = d.get(b"InkList") {
        let ink = Object::Array(ink.iter().map(|s| shift_nums(s, dx, dy)).collect());
        d.set("InkList", ink);
    }
    d.set("M", now_date());
    let new_id = doc.add_object(d);
    annots[pos] = Object::Reference(new_id);
    set_annots(doc, page, annots)?;
    Ok(new_id.0)
}

/// Annotations (object number, subtype, rect) on a page — used by redaction.
pub fn rects(doc: &PdfDoc, page_no: u32) -> Vec<(u32, String, [f64; 4])> {
    let Ok(page) = page_id(doc, page_no) else { return vec![] };
    let Ok(p) = doc.get_object(page).and_then(Object::as_dict) else { return vec![] };
    annots_of(doc, p)
        .iter()
        .filter_map(|a| {
            let id = a.as_reference().ok()?;
            let d = doc.get_object(id).ok()?.as_dict().ok()?;
            let sub = String::from_utf8_lossy(d.get(b"Subtype").ok()?.as_name().ok()?).into_owned();
            let v: Vec<f64> = d.get(b"Rect").ok()?.as_array().ok()?.iter().filter_map(|o| o.as_float().ok().map(|f| f as f64)).collect();
            (v.len() == 4).then(|| (id.0, sub, norm([v[0], v[1], v[2], v[3]])))
        })
        .collect()
}

/// Recolour an annotation: its /C colour, and every colour operator in its
/// appearance (and FreeText /DA) that used the old colour. The annotation is
/// replaced by an updated copy under a new object number (like `translate`).
pub fn recolor(doc: &mut PdfDoc, page_no: u32, obj: u32, rgb: [f64; 3]) -> Result<u32, EngineError> {
    let page = page_id(doc, page_no)?;
    let mut annots = annots_of(doc, doc.get_object(page)?.as_dict()?);
    let pos = annots
        .iter()
        .position(|a| a.as_reference().map(|r| r.0 == obj).unwrap_or(false))
        .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "annotation {obj} is not on page {page_no}"))?;
    let old_id = annots[pos].as_reference()?;
    let mut d = doc.get_object(old_id)?.as_dict()?.clone();
    if matches!(d.get(b"Subtype").and_then(Object::as_name), Ok(b"Widget") | Ok(b"Popup") | Ok(b"Link")) {
        return Err(crate::err!(ErrorCode::ValidationFailed, "form fields and links keep their colours"));
    }
    let nums = |o: &Object| -> Vec<f64> { o.as_array().map(|a| a.iter().filter_map(|v| v.as_float().ok().map(|f| f as f64)).collect()).unwrap_or_default() };
    let old: Vec<f64> = d.get(b"C").map(nums).unwrap_or_default();
    let new_obj = |c: &[f64; 3]| -> Vec<Object> { c.iter().map(|v| Object::Real(*v as f32)).collect() };
    d.set("C", new_obj(&rgb));
    let same = |v: &[f64]| old.len() == 3 && v.len() == 3 && v.iter().zip(&old).all(|(a, b)| (a - b).abs() < 0.004);

    // Appearance stream: swap the old colour for the new one.
    let ap_ref = d.get(b"AP").ok().and_then(|ap| ap.as_dict().ok()).and_then(|ap| ap.get(b"N").ok()).and_then(|n| n.as_reference().ok());
    if let Some(r) = ap_ref {
        if let Ok(Object::Stream(s)) = doc.get_object(r).cloned() {
            if let Ok(content) = s.decompressed_content().and_then(|c| lopdf::content::Content::decode(&c)) {
                let mut ops = content.operations;
                let mut changed = false;
                for op in ops.iter_mut() {
                    if matches!(op.operator.as_str(), "RG" | "rg" | "SC" | "sc" | "SCN" | "scn") {
                        let v: Vec<f64> = op.operands.iter().filter_map(|o| o.as_float().ok().map(|f| f as f64)).collect();
                        // No old /C (e.g. a stamp drawn by another app): change the stroke colour.
                        if same(&v) || (old.is_empty() && op.operator == "RG") {
                            op.operands = new_obj(&rgb);
                            changed = true;
                        }
                    }
                }
                if changed {
                    let mut ns = lopdf::Stream::new(s.dict.clone(), lopdf::content::Content { operations: ops }.encode()?);
                    ns.dict.remove(b"Filter");
                    ns.dict.remove(b"DecodeParms");
                    let _ = ns.compress();
                    let nid = doc.add_object(ns);
                    let mut ap = d.get(b"AP")?.as_dict()?.clone();
                    ap.set("N", nid);
                    d.set("AP", ap);
                }
            }
        }
    }
    // FreeText: text colour in /DA.
    if let Ok(Object::String(da, f)) = d.get(b"DA").cloned() {
        let da_s = String::from_utf8_lossy(&da).into_owned();
        let toks: Vec<&str> = da_s.split_whitespace().collect();
        if let Some(i) = toks.iter().position(|t| *t == "rg") {
            if i >= 3 {
                let v: Vec<f64> = toks[i - 3..i].iter().filter_map(|t| t.parse().ok()).collect();
                if same(&v) || old.is_empty() {
                    let mut t: Vec<String> = toks.iter().map(|s| s.to_string()).collect();
                    for k in 0..3 {
                        t[i - 3 + k] = format!("{:.3}", rgb[k]);
                    }
                    d.set("DA", Object::String(t.join(" ").into_bytes(), f));
                }
            }
        }
    }
    d.set("M", now_date());
    let new_id = doc.add_object(d);
    annots[pos] = Object::Reference(new_id);
    set_annots(doc, page, annots)?;
    Ok(new_id.0)
}
