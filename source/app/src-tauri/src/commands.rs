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
use stitchup_core::oplog::{self, OpLog, OpRecord};
use stitchup_core::ops::{
    assert_source_unmodified, commit_export as core_commit, delete_pages, extract, insert_blank,
    merge_doc, open_bytes, reorder, replace_text, rotate_pages, set_metadata as ops_set_metadata,
    unlock_document as core_unlock,
};

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
    /// True until the document's password has been supplied (A-c6). The page
    /// tree is unusable before unlock (lopdf keeps it opaque), so the session
    /// deliberately holds `doc: None` in this state.
    pub password_required: bool,
    /// Exact structural undo/redo journal (A-c8).
    pub oplog: OpLog,
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
/// `password_required` comes from the explicit session flag: lopdf's
/// `/Encrypt` trailer entry survives decryption, so `doc.is_encrypted()` can
/// NOT be used to mean "password still needed".
#[allow(clippy::too_many_arguments)]
fn payload_for(
    password_required: bool,
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
        password_required,
        warnings: warnings.to_vec(),
        bytes_b64: B64.encode(bytes),
    })
}

/// Password-gated open payload for a document the user has NOT unlocked yet.
fn password_payload(g: &SessionState) -> Result<OpenPayload, EngineError> {
    let path = g.path.clone().ok_or_else(no_doc)?;
    let hash = g.source_hash.clone().unwrap_or_default();
    let bytes = g.opened_bytes.as_ref().ok_or_else(no_doc)?.clone();
    Ok(OpenPayload {
        path: path.to_string_lossy().into_owned(),
        source_hash: hash,
        source_size: g.source_size,
        page_count: 0,
        pages: Vec::new(),
        metadata: Metadata::default(),
        encrypted: true,
        password_required: true,
        warnings: g.warnings.clone(),
        bytes_b64: B64.encode(&bytes),
    })
}

fn session_state(g: &SessionState) -> Result<(PathBuf, String, u64, Vec<String>, Vec<u8>), EngineError> {
    Ok((
        g.path.clone().ok_or_else(no_doc)?,
        g.source_hash.clone().ok_or_else(no_doc)?,
        g.source_size,
        g.warnings.clone(),
        g.opened_bytes.clone().ok_or_else(no_doc)?,
    ))
}

/// Open a document into the session. If `path` is already open, return the
/// CURRENT in-memory state (mutations included); otherwise read from disk and
/// start a fresh session.
#[tauri::command]
pub fn open_document(
    st: tauri::State<'_, Mutex<SessionState>>,
    path: String,
) -> Result<OpenPayload, EngineError> {
    open_document_inner(&st, path)
}

/// Session-logic twin of [`open_document`], callable without a Tauri app.
pub fn open_document_inner(
    st: &Mutex<SessionState>,
    path: String,
) -> Result<OpenPayload, EngineError> {
    let path_buf = PathBuf::from(&path);

    // Already open => return current in-memory state (refreshPages path).
    {
        let g = lock(&st)?;
        if g.path.as_deref() == Some(path_buf.as_path()) {
            if let (Some(doc), Some(bytes)) = (&g.doc, &g.opened_bytes) {
                let (p, h, s, w, _) = session_state(&g)?;
                return payload_for(g.password_required, p, &h, s, &w, doc, bytes);
            }
            return password_payload(&g);
        }
    }

    // Fresh open from disk.
    let bytes = std::fs::read(&path_buf)?;
    let info = open_bytes(bytes.clone(), path_buf.clone())?;

    let mut g = lock(&st)?;
    g.path = Some(path_buf.clone());
    g.source_hash = Some(info.source_hash.clone());
    g.source_size = info.source_size;
    g.warnings = info.warnings.clone();
    g.opened_bytes = Some(bytes.clone());
    g.oplog = OpLog::default();

    if info.password_required {
        // No object map yet — keep the session password-gated (A-c6).
        g.password_required = true;
        g.doc = None;
        return password_payload(&g);
    }

    let doc = PdfDoc::load_mem(&bytes).map_err(EngineError::from)?;
    g.password_required = false;
    g.doc = Some(doc);
    payload_for(
        false,
        path_buf,
        &info.source_hash,
        info.source_size,
        &info.warnings,
        g.doc.as_ref().unwrap(),
        &bytes,
    )
}

/// Run a logged structural mutation against the session. Records exact
/// before/after snapshots so undo/redo restores the full document state.
fn logged<F>(g: &mut SessionState, label: &str, f: F) -> Result<Vec<PageMeta>, EngineError>
where
    F: FnOnce(&mut PdfDoc) -> Result<(), EngineError>,
{
    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    if g.password_required {
        return Err(EngineError::new(
            ErrorCode::PasswordRequired,
            "document is still password-protected",
        ));
    }
    let before = oplog::snapshot(doc)?;
    f(doc)?;
    let after = oplog::snapshot(doc)?;
    g.oplog.push(OpRecord { label: label.into(), before, after });
    collect_pages(doc)
}

