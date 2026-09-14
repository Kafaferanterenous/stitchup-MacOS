//! Structural operations + atomic save-as — mirrors the PageTreeAdapter +
//! CommitAdapter contracts (STAGE0_ENGINE_INTERFACES §4/§6) using D-05 lopdf.

use std::io::Write;
use std::path::{Path, PathBuf};

use lopdf::{dictionary, Document as PdfDoc, Object, Stream};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, ErrorCode};
use crate::hash::sha256_hex;
use crate::model::{build_open_info, Metadata, OpenInfo, PageMeta};

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

pub fn open_bytes(bytes: Vec<u8>, path: PathBuf) -> Result<OpenInfo, EngineError> {
    let source_hash = sha256_hex(&bytes);
    let source_size = bytes.len() as u64;
    let mut warnings = Vec::new();
    let doc =
        PdfDoc::load_mem(&bytes).map_err(|e| classify_open_error(&e.to_string(), bytes.len()))?;
    if doc.objects.is_empty() {
        return Err(crate::err!(ErrorCode::Malformed, "empty object map after parse"));
    }
    // Renderer-side flags we can infer cheaply: XFA, JS, embedded files.
    if let Ok(cat_ref) = doc.trailer.get(b"Root") {
        if let Ok(id) = cat_ref.as_reference() {
            if let Ok(cat) = doc.get_object(id) {
                if let Ok(d) = cat.as_dict() {
                    if d.get(b"AcroForm")
                        .ok()
                        .and_then(|o| o.as_dict().ok())
                        .and_then(|f| f.get(b"XFA").ok())
                        .is_some()
                    {
                        warnings.push("document contains an XFA form (view disabled, read-only)".into());
                    }
                    if d.get(b"Names").is_ok() {
                        warnings.push("document declares EmbeddedFiles/Names (kept inert)".into());
                    }
                }
            }
        }
    }
    build_open_info(&doc, path, source_hash, source_size, warnings)
}

fn classify_open_error(msg: &str, _size: usize) -> EngineError {
    let lower = msg.to_lowercase();
    let mut code = ErrorCode::Malformed;
    if lower.contains("password") || lower.contains("encrypt") || lower.contains("decrypt") {
        code = ErrorCode::Encrypted;
    }
    EngineError::new(code, format!("parse failed: {msg}"))
}

pub fn open_path(path: &Path) -> Result<OpenInfo, EngineError> {
    let bytes = std::fs::read(path)?;
    open_bytes(bytes, path.to_path_buf())
}

// ---------------------------------------------------------------------------
// Page tree helpers
// ---------------------------------------------------------------------------

fn catalog_pages_id(doc: &PdfDoc) -> Result<(u32, u16), EngineError> {
    let root_ref = doc.trailer.get(b"Root")?;
    let root_id = root_ref.as_reference()?;
    let catalog = doc.get_object(root_id)?;
    let pages_ref = catalog.as_dict()?.get(b"Pages")?;
    pages_ref.as_reference().map_err(Into::into)
}

