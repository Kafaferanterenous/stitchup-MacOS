//! Tauri commands — the IPC surface the frontend adapters call (ipc.ts).
//! Shapes mirror docs/STAGE0_ENGINE_INTERFACES §2/§4/§5/§6 payloads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use lopdf::Document as PdfDoc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stitchup_core::error::{EngineError, ErrorCode};
use stitchup_core::model::{collect_pages, read_metadata, Metadata, PageMeta};
use stitchup_core::ops::{assert_source_unmodified, commit_export as core_commit, delete_pages, extract, insert_blank, open_bytes, reorder, rotate_pages, set_metadata as ops_set_metadata};

/// In-memory session: the ONE document the UI is working on.
#[derive(Default)]
pub struct SessionState {
    pub path: Option<PathBuf>,
    pub source_hash: Option<String>,
    pub source_size: u64,
    pub warnings: Vec<String>,
    pub doc: Option<PdfDoc>,
    /// Opened bytes verbatim — the renderer's source of truth (never rewritten).
    pub opened_bytes: Option<Vec<u8>>,
}

/// OpenResult payload, field-for-field with the frontend `OpenResult`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct OpenPayload {
    pub path: String,
    pub source_hash: String,
    pub source_size: u64,
    pub page_count: u32,
    pub pages: Vec<PageMeta>,
    pub metadata: Metadata,
    pub encrypted: bool,
    pub password_required: bool,
    pub warnings: Vec<String>,
    pub bytes_b64: String,
}

fn lock<'a>(st: &'a Mutex<SessionState>) -> Result<MutexGuard<'a, SessionState>, EngineError> {
    st.lock()
        .map_err(|_| EngineError::new(ErrorCode::IoError, "session lock poisoned"))
}

/// Build a frontend-shaped payload from in-memory CURRENT state (mutations
/// included). `bytes` are the originally-opened bytes, kept stable across ops.
fn payload_for(
    path: PathBuf,
    hash: &str,
    size: u64,
    warnings: &[String],
    doc: &PdfDoc,
    bytes: &[u8],
) -> Result<OpenPayload, EngineError> {
    let protected = doc.is_encrypted() || doc.was_encrypted();
    let pages = collect_pages(doc)?;
    Ok(OpenPayload {
        path: path.to_string_lossy().into_owned(),
        source_hash: hash.to_string(),
        source_size: size,
        page_count: pages.len() as u32,
        pages,
        metadata: read_metadata(doc),
        encrypted: protected,
        password_required: protected,
        warnings: warnings.to_vec(),
        bytes_b64: B64.encode(bytes),
    })
}

/// Open a document into the session. If `path` is already open, return the
/// CURRENT in-memory state (mutations included); otherwise read from disk and
/// start a fresh session.
#[tauri::command]
pub fn open_document(
    st: tauri::State<'_, Mutex<SessionState>>,
    path: String,
) -> Result<OpenPayload, EngineError> {
    let path_buf = PathBuf::from(&path);

    // Already open => return current in-memory state (refreshPages path).
    {
        let g = lock(&st)?;
        if g.path.as_deref() == Some(path_buf.as_path()) {
            if let (Some(doc), Some(bytes)) = (&g.doc, &g.opened_bytes) {
                let hash = g.source_hash.as_deref().unwrap_or("");
                return payload_for(path_buf, hash, g.source_size, &g.warnings, doc, bytes);
            }
        }
    }

    // Fresh open from disk.
    let bytes = std::fs::read(&path_buf)?;
    let info = open_bytes(bytes.clone(), path_buf.clone())?;
    let doc = PdfDoc::load_mem(&bytes).map_err(EngineError::from)?;

    let mut g = lock(&st)?;
    g.path = Some(path_buf.clone());
    g.source_hash = Some(info.source_hash.clone());
    g.source_size = info.source_size;
    g.warnings = info.warnings.clone();
    g.opened_bytes = Some(bytes.clone());
    g.doc = Some(doc);
    payload_for(path_buf, &info.source_hash, info.source_size, &info.warnings, g.doc.as_ref().unwrap(), &bytes)
}

