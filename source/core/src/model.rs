//! Slim document model — mirrors STAGE0_DOCUMENT_MODEL §2/§3 for the structural core.
//! We keep a `lopdf::Document` handle for structural work but expose only model
//! values to callers.

use crate::error::EngineError;
use lopdf::{Document as PdfDoc, Object};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageMeta {
    pub page_no: u32,
    pub width_pt: f64,
    pub height_pt: f64,
    pub rotation: i64,
    pub has_annotations: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenInfo {
    pub path: std::path::PathBuf,
    pub source_hash: String,
    pub source_size: u64,
    pub page_count: u32,
    pub pages: Vec<PageMeta>,
    pub metadata: Metadata,
    pub encrypted: bool,
    pub password_required: bool,
    pub warnings: Vec<String>,
}

fn box_arr(page: &Object) -> Option<&Object> {
    let dict = page.as_dict().ok()?;
    dict.get(b"MediaBox").ok().or_else(|| dict.get(b"CropBox").ok())
}

/// MediaBox / CropBox -> (width, height) in points.
pub fn page_box(page: &Object) -> Option<(f64, f64)> {
    let arr = box_arr(page)?.as_array().ok()?;
    if arr.len() < 4 {
        return None;
    }
    let x = |i: usize| arr[i].as_float().ok().map(|v| v as f64);
    let (llx, lly, urx, ury) = (x(0)?, x(1)?, x(2)?, x(3)?);
    let w = (urx - llx).abs();
    let h = (ury - lly).abs();
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some((w, h))
}

pub fn page_rotation(page: &Object) -> i64 {
    page.as_dict()
        .ok()
        .and_then(|d| d.get(b"Rotate").ok())
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        % 360
}

pub fn page_has_annotations(page: &Object) -> bool {
    page.as_dict()
        .ok()
        .and_then(|d| d.get(b"Annots").ok())
        .map(|o| !o.is_null())
        .unwrap_or(false)
}

fn metadata_string(info: &Object, key: &[u8]) -> Option<String> {
    let obj = info.as_dict().ok()?.get(key).ok()?;
    match obj {
        Object::String(s, _) => Some(String::from_utf8_lossy(s).into_owned()),
        _ => None,
    }
}

pub fn read_metadata(doc: &PdfDoc) -> Metadata {
    let mut meta = Metadata::default();
    if let Ok(info_ref) = doc.trailer.get(b"Info") {
        if let Ok(id) = info_ref.as_reference() {
            if let Ok(info_obj) = doc.get_object(id) {
                meta.title = metadata_string(info_obj, b"Title");
                meta.author = metadata_string(info_obj, b"Author");
                meta.subject = metadata_string(info_obj, b"Subject");
                meta.keywords = metadata_string(info_obj, b"Keywords");
                meta.creator = metadata_string(info_obj, b"Creator");
                meta.producer = metadata_string(info_obj, b"Producer");
            }
        }
    }
    meta
}

/// Collect page metadata in original order using lopdf's page iteration.
/// `get_pages()` maps 1-based page number -> page ObjectId.
pub fn collect_pages(doc: &PdfDoc) -> Result<Vec<PageMeta>, EngineError> {
    let pages = doc.get_pages();
    let mut out = Vec::with_capacity(pages.len());
    for (page_no, id) in pages {
        let page = doc.get_object(id)?;
        let (w, h) = page_box(page)
            .ok_or_else(|| crate::err!(crate::ErrorCode::Malformed, "page {page_no} has no usable box"))?;
        out.push(PageMeta {
            page_no,
            width_pt: w,
            height_pt: h,
            rotation: page_rotation(page),
            has_annotations: page_has_annotations(page),
        });
    }
    if out.is_empty() {
        return Err(crate::err!(crate::ErrorCode::Malformed, "document has no pages"));
    }
    Ok(out)
}

pub fn build_open_info(
    doc: &PdfDoc,
    path: std::path::PathBuf,
    source_hash: String,
    source_size: u64,
    warnings: Vec<String>,
) -> Result<OpenInfo, EngineError> {
    let pages = collect_pages(doc)?;
    let page_count = pages.iter().map(|p| p.page_no).max().unwrap_or(0);
    // lopdf is lazy WRT decryption: `is_encrypted()` stays true for a
    // password-free load (trailer /Encrypt present); `was_encrypted()` stays
    // true even after a successful password load strips it. Either => the file
    // is (or was) protected, so the UI must have authenticated to decrypt
    // streams; that is the renderer's concern, but flag it either way.
    let protected = doc.is_encrypted() || doc.was_encrypted();
    Ok(OpenInfo {
        path,
        source_hash,
        source_size,
        page_count,
        encrypted: protected,
        password_required: protected,
        metadata: read_metadata(doc),
        pages,
        warnings,
    })
}