/// Returns (pages_node_id, kid ids) for the single declarative root Pages node.
fn pages_node(doc: &PdfDoc) -> Result<(u32, Vec<(u32, u16)>), EngineError> {
    let pages_id = catalog_pages_id(doc)?;
    let node = doc.get_object(pages_id)?;
    let kids_ref = node.as_dict()?.get(b"Kids")?;
    let kids_arr = kids_ref.as_array()?;
    let kids = kids_arr
        .iter()
        .map(|o| o.as_reference().map_err(|e| crate::err!(ErrorCode::Malformed, "kid is not a reference: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((pages_id.0, kids))
}

fn set_kids(doc: &mut PdfDoc, pages_id: u32, kids: &[(u32, u16)], count: u32) -> Result<(), EngineError> {
    let node_id = (pages_id, 0);
    let node = doc
        .objects
        .get_mut(&node_id)
        .ok_or_else(|| crate::err!(ErrorCode::Malformed, "pages node missing"))?;
    let dict = node.as_dict_mut()?;
    dict.set("Kids", Object::Array(kids.iter().map(|&r| Object::Reference(r)).collect()));
    dict.set("Count", Object::Integer(count as i64));
    Ok(())
}

fn pages_struct(doc: &PdfDoc) -> Result<(u32, Vec<(u32, u16)>), EngineError> {
    pages_node(doc)
}

// ---------------------------------------------------------------------------
// Page operations
// ---------------------------------------------------------------------------

/// Reorder pages so that `new_order` (1-based page numbers, each exactly once)
/// is the resulting order.
pub fn reorder(doc: &mut PdfDoc, new_order: &[u32]) -> Result<Vec<PageMeta>, EngineError> {
    let (pages_id, kids) = pages_struct(doc)?;
    let pages_map = doc.get_pages();
    if new_order.len() != kids.len() {
        return Err(crate::err!(
            ErrorCode::ValidationFailed,
            "reorder needs {} pages, got {}",
            kids.len(),
            new_order.len()
        ));
    }
    let mut used = std::collections::HashSet::new();
    for &n in new_order {
        if n < 1 || n as usize > kids.len() || !used.insert(n) {
            return Err(crate::err!(ErrorCode::ValidationFailed, "bad reorder list"));
        }
    }
    let mut new_kids = Vec::with_capacity(kids.len());
    for &n in new_order {
        let k = pages_map.get(&n).copied();
        if let Some(k) = k {
            new_kids.push(k);
        } else {
            new_kids.push(kids[(n - 1) as usize]);
        }
    }
    set_kids(doc, pages_id, &new_kids, new_kids.len() as u32)?;
    crate::model::collect_pages(doc)
}

/// Rotate pages 90 deg clockwise (or counter-clockwise); /Rotate stays 0..<360.
pub fn rotate_pages(doc: &mut PdfDoc, page_nos: &[u32], cw: bool) -> Result<Vec<PageMeta>, EngineError> {
    let pages_map = doc.get_pages();
    for &n in page_nos {
        let &id = pages_map
            .get(&n)
            .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {n}"))?;
        let page = doc.get_object(id)?;
        let cur = crate::model::page_rotation(page);
        let delta = if cw { 90 } else { -90 };
        let next = (cur + delta).rem_euclid(360);
        if let Some(d) = doc.objects.get_mut(&id).and_then(|o| o.as_dict_mut().ok()) {
            d.set("Rotate", Object::Integer(next));
        }
    }
    crate::model::collect_pages(doc)
}

/// Delete pages by 1-based numbers (refuses the last page).
pub fn delete_pages(doc: &mut PdfDoc, page_nos: &[u32]) -> Result<Vec<PageMeta>, EngineError> {
    let (pages_id, kids) = pages_struct(doc)?;
    let del: std::collections::HashSet<u32> = page_nos.iter().copied().collect();
    let keep: Vec<(u32, u16)> = kids
        .iter()
        .enumerate()
        .filter(|(i, _)| !del.contains(&((*i + 1) as u32)))
        .map(|(_, k)| *k)
        .collect();
    if keep.is_empty() {
        return Err(crate::err!(
            ErrorCode::ValidationFailed,
            "refusing to delete all pages"
        ));
    }
    set_kids(doc, pages_id, &keep, keep.len() as u32)?;
    crate::model::collect_pages(doc)
}

/// Insert a blank page (default letter size) after 1-based `after` (0 = before page 1).
pub fn insert_blank(doc: &mut PdfDoc, after: u32, width_pt: f64, height_pt: f64) -> Result<Vec<PageMeta>, EngineError> {
    let (pages_id, kids) = pages_struct(doc)?;
    if after as usize > kids.len() {
        return Err(crate::err!(ErrorCode::ValidationFailed, "after {} out of range", after));
    }
    let content_id = doc.add_object(Stream::new(dictionary! {}, Vec::<u8>::new()));
    let page_id = doc.add_object(dictionary! {
        "Type" => Object::Name("Page".into()),
        "Parent" => Object::Reference((pages_id, 0)),
        "Contents" => Object::Reference(content_id),
        "Resources" => Object::Dictionary(lopdf::Dictionary::new()),
        "MediaBox" => Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(width_pt as f32),
            Object::Real(height_pt as f32),
        ]),
    });
    let mut kids = kids;
    kids.insert(after as usize, page_id);
    set_kids(doc, pages_id, &kids, kids.len() as u32)?;
    crate::model::collect_pages(doc)
}

/// Extract pages (1-based) into a fresh in-memory document (orphan-safe copy).
pub fn extract(doc: &PdfDoc, page_nos: &[u32]) -> Result<PdfDoc, EngineError> {
    let mut out = PdfDoc::with_version(doc.version.as_str());
    out.trailer = doc.trailer.clone();
    for (old_id, obj) in doc.objects.iter() {
        out.objects.insert(*old_id, obj.clone());
    }
    let pages_map = doc.get_pages();
    let mut kids: Vec<(u32, u16)> = Vec::new();
    for &n in page_nos {
        let &id = pages_map
            .get(&n)
            .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {n}"))?;
        kids.push(id);
    }
    let pages_id = catalog_pages_id(&out)?;
    for (id, _gen) in kids.iter() {
        if let Some(obj) = out.objects.get_mut(&(*id, 0)) {
            if let Ok(d) = obj.as_dict_mut() {
                d.set("Parent", Object::Reference(pages_id));
            }
        }
    }
    let node = out
        .objects
        .get_mut(&pages_id)
        .ok_or_else(|| crate::err!(ErrorCode::Malformed, "pages node missing"))?;
    let dict = node.as_dict_mut()?;
    dict.set("Kids", Object::Array(kids.iter().map(|&r| Object::Reference(r)).collect()));
    dict.set("Count", Object::Integer(kids.len() as i64));
    Ok(out)
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

pub fn set_metadata(doc: &mut PdfDoc, meta: &Metadata) -> Result<(), EngineError> {
    let info_id = match doc.trailer.get(b"Info") {
        Ok(id) => (*id).as_reference().map_err(|e| crate::err!(ErrorCode::Malformed, "info ref: {e}"))?,
        Err(_) => {
            let id = doc.add_object(Object::Dictionary(lopdf::Dictionary::new()));
            doc.trailer.set("Info", Object::Reference(id));
            id
        }
    };
    let obj = doc
        .objects
        .get_mut(&info_id)
        .ok_or_else(|| crate::err!(ErrorCode::Malformed, "info obj missing"))?;
    let dict = obj.as_dict_mut()?;
    macro_rules! set_opt {
        ($key:literal, $field:expr) => {
            if let Some(s) = $field {
                dict.set($key, Object::string_literal(s.clone()));
            }
        };
    }
    set_opt!("Title", &meta.title);
    set_opt!("Author", &meta.author);
    set_opt!("Subject", &meta.subject);
    set_opt!("Keywords", &meta.keywords);
    set_opt!("Creator", &meta.creator);
    set_opt!("Producer", &meta.producer);
    Ok(())
}

// ---------------------------------------------------------------------------
// Commit / export (atomic save-as)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitResult {
    pub new_sha256: String,
    pub size_bytes: u64,
    pub published_path: PathBuf,
}

/// Serialize through lopdf to the target dir as a temp file, fsync, then rename
/// (same volume) and hash the published bytes. Callers must not pass a source path.
pub fn commit_export(doc: &mut PdfDoc, target: &Path) -> Result<CommitResult, EngineError> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| crate::err!(ErrorCode::IoError, "target has no parent dir"))?;
    std::fs::create_dir_all(dir)?;
    let tmp_path = tmp_path_for(target)?;
    {
        let mut data = Vec::new();
        doc.save_to(&mut data)?;
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(&data)?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp_path, target) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(EngineError::new(ErrorCode::AtomicPublishFailed, e.to_string()));
    }
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    let bytes = std::fs::read(target)?;
    Ok(CommitResult {
        new_sha256: sha256_hex(&bytes),
        size_bytes: bytes.len() as u64,
        published_path: target.to_path_buf(),
    })
}

fn tmp_path_for(target: &Path) -> Result<PathBuf, EngineError> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("stitchup_export.pdf");
    let pid = std::process::id();
    Ok(dir.join(format!(".stitchup-tmp-{pid}-{name}")))
}

/// Guard: refuse to commit over a source file whose on-disk hash drifted from
/// the hash captured at open time.
pub fn assert_source_unmodified(source_hash: &str, source_path: &Path) -> Result<(), EngineError> {
    let bytes = std::fs::read(source_path)?;
    let now = sha256_hex(&bytes);
    if now != source_hash {
        return Err(crate::err!(
            ErrorCode::PermissionDenied,
            "source file changed on disk during session; refusing to overwrite anything"
        ));
    }
    Ok(())
}