//! OCR text layer: lines recognised on a rendered page image (by the
//! `stitchup-ocr` helper, Apple Vision) are written into the page as
//! INVISIBLE text (render mode 3), so the page looks the same but becomes
//! searchable, selectable and copyable — like Acrobat's "searchable image".

use crate::docops::{append_to_page, page_box};
use crate::error::{EngineError, ErrorCode};
use crate::textedit::{std14_encode, std14_font};
use lopdf::content::Operation;
use lopdf::{Document as PdfDoc, Object, StringFormat};
use serde::{Deserialize, Serialize};

/// One recognised line. Box is normalised (0..1) on the image as DISPLAYED
/// (after the page's rotation), origin bottom-left.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrLine {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub confidence: f64,
}

/// Displayed (u, v) → unrotated user space, plus the text direction matrix.
fn mapper(bx: [f64; 4], rotation: i64) -> (impl Fn(f64, f64) -> (f64, f64), [f64; 4], f64, f64) {
    let (x0, y0, w, h) = (bx[0], bx[1], bx[2] - bx[0], bx[3] - bx[1]);
    let r = rotation.rem_euclid(360);
    let (dir, dw, dh) = match r {
        90 => ([0.0, 1.0, -1.0, 0.0], h, w),
        180 => ([-1.0, 0.0, 0.0, -1.0], w, h),
        270 => ([0.0, -1.0, 1.0, 0.0], h, w),
        _ => ([1.0, 0.0, 0.0, 1.0], w, h),
    };
    let f = move |u: f64, v: f64| match r {
        90 => (x0 + (1.0 - v) * w, y0 + u * h),
        180 => (x0 + (1.0 - u) * w, y0 + (1.0 - v) * h),
        270 => (x0 + v * w, y0 + (1.0 - u) * h),
        _ => (x0 + u * w, y0 + v * h),
    };
    (f, dir, dw, dh)
}

/// Add an invisible text layer to page `page_no` (1-based). `rotation` is
/// the rotation the image was rendered with. Returns how many lines were added.
pub fn add_text_layer(doc: &mut PdfDoc, page_no: u32, rotation: i64, lines: &[OcrLine]) -> Result<usize, EngineError> {
    let page = *doc
        .get_pages()
        .get(&page_no)
        .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {page_no}"))?;
    let (map, dir, dw, dh) = mapper(page_box(doc, page), rotation);
    let mut ops = vec![Operation::new("BT", vec![]), Operation::new("Tr", vec![3.into()])];
    let mut fonts = vec![];
    let mut n = 0;
    for (i, l) in lines.iter().enumerate() {
        let text = l.text.trim();
        if text.is_empty() || l.w <= 0.0 || l.h <= 0.0 {
            continue;
        }
        let enc = std14_encode(text);
        // Plain WinAnsi lines share one font; lines with extra letters get their own encoding.
        let name = if enc.diffs.is_empty() { "StOcr".to_string() } else { format!("StOcr{i}") };
        let size = (l.h * dh * 0.85).max(1.0);
        let natural: f64 = enc
            .bytes
            .iter()
            .map(|&b| crate::std14::width("Helvetica", b as u32).unwrap_or(556.0))
            .sum::<f64>()
            / 1000.0
            * size;
        let scale = if natural > 0.0 { (l.w * dw / natural * 100.0).clamp(10.0, 1000.0) } else { 100.0 };
        let (x, y) = map(l.x, l.y + l.h * 0.2);
        if !fonts.iter().any(|(k, _): &(Vec<u8>, Object)| k == name.as_bytes()) {
            fonts.push((name.clone().into_bytes(), std14_font("Helvetica", &enc.diffs)));
        }
        ops.push(Operation::new("Tf", vec![Object::Name(name.into_bytes()), Object::Real(size as f32)]));
        ops.push(Operation::new("Tz", vec![Object::Real(scale as f32)]));
        ops.push(Operation::new(
            "Tm",
            [dir[0], dir[1], dir[2], dir[3], x, y].iter().map(|v| Object::Real(*v as f32)).collect(),
        ));
        ops.push(Operation::new("Tj", vec![Object::String(enc.bytes, StringFormat::Literal)]));
        n += 1;
    }
    if n == 0 {
        return Ok(0);
    }
    ops.push(Operation::new("ET", vec![]));
    append_to_page(doc, page, ops, fonts, vec![])?;
    Ok(n)
}