/// Run a PageTreeAdapter op against the session and return the new page list.
#[tauri::command]
pub fn page_op(
    st: tauri::State<'_, Mutex<SessionState>>,
    kind: String,
    data: Value,
) -> Result<Vec<PageMeta>, EngineError> {
    page_op_inner(&st, kind, data)
}

/// Session-logic twin of [`page_op`], callable without a Tauri app.
pub fn page_op_inner(
    st: &Mutex<SessionState>,
    kind: String,
    data: Value,
) -> Result<Vec<PageMeta>, EngineError> {
    match kind.as_str() {
        "reorder" => {
            let d: ReorderPayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            logged(&mut g, "Reorder", |doc| {
                reorder(doc, &d.new_order)?;
                Ok(())
            })
        }
        "rotate" => {
            let d: RotatePayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            logged(&mut g, "Rotate", |doc| {
                rotate_pages(doc, &d.pages, d.cw)?;
                Ok(())
            })
        }
        "delete" => {
            let d: PagesPayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            logged(&mut g, "Delete", |doc| {
                delete_pages(doc, &d.pages)?;
                Ok(())
            })
        }
        "edit_text" => {
            let d: EditTextPayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            logged(&mut g, "Edit text", |doc| {
                replace_text(
                    doc,
                    d.page,
                    d.x,
                    d.y,
                    d.width,
                    d.height,
                    d.font_size,
                    &d.new_text,
                )?;
                Ok(())
            })
        }
        "insert_blank" => {
            let d: InsertPayload = serde_json::from_value(data).map_err(validation)?;
            let mut g = lock(&st)?;
            logged(&mut g, "Insert blank page", |doc| {
                insert_blank(doc, d.after, d.width_pt, d.height_pt)?;
                Ok(())
            })
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

/// Merge another PDF's pages (in order) onto the end of the current document.
#[tauri::command]
pub fn merge_document(
    st: tauri::State<'_, Mutex<SessionState>>,
    path: String,
) -> Result<Vec<PageMeta>, EngineError> {
    merge_document_inner(&st, path)
}

/// Session-logic twin of [`merge_document`], callable without a Tauri app.
pub fn merge_document_inner(
    st: &Mutex<SessionState>,
    path: String,
) -> Result<Vec<PageMeta>, EngineError> {
    let mut g = lock(&st)?;
    let source = g.path.clone().ok_or_else(no_doc)?;
    let target = PathBuf::from(&path);
    if source == target || std::fs::canonicalize(&source).ok() == std::fs::canonicalize(&target).ok() {
        return Err(EngineError::new(
            ErrorCode::PermissionDenied,
            "cannot merge a file onto itself",
        ));
    }
    let other_bytes = std::fs::read(&target)?;
    let other = PdfDoc::load_mem(&other_bytes).map_err(EngineError::from)?;
    if other.is_encrypted() {
        return Err(EngineError::new(
            ErrorCode::Encrypted,
            "merge source is password-protected; unlock it in its own session first",
        ));
    }
    logged(&mut g, "Merge", |doc| {
        merge_doc(doc, &other)?;
        Ok(())
    })
}

/// Apply Info-dictionary updates to the session, then return the page list.
#[tauri::command]
pub fn cmd_set_metadata(
    st: tauri::State<'_, Mutex<SessionState>>,
    meta: Value,
) -> Result<Vec<PageMeta>, EngineError> {
    cmd_set_metadata_inner(&st, meta)
}

/// Session-logic twin of [`cmd_set_metadata`], callable without a Tauri app.
pub fn cmd_set_metadata_inner(
    st: &Mutex<SessionState>,
    meta: Value,
) -> Result<Vec<PageMeta>, EngineError> {
    let m: Metadata = serde_json::from_value(meta).map_err(validation)?;
    let mut g = lock(&st)?;
    logged(&mut g, "Edit metadata", |doc| {
        ops_set_metadata(doc, &m)?;
        Ok(())
    })
}

/// Undo the most recent structural mutation (exact restore, A-c8).
#[tauri::command]
pub fn undo(
    st: tauri::State<'_, Mutex<SessionState>>,
) -> Result<Vec<PageMeta>, EngineError> {
    undo_inner(&st)
}

/// Session-logic twin of [`undo`], callable without a Tauri app.
pub fn undo_inner(st: &Mutex<SessionState>) -> Result<Vec<PageMeta>, EngineError> {
    let mut g = lock(&st)?;
    let rec = g
        .oplog
        .pop_undo()
        .ok_or_else(|| EngineError::new(ErrorCode::ValidationFailed, "nothing to undo"))?;
    let pages = {
        let doc = g.doc.as_mut().ok_or_else(no_doc)?;
        oplog::apply(doc, &rec, true)?;
        collect_pages(doc)?
    };
    g.oplog.redo.push(rec);
    Ok(pages)
}

/// Redo the most recently undone structural mutation (exact replay, A-c8).
#[tauri::command]
pub fn redo(
    st: tauri::State<'_, Mutex<SessionState>>,
) -> Result<Vec<PageMeta>, EngineError> {
    redo_inner(&st)
}

/// Session-logic twin of [`redo`], callable without a Tauri app.
pub fn redo_inner(st: &Mutex<SessionState>) -> Result<Vec<PageMeta>, EngineError> {
    let mut g = lock(&st)?;
    let rec = g
        .oplog
        .pop_redo()
        .ok_or_else(|| EngineError::new(ErrorCode::ValidationFailed, "nothing to redo"))?;
    let pages = {
        let doc = g.doc.as_mut().ok_or_else(no_doc)?;
        oplog::apply(doc, &rec, false)?;
        collect_pages(doc)?
    };
    g.oplog.undo.push(rec);
    Ok(pages)
}

/// Supply the document password and (on success) load the page tree (A-c6).
/// Returns the full open payload with `password_required` cleared.
#[tauri::command]
pub fn unlock_document(
    st: tauri::State<'_, Mutex<SessionState>>,
    password: String,
) -> Result<OpenPayload, EngineError> {
    unlock_document_inner(&st, password)
}

/// Session-logic twin of [`unlock_document`], callable without a Tauri app.
pub fn unlock_document_inner(
    st: &Mutex<SessionState>,
    password: String,
) -> Result<OpenPayload, EngineError> {
    let mut g = lock(&st)?;
    if !g.password_required {
        // Nothing to do — already open (or never protected).
        if let (Some(p), Some(h), Some(bytes)) = (g.path.clone(), g.source_hash.clone(), g.opened_bytes.clone()) {
            if let Some(doc) = g.doc.as_ref() {
                return payload_for(false, p, &h, g.source_size, &g.warnings, doc, &bytes);
            }
        }
        return password_payload(&g);
    }

    let bytes = g.opened_bytes.clone().ok_or_else(no_doc)?;
    let mut doc = PdfDoc::load_mem_with_options(
        &bytes,
        lopdf::LoadOptions {
            password: Some(password.clone()),
            ..Default::default()
        },
    )
    .map_err(|e| {
        EngineError::new(ErrorCode::Encrypted, format!("cannot open with that password: {e}"))
    })?;
    if doc.is_encrypted() {
        core_unlock(&mut doc, &password).map_err(|e| {
            EngineError::new(
                e.code,
                format!("wrong password or unsupported encryption: {}", e.message),
            )
        })?;
    }
    // Verify the page tree is usable before committing the session.
    let pages = collect_pages(&doc)?;
    if pages.is_empty() {
        return Err(EngineError::new(
            ErrorCode::Encrypted,
            "password accepted but the page tree is unreadable",
        ));
    }

    g.password_required = false;
    g.doc = Some(doc);
    let (p, h, s, w, b) = session_state(&g)?;
    payload_for(false, p, &h, s, &w, g.doc.as_ref().unwrap(), &b)
}

#[tauri::command]
pub fn commit_export(
    st: tauri::State<'_, Mutex<SessionState>>,
    target: String,
    overwrite: bool,
) -> Result<stitchup_core::ops::CommitResult, EngineError> {
    commit_export_inner(&st, target, overwrite)
}

/// Session-logic twin of [`commit_export`], callable without a Tauri app.
pub fn commit_export_inner(
    st: &Mutex<SessionState>,
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
    if g.password_required {
        return Err(EngineError::new(
            ErrorCode::PermissionDenied,
            "document is password-protected; unlock before exporting",
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
    source_hash_inner(&st)
}

/// Session-logic twin of [`source_hash`], callable without a Tauri app.
pub fn source_hash_inner(st: &Mutex<SessionState>) -> Result<HashMap<String, String>, EngineError> {
    let g = lock(&st)?;
    let hash = g.source_hash.clone().ok_or_else(no_doc)?;
    Ok(HashMap::from([("source_hash".to_string(), hash)]))
}

/// Serialize the CURRENT in-memory document (mutations included) so the
/// renderer can rebind to the patched bytes after an edit or unlock.
#[tauri::command]
pub fn render_bytes(
    st: tauri::State<'_, Mutex<SessionState>>,
) -> Result<HashMap<String, String>, EngineError> {
    render_bytes_inner(&st)
}

/// Session-logic twin of [`render_bytes`], callable without a Tauri app.
pub fn render_bytes_inner(
    st: &Mutex<SessionState>,
) -> Result<HashMap<String, String>, EngineError> {
    let mut g = lock(&st)?;
    if g.password_required {
        return Err(EngineError::new(
            ErrorCode::PasswordRequired,
            "render refresh is disabled until the document is unlocked",
        ));
    }
    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    let mut data = Vec::new();
    doc.save_to(&mut data)
        .map_err(|e| EngineError::new(ErrorCode::IoError, format!("serialize: {e}")))?;
    Ok(HashMap::from([("bytes_b64".to_string(), B64.encode(&data))]))
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

#[derive(Debug, Deserialize)]
struct EditTextPayload {
    page: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    font_size: f64,
    new_text: String,
}

fn validation(e: serde_json::Error) -> EngineError {
    EngineError::new(ErrorCode::ValidationFailed, format!("payload: {e}"))
}