/// Run a PageTreeAdapter op against the session and return the new page list.
#[tauri::command]
pub fn page_op(
    st: tauri::State<'_, Mutex<SessionState>>,
    kind: String,
    data: Value,
) -> Result<Vec<PageMeta>, EngineError> {
    match kind.as_str() {
        "reorder" => {
            let d: ReorderPayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            let doc = g.doc.as_mut().ok_or_else(no_doc)?;
            reorder(doc, &d.new_order)
        }
        "rotate" => {
            let d: RotatePayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            let doc = g.doc.as_mut().ok_or_else(no_doc)?;
            rotate_pages(doc, &d.pages, d.cw)
        }
        "delete" => {
            let d: PagesPayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            let doc = g.doc.as_mut().ok_or_else(no_doc)?;
            delete_pages(doc, &d.pages)
        }
        "insert_blank" => {
            let d: InsertPayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            let doc = g.doc.as_mut().ok_or_else(no_doc)?;
            insert_blank(doc, d.after, d.width_pt, d.height_pt)
        }
        "extract" => {
            let d: PagesPayload = serde_json::from_value(data).map_err(validation)?;
            let g = lock(&st)?;
            let doc = g.doc.as_ref().ok_or_else(no_doc)?;
            let out = extract(doc, &d.pages)?;
            collect_pages(&out)
        }
        _ => Err(EngineError::new(
            ErrorCode::ValidationFailed,
            format!("unknown op kind {kind:?}"),
        )),
    }
}

/// Apply Info-dictionary updates to the session, then return the page list.
#[tauri::command]
pub fn cmd_set_metadata(
    st: tauri::State<'_, Mutex<SessionState>>,
    meta: Value,
) -> Result<Vec<PageMeta>, EngineError> {
    let m: Metadata = serde_json::from_value(meta).map_err(validation)?;
    let mut g = lock(&st)?;
    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    ops_set_metadata(doc, &m)?;
    collect_pages(doc)
}

#[tauri::command]
pub fn commit_export(
    st: tauri::State<'_, Mutex<SessionState>>,
    target: String,
    overwrite: bool,
) -> Result<stitchup_core::ops::CommitResult, EngineError> {
    let (mut g, hash, source) = {
        let g = lock(&st)?;
        let hash = g.source_hash.clone().ok_or_else(no_doc)?;
        let source = g.path.clone().ok_or_else(no_doc)?;
        (g, hash, source)
    };
    let target_path = PathBuf::from(&target);

    // Data-safety rule: never save over the opened source file.
    let same = source == target_path
        || std::fs::canonicalize(&source)
            .map(|a| std::fs::canonicalize(&target_path).map(|b| a == b).unwrap_or(false))
            .unwrap_or(false);
    if same {
        return Err(EngineError::new(
            ErrorCode::PermissionDenied,
            "attempted to save over the opened source file",
        ));
    }
    if !overwrite && target_path.exists() {
        return Err(EngineError::new(
            ErrorCode::PermissionDenied,
            "target file exists; enable overwrite to replace it",
        ));
    }

    // Source-shift guard: refuse to derive an export from a drifted source.
    assert_source_unmodified(&hash, &source)?;

    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    core_commit(doc, &target_path)
}

#[tauri::command]
pub fn source_hash(
    st: tauri::State<'_, Mutex<SessionState>>,
) -> Result<HashMap<String, String>, EngineError> {
    let g = lock(&st)?;
    let hash = g.source_hash.clone().ok_or_else(no_doc)?;
    Ok(HashMap::from([("source_hash".to_string(), hash)]))
}

fn no_doc() -> EngineError {
    EngineError::new(ErrorCode::OpenFailed, "no document open")
}

// ---------------------------------------------------------------------------
// Wire payloads (snake_case, mirror ipc.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ReorderPayload {
    new_order: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct RotatePayload {
    pages: Vec<u32>,
    cw: bool,
}

#[derive(Debug, Deserialize)]
struct PagesPayload {
    pages: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct InsertPayload {
    after: u32,
    width_pt: f64,
    height_pt: f64,
}

fn validation(e: serde_json::Error) -> EngineError {
    EngineError::new(ErrorCode::ValidationFailed, format!("payload: {e}"))
}