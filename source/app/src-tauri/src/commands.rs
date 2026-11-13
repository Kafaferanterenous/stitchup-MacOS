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
    assert_source_unmodified, commit_export as core_commit, delete_pages, extract,
    flatten_page_tree, insert_blank, merge_doc, open_bytes, reorder, replace_text, rotate_pages,
    set_metadata as ops_set_metadata, split, unlock_document as core_unlock, CommitResult,
};
use stitchup_core::textedit::{self, EditOutcome, EditRequest};
use stitchup_core::docops;

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
    /// A New document that has never been saved (Save acts as Save As).
    pub untitled: bool,
    /// Password the document was unlocked with: Save in place re-encrypts with
    /// it, so a protected file is never silently written back unprotected.
    pub password: Option<String>,
    /// `oplog.undo.len()` at the last save (unsaved = differs).
    pub saved_ops: usize,
}

impl SessionState {
    pub fn dirty(&self) -> bool {
        self.doc.is_some() && (self.untitled && !self.oplog.undo.is_empty() || self.oplog.undo.len() != self.saved_ops)
    }
}

/// Virtual path prefix for never-saved documents.
pub const UNTITLED_PREFIX: &str = "stitchup-untitled:";

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
    g.untitled = false;
    g.password = None;

    if info.password_required {
        // No object map yet — keep the session password-gated (A-c6).
        g.password_required = true;
        g.doc = None;
        return password_payload(&g);
    }

    let mut doc = PdfDoc::load_mem(&bytes).map_err(EngineError::from)?;
    // Nested page trees are flattened up front (outside the op log) so every
    // structural op and its undo snapshot work on a flat /Kids list.
    let _ = flatten_page_tree(&mut doc);
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
    if g.password_required {
        return Err(EngineError::new(
            ErrorCode::PasswordRequired,
            "document is still password-protected",
        ));
    }
    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    let before = oplog::snapshot(doc)?;
    f(doc)?;
    let after = oplog::snapshot(doc)?;
    g.oplog.push(OpRecord { label: label.into(), before, after });
    collect_pages(doc)
}

/// Like [`logged`], but also snapshots extra objects the op will modify
/// in place (form field dictionaries, the catalog…), so undo restores them.
fn logged_objects<F>(g: &mut SessionState, label: &str, ids: &[(u32, u16)], f: F) -> Result<Vec<PageMeta>, EngineError>
where
    F: FnOnce(&mut PdfDoc) -> Result<(), EngineError>,
{
    if g.password_required {
        return Err(EngineError::new(ErrorCode::PasswordRequired, "document is still password-protected"));
    }
    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    let before = oplog::with_extra(oplog::snapshot(doc)?, doc, ids);
    f(doc)?;
    let after = oplog::with_extra(oplog::snapshot(doc)?, doc, ids);
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
    // Decrypted in memory: drop /Encrypt so serialized bytes (render refresh,
    // save-as) are a valid UNPROTECTED PDF instead of plaintext objects behind
    // an encryption dictionary (which no reader can open).
    doc.trailer.remove(b"Encrypt");
    let _ = flatten_page_tree(&mut doc);
    // Verify the page tree is usable before committing the session.
    let pages = collect_pages(&doc)?;
    if pages.is_empty() {
        return Err(EngineError::new(
            ErrorCode::Encrypted,
            "password accepted but the page tree is unreadable",
        ));
    }

    g.password_required = false;
    g.password = Some(password);
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
    if !g.untitled {
        assert_source_unmodified(&hash, &source)?;
    }

    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    core_commit(doc, &target_path)
}

/// Result of a live text edit: the new page list plus how the edit was done
/// (in the original font, replaced with a standard font, or overlay fallback).
#[derive(Debug, Clone, Serialize)]
pub struct EditResult {
    pub pages: Vec<PageMeta>,
    #[serde(flatten)]
    pub outcome: EditOutcome,
}

/// In-place text edit (see `stitchup_core::textedit`). Logged for undo/redo.
#[tauri::command]
pub fn edit_text(
    st: tauri::State<'_, Mutex<SessionState>>,
    data: Value,
) -> Result<EditResult, EngineError> {
    edit_text_inner(&st, data)
}

/// Session-logic twin of [`edit_text`], callable without a Tauri app.
pub fn edit_text_inner(st: &Mutex<SessionState>, data: Value) -> Result<EditResult, EngineError> {
    let req: EditRequest = serde_json::from_value(data).map_err(validation)?;
    let mut g = lock(st)?;
    let mut outcome = None;
    let pages = logged(&mut g, "Edit text", |doc| {
        outcome = Some(textedit::edit_text(doc, &req)?);
        Ok(())
    })?;
    Ok(EditResult { pages, outcome: outcome.expect("edit ran") })
}

/// Refuse any export target that is the opened source file.
fn guard_target(g: &SessionState, target: &std::path::Path, overwrite: bool) -> Result<(), EngineError> {
    let source = g.path.clone().ok_or_else(no_doc)?;
    let same = source == target
        || std::fs::canonicalize(&source)
            .map(|a| std::fs::canonicalize(target).map(|b| a == b).unwrap_or(false))
            .unwrap_or(false);
    if same {
        return Err(EngineError::new(ErrorCode::PermissionDenied, "attempted to save over the opened source file"));
    }
    if !overwrite && target.exists() {
        return Err(EngineError::new(
            ErrorCode::PermissionDenied,
            format!("{} already exists", target.display()),
        ));
    }
    if g.password_required {
        return Err(EngineError::new(ErrorCode::PermissionDenied, "unlock the document first"));
    }
    Ok(())
}

/// Save the given pages (1-based, in the given order) as a NEW PDF (atomic).
#[tauri::command]
pub fn export_pages(
    st: tauri::State<'_, Mutex<SessionState>>,
    pages: Vec<u32>,
    target: String,
    overwrite: bool,
) -> Result<CommitResult, EngineError> {
    export_pages_inner(&st, pages, target, overwrite)
}

pub fn export_pages_inner(
    st: &Mutex<SessionState>,
    pages: Vec<u32>,
    target: String,
    overwrite: bool,
) -> Result<CommitResult, EngineError> {
    let g = lock(st)?;
    let target = PathBuf::from(target);
    guard_target(&g, &target, overwrite)?;
    let doc = g.doc.as_ref().ok_or_else(no_doc)?;
    let mut out = extract(doc, &pages)?;
    core_commit(&mut out, &target)
}

/// Split the document into files of `every` pages: `<dir>/<stem>-part<N>.pdf`.
#[tauri::command]
pub fn split_document(
    st: tauri::State<'_, Mutex<SessionState>>,
    every: u32,
    dir: String,
    stem: String,
    overwrite: bool,
) -> Result<Vec<CommitResult>, EngineError> {
    split_document_inner(&st, every, dir, stem, overwrite)
}

pub fn split_document_inner(
    st: &Mutex<SessionState>,
    every: u32,
    dir: String,
    stem: String,
    overwrite: bool,
) -> Result<Vec<CommitResult>, EngineError> {
    let g = lock(st)?;
    let doc = g.doc.as_ref().ok_or_else(no_doc)?;
    let parts = split(doc, every)?;
    let width = parts.len().to_string().len();
    let targets: Vec<PathBuf> = (1..=parts.len())
        .map(|i| PathBuf::from(&dir).join(format!("{stem}-part{i:0width$}.pdf")))
        .collect();
    // Check every target before writing any, so a clash never leaves half a split.
    for t in &targets {
        guard_target(&g, t, overwrite)?;
    }
    parts
        .into_iter()
        .zip(targets)
        .map(|(mut d, t)| core_commit(&mut d, &t))
        .collect()
}

// ---------------------------------------------------------------------------
// Document-level commands (New, Save, Watermark, Crop, Encrypt, Compress,
// Attachments, file export, print)
// ---------------------------------------------------------------------------

static UNTITLED_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// Start a new blank one-page document (Letter or A4 etc.).
#[tauri::command]
pub fn new_document(
    st: tauri::State<'_, Mutex<SessionState>>,
    width_pt: f64,
    height_pt: f64,
) -> Result<OpenPayload, EngineError> {
    new_document_inner(&st, width_pt, height_pt)
}

pub fn new_document_inner(st: &Mutex<SessionState>, width_pt: f64, height_pt: f64) -> Result<OpenPayload, EngineError> {
    let mut doc = docops::new_blank(width_pt, height_pt)?;
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))?;
    let n = UNTITLED_N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = PathBuf::from(format!("{UNTITLED_PREFIX}{n}"));
    let hash = stitchup_core::hash::sha256_hex(&bytes);
    let mut g = lock(st)?;
    *g = SessionState {
        path: Some(path.clone()),
        source_hash: Some(hash.clone()),
        source_size: bytes.len() as u64,
        warnings: Vec::new(),
        doc: Some(doc),
        opened_bytes: Some(bytes.clone()),
        password_required: false,
        oplog: OpLog::default(),
        untitled: true,
        password: None,
        saved_ops: 0,
    };
    payload_for(false, path, &hash, bytes.len() as u64, &[], g.doc.as_ref().unwrap(), &bytes)
}

/// Combine several PDFs (in order) into a NEW untitled document.
#[tauri::command]
pub fn combine_documents(st: tauri::State<'_, Mutex<SessionState>>, paths: Vec<String>) -> Result<OpenPayload, EngineError> {
    combine_documents_inner(&st, paths)
}

pub fn combine_documents_inner(st: &Mutex<SessionState>, paths: Vec<String>) -> Result<OpenPayload, EngineError> {
    let mut iter = paths.iter();
    let first = iter.next().ok_or_else(|| EngineError::new(ErrorCode::ValidationFailed, "choose at least one PDF"))?;
    let load = |p: &String| -> Result<PdfDoc, EngineError> {
        let doc = PdfDoc::load_mem(&std::fs::read(p)?).map_err(EngineError::from)?;
        if doc.is_encrypted() {
            return Err(EngineError::new(ErrorCode::Encrypted, format!("{p} is password-protected")));
        }
        Ok(doc)
    };
    let mut doc = load(first)?;
    flatten_page_tree(&mut doc)?;
    for p in iter {
        merge_doc(&mut doc, &load(p)?)?;
    }
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))?;
    let n = UNTITLED_N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = PathBuf::from(format!("{UNTITLED_PREFIX}{n}"));
    let hash = stitchup_core::hash::sha256_hex(&bytes);
    let mut g = lock(st)?;
    *g = SessionState {
        path: Some(path.clone()),
        source_hash: Some(hash.clone()),
        source_size: bytes.len() as u64,
        doc: Some(doc),
        opened_bytes: Some(bytes.clone()),
        untitled: true,
        ..Default::default()
    };
    payload_for(false, path, &hash, bytes.len() as u64, &[], g.doc.as_ref().unwrap(), &bytes)
}

/// After Save As: the saved file becomes the document (later Saves go there).
#[tauri::command]
pub fn adopt_saved_path(st: tauri::State<'_, Mutex<SessionState>>, path: String) -> Result<OpenPayload, EngineError> {
    adopt_saved_path_inner(&st, path)
}

pub fn adopt_saved_path_inner(st: &Mutex<SessionState>, path: String) -> Result<OpenPayload, EngineError> {
    let mut g = lock(st)?;
    let target = PathBuf::from(&path);
    let bytes = std::fs::read(&target)?;
    g.path = Some(target.clone());
    g.source_hash = Some(stitchup_core::hash::sha256_hex(&bytes));
    g.source_size = bytes.len() as u64;
    g.untitled = false;
    g.saved_ops = g.oplog.undo.len();
    // Save As writes an unprotected copy, so the adopted file has no password.
    g.password = None;
    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    let mut render = Vec::new();
    doc.save_to(&mut render).map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))?;
    let (h, s, w) = (g.source_hash.clone().unwrap(), g.source_size, g.warnings.clone());
    g.opened_bytes = Some(render.clone());
    payload_for(false, target, &h, s, &w, g.doc.as_ref().unwrap(), &render)
}

/// Paths the OS asked us to open (Finder double-click / Dock drop / argv)
/// before the window was ready; the frontend drains them at start-up.
pub static PENDING_OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[tauri::command]
pub fn take_pending_open() -> Vec<String> {
    PENDING_OPEN.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

/// Save over the opened file (atomic temp + rename). Untitled documents must
/// use Save As; a file changed on disk since it was opened is refused.
#[tauri::command]
pub fn save_document(st: tauri::State<'_, Mutex<SessionState>>) -> Result<CommitResult, EngineError> {
    save_document_inner(&st)
}

pub fn save_document_inner(st: &Mutex<SessionState>) -> Result<CommitResult, EngineError> {
    let mut g = lock(st)?;
    if g.untitled {
        return Err(EngineError::new(ErrorCode::ValidationFailed, "this document has not been saved yet — use Save As"));
    }
    if g.password_required {
        return Err(EngineError::new(ErrorCode::PermissionDenied, "unlock the document first"));
    }
    let path = g.path.clone().ok_or_else(no_doc)?;
    let hash = g.source_hash.clone().ok_or_else(no_doc)?;
    assert_source_unmodified(&hash, &path)?;
    let password = g.password.clone();
    let doc = g.doc.as_mut().ok_or_else(no_doc)?;
    let result = match &password {
        Some(pw) => core_commit(&mut docops::encrypted_copy(doc, pw, pw)?, &path)?,
        None => core_commit(doc, &path)?,
    };
    // The saved file is the new baseline (source-shift guard + renderer bytes).
    let mut render = Vec::new();
    doc.save_to(&mut render).map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))?;
    g.source_hash = Some(result.new_sha256.clone());
    g.source_size = result.size_bytes;
    g.opened_bytes = Some(render);
    g.saved_ops = g.oplog.undo.len();
    Ok(result)
}

/// Draw a text watermark on the pages (logged; undoable).
#[tauri::command]
pub fn add_watermark(st: tauri::State<'_, Mutex<SessionState>>, spec: Value) -> Result<Vec<PageMeta>, EngineError> {
    add_watermark_inner(&st, spec)
}

pub fn add_watermark_inner(st: &Mutex<SessionState>, spec: Value) -> Result<Vec<PageMeta>, EngineError> {
    let spec: docops::WatermarkSpec = serde_json::from_value(spec).map_err(validation)?;
    let mut g = lock(st)?;
    logged(&mut g, "Watermark", |doc| docops::watermark(doc, &spec).map(|_| ()))
}

/// Set crop boxes: `[[page, x0, y0, x1, y1], ...]` in PDF user space (logged).
#[tauri::command]
pub fn set_crop(st: tauri::State<'_, Mutex<SessionState>>, boxes: Vec<[f64; 5]>) -> Result<Vec<PageMeta>, EngineError> {
    set_crop_inner(&st, boxes)
}

pub fn set_crop_inner(st: &Mutex<SessionState>, boxes: Vec<[f64; 5]>) -> Result<Vec<PageMeta>, EngineError> {
    let mut g = lock(st)?;
    logged(&mut g, "Crop", |doc| {
        for b in &boxes {
            docops::set_crop_box(doc, b[0] as u32, [b[1], b[2], b[3], b[4]])?;
        }
        Ok(())
    })
}

/// Save an AES-256 password-protected copy; the open document is unchanged.
#[tauri::command]
pub fn save_encrypted(
    st: tauri::State<'_, Mutex<SessionState>>,
    target: String,
    user_password: String,
    owner_password: String,
) -> Result<CommitResult, EngineError> {
    save_encrypted_inner(&st, target, user_password, owner_password)
}

pub fn save_encrypted_inner(
    st: &Mutex<SessionState>,
    target: String,
    user_password: String,
    owner_password: String,
) -> Result<CommitResult, EngineError> {
    let g = lock(st)?;
    let target = PathBuf::from(target);
    guard_target(&g, &target, true)?;
    let doc = g.doc.as_ref().ok_or_else(no_doc)?;
    core_commit(&mut docops::encrypted_copy(doc, &user_password, &owner_password)?, &target)
}

/// Save a compressed copy; reports the size before and after.
#[tauri::command]
pub fn save_compressed(st: tauri::State<'_, Mutex<SessionState>>, target: String) -> Result<Value, EngineError> {
    save_compressed_inner(&st, target)
}

pub fn save_compressed_inner(st: &Mutex<SessionState>, target: String) -> Result<Value, EngineError> {
    let g = lock(st)?;
    let target = PathBuf::from(target);
    guard_target(&g, &target, true)?;
    let doc = g.doc.as_ref().ok_or_else(no_doc)?;
    let mut plain = Vec::new();
    doc.clone().save_to(&mut plain).map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))?;
    let bytes = docops::compressed_bytes(doc)?;
    let res = write_atomic(&target, &bytes)?;
    Ok(serde_json::json!({ "before": plain.len(), "after": bytes.len(), "result": res }))
}

#[tauri::command]
pub fn list_attachments(st: tauri::State<'_, Mutex<SessionState>>) -> Result<Vec<docops::Attachment>, EngineError> {
    list_attachments_inner(&st)
}

pub fn list_attachments_inner(st: &Mutex<SessionState>) -> Result<Vec<docops::Attachment>, EngineError> {
    let g = lock(st)?;
    Ok(docops::list_attachments(g.doc.as_ref().ok_or_else(no_doc)?))
}

#[tauri::command]
pub fn save_attachment(st: tauri::State<'_, Mutex<SessionState>>, index: usize, target: String) -> Result<CommitResult, EngineError> {
    save_attachment_inner(&st, index, target)
}

pub fn save_attachment_inner(st: &Mutex<SessionState>, index: usize, target: String) -> Result<CommitResult, EngineError> {
    let g = lock(st)?;
    let target = PathBuf::from(target);
    guard_target(&g, &target, true)?;
    let bytes = docops::attachment_bytes(g.doc.as_ref().ok_or_else(no_doc)?, index)?;
    write_atomic(&target, &bytes)
}

/// Write an export (text, image, DOCX, ...) produced by the frontend.
#[tauri::command]
pub fn write_file(st: tauri::State<'_, Mutex<SessionState>>, path: String, b64: String) -> Result<CommitResult, EngineError> {
    write_file_inner(&st, path, b64)
}

pub fn write_file_inner(st: &Mutex<SessionState>, path: String, b64: String) -> Result<CommitResult, EngineError> {
    let bytes = B64.decode(b64.as_bytes()).map_err(|e| EngineError::new(ErrorCode::ValidationFailed, format!("bad data: {e}")))?;
    let target = PathBuf::from(path);
    if let Ok(g) = st.lock() {
        if g.path.is_some() {
            guard_target(&g, &target, true)?;
        }
    }
    write_atomic(&target, &bytes)
}

/// Temp file in the target directory, fsync, rename — never a half-written file.
fn write_atomic(target: &std::path::Path, bytes: &[u8]) -> Result<CommitResult, EngineError> {
    use std::io::Write;
    let dir = target.parent().filter(|p| !p.as_os_str().is_empty()).ok_or_else(|| EngineError::new(ErrorCode::IoError, "target has no folder"))?;
    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("export");
    let tmp = dir.join(format!(".stitchup-tmp-{}-{name}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(EngineError::new(ErrorCode::AtomicPublishFailed, e.to_string()));
    }
    Ok(CommitResult {
        new_sha256: stitchup_core::hash::sha256_hex(bytes),
        size_bytes: bytes.len() as u64,
        published_path: target.to_path_buf(),
    })
}

/// Open the macOS print panel for the window (the page prints whatever the
/// frontend placed in its print-only layer).
#[tauri::command]
pub fn print_window(window: tauri::WebviewWindow) -> Result<(), EngineError> {
    window.print().map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))
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

// ---------------------------------------------------------------------------
// Annotations, content objects, redaction, signatures, links
// ---------------------------------------------------------------------------

use stitchup_core::{annots, forms, objects, sign};

#[tauri::command]
pub fn add_annotation(st: tauri::State<'_, Mutex<SessionState>>, page: u32, spec: Value) -> Result<Vec<PageMeta>, EngineError> {
    add_annotation_inner(&st, page, spec)
}

pub fn add_annotation_inner(st: &Mutex<SessionState>, page: u32, spec: Value) -> Result<Vec<PageMeta>, EngineError> {
    let spec: annots::AnnotSpec = serde_json::from_value(spec).map_err(validation)?;
    let mut g = lock(st)?;
    logged(&mut g, "Annotate", |doc| annots::add(doc, page, &spec).map(|_| ()))
}

#[tauri::command]
pub fn delete_annotation(st: tauri::State<'_, Mutex<SessionState>>, page: u32, obj: u32) -> Result<Vec<PageMeta>, EngineError> {
    delete_annotation_inner(&st, page, obj)
}

pub fn delete_annotation_inner(st: &Mutex<SessionState>, page: u32, obj: u32) -> Result<Vec<PageMeta>, EngineError> {
    let mut g = lock(st)?;
    logged(&mut g, "Delete annotation", |doc| annots::delete(doc, page, obj))
}

#[tauri::command]
pub fn move_annotation(st: tauri::State<'_, Mutex<SessionState>>, page: u32, obj: u32, dx: f64, dy: f64) -> Result<Vec<PageMeta>, EngineError> {
    move_annotation_inner(&st, page, obj, dx, dy)
}

pub fn move_annotation_inner(st: &Mutex<SessionState>, page: u32, obj: u32, dx: f64, dy: f64) -> Result<Vec<PageMeta>, EngineError> {
    let mut g = lock(st)?;
    logged(&mut g, "Move annotation", |doc| annots::translate(doc, page, obj, dx, dy).map(|_| ()))
}

/// True redaction of a rectangle (PDF user space) on a page.
#[tauri::command]
pub fn redact_area(st: tauri::State<'_, Mutex<SessionState>>, page: u32, rect: [f64; 4]) -> Result<Value, EngineError> {
    redact_area_inner(&st, page, rect)
}

pub fn redact_area_inner(st: &Mutex<SessionState>, page: u32, rect: [f64; 4]) -> Result<Value, EngineError> {
    let mut g = lock(st)?;
    let mut report = objects::RedactReport::default();
    let pages = logged(&mut g, "Redact", |doc| {
        report = objects::redact(doc, page, rect)?;
        Ok(())
    })?;
    Ok(serde_json::json!({ "pages": pages, "report": report }))
}

#[tauri::command]
pub fn list_objects(st: tauri::State<'_, Mutex<SessionState>>, page: u32) -> Result<Vec<objects::PageObject>, EngineError> {
    list_objects_inner(&st, page)
}

pub fn list_objects_inner(st: &Mutex<SessionState>, page: u32) -> Result<Vec<objects::PageObject>, EngineError> {
    let g = lock(st)?;
    objects::list(g.doc.as_ref().ok_or_else(no_doc)?, page)
}

#[derive(Debug, Deserialize)]
pub struct ObjectOp {
    /// "delete" | "move" | "recolor" | "font"
    pub op: String,
    pub ids: Vec<String>,
    #[serde(default)]
    pub dx: f64,
    #[serde(default)]
    pub dy: f64,
    #[serde(default)]
    pub color: Option<[f64; 3]>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub size: Option<f64>,
}

/// Delete / move / recolour / change the font of selected content objects.
#[tauri::command]
pub fn object_op(st: tauri::State<'_, Mutex<SessionState>>, page: u32, data: Value) -> Result<Vec<PageMeta>, EngineError> {
    object_op_inner(&st, page, data)
}

pub fn object_op_inner(st: &Mutex<SessionState>, page: u32, data: Value) -> Result<Vec<PageMeta>, EngineError> {
    let d: ObjectOp = serde_json::from_value(data).map_err(validation)?;
    let mut g = lock(st)?;
    let label = match d.op.as_str() {
        "delete" => "Delete object",
        "move" => "Move object",
        "recolor" => "Recolor",
        "font" => "Change font",
        other => return Err(EngineError::new(ErrorCode::ValidationFailed, format!("unknown object op {other}"))),
    };
    logged(&mut g, label, |doc| {
        match d.op.as_str() {
            "delete" => objects::delete(doc, page, &d.ids),
            "move" => objects::translate(doc, page, &d.ids, d.dx, d.dy),
            "recolor" => objects::recolor(doc, page, &d.ids, d.color.unwrap_or([0.0, 0.0, 0.0])),
            _ => objects::set_font(doc, page, &d.ids, d.family.as_deref().unwrap_or("Helvetica"), d.size),
        }
        .map(|_| ())
    })
}

#[tauri::command]
pub fn insert_shape(st: tauri::State<'_, Mutex<SessionState>>, page: u32, spec: Value) -> Result<Vec<PageMeta>, EngineError> {
    insert_shape_inner(&st, page, spec)
}

pub fn insert_shape_inner(st: &Mutex<SessionState>, page: u32, spec: Value) -> Result<Vec<PageMeta>, EngineError> {
    let spec: objects::ShapeSpec = serde_json::from_value(spec).map_err(validation)?;
    let mut g = lock(st)?;
    logged(&mut g, "Insert shape", |doc| objects::insert_shape(doc, page, &spec))
}

/// Digitally sign and save a signed copy (the open document is unchanged).
#[tauri::command]
pub fn sign_document(st: tauri::State<'_, Mutex<SessionState>>, target: String, spec: Value) -> Result<CommitResult, EngineError> {
    sign_document_inner(&st, target, spec)
}

pub fn sign_document_inner(st: &Mutex<SessionState>, target: String, spec: Value) -> Result<CommitResult, EngineError> {
    let spec: sign::SignSpec = serde_json::from_value(spec).map_err(validation)?;
    let g = lock(st)?;
    let target = PathBuf::from(target);
    guard_target(&g, &target, true)?;
    // Unchanged since it was opened/saved → sign the file's exact bytes as an
    // incremental update, so signatures already in it stay valid.
    let pristine = !g.dirty() && !g.untitled && g.password.is_none();
    let on_disk = g.path.as_ref().and_then(|p| std::fs::read(p).ok()).filter(|b| {
        g.source_hash.as_deref() == Some(stitchup_core::hash::sha256_hex(b).as_str())
    });
    let bytes = match (pristine, on_disk) {
        (true, Some(b)) => sign::sign_bytes(b, &spec)?,
        _ => sign::sign_document(g.doc.as_ref().ok_or_else(no_doc)?, &spec)?,
    };
    write_atomic(&target, &bytes)
}

/// Open a web/mail link from a PDF in the default browser / mail app.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), EngineError> {
    let lower = url.to_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
        return Err(EngineError::new(ErrorCode::PermissionDenied, "only web and mail links are opened"));
    }
    std::process::Command::new("open").arg(&url).spawn().map(|_| ()).map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))
}

/// All form fields with their widgets (page + rect) and current values.
#[tauri::command]
pub fn list_fields(st: tauri::State<'_, Mutex<SessionState>>) -> Result<Vec<forms::Field>, EngineError> {
    list_fields_inner(&st)
}

pub fn list_fields_inner(st: &Mutex<SessionState>) -> Result<Vec<forms::Field>, EngineError> {
    let g = lock(st)?;
    Ok(g.doc.as_ref().map(forms::list).unwrap_or_default())
}

/// Fill one field (text/choice: string, checkbox: bool, radio: option name).
#[tauri::command]
pub fn set_field(st: tauri::State<'_, Mutex<SessionState>>, name: String, value: Value) -> Result<Vec<PageMeta>, EngineError> {
    set_field_inner(&st, name, value)
}

pub fn set_field_inner(st: &Mutex<SessionState>, name: String, value: Value) -> Result<Vec<PageMeta>, EngineError> {
    let v: forms::FieldValue = serde_json::from_value(value).map_err(validation)?;
    let mut g = lock(st)?;
    let ids = forms::touched(g.doc.as_ref().ok_or_else(no_doc)?, &name);
    logged_objects(&mut g, "Fill form", &ids, |doc| forms::set_value(doc, &name, &v))
}

#[derive(Debug, Deserialize)]
struct ImagePayload {
    format: String,
    b64: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

/// Insert an image (JPEG as-is, or RGBA pixels) into `rect` on a page.
#[tauri::command]
pub fn insert_image(st: tauri::State<'_, Mutex<SessionState>>, page: u32, rect: [f64; 4], image: Value) -> Result<Vec<PageMeta>, EngineError> {
    insert_image_inner(&st, page, rect, image)
}

pub fn insert_image_inner(st: &Mutex<SessionState>, page: u32, rect: [f64; 4], image: Value) -> Result<Vec<PageMeta>, EngineError> {
    let p: ImagePayload = serde_json::from_value(image).map_err(validation)?;
    let bytes = B64.decode(p.b64.as_bytes()).map_err(|e| EngineError::new(ErrorCode::ValidationFailed, format!("bad image data: {e}")))?;
    let img = match p.format.as_str() {
        "jpeg" => objects::ImageData::Jpeg { bytes },
        _ => objects::ImageData::Rgba { width: p.width, height: p.height, pixels: bytes },
    };
    let mut g = lock(st)?;
    logged(&mut g, "Insert image", |doc| objects::insert_image(doc, page, rect, &img).map(|_| ()))
}

/// Read an image file the user picked (for Insert Image).
#[tauri::command]
pub fn read_image_file(path: String) -> Result<HashMap<String, String>, EngineError> {
    let lower = path.to_lowercase();
    if ![".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".heic", ".tif", ".tiff"].iter().any(|e| lower.ends_with(e)) {
        return Err(EngineError::new(ErrorCode::ValidationFailed, "choose a PNG, JPEG, GIF, WebP, BMP, HEIC or TIFF image"));
    }
    let meta = std::fs::metadata(&path)?;
    if meta.len() > 60 * 1024 * 1024 {
        return Err(EngineError::new(ErrorCode::ValidationFailed, "image is larger than 60 MB"));
    }
    Ok(HashMap::from([("b64".to_string(), B64.encode(std::fs::read(&path)?))]))
}

// ---------------------------------------------------------------------------
// Document tabs: every tab is its own session; inactive ones are parked here.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct TabStore {
    pub active: u32,
    pub next: u32,
    pub parked: HashMap<u32, SessionState>,
}

pub static TABS: Mutex<Option<TabStore>> = Mutex::new(None);

fn tabs<'a>() -> Result<MutexGuard<'a, Option<TabStore>>, EngineError> {
    let mut t = TABS.lock().map_err(|_| EngineError::new(ErrorCode::IoError, "tab lock poisoned"))?;
    if t.is_none() {
        *t = Some(TabStore { active: 1, next: 2, parked: HashMap::new() });
    }
    Ok(t)
}

/// Park the current document and start an empty tab; returns its id.
#[tauri::command]
pub fn tab_new(st: tauri::State<'_, Mutex<SessionState>>) -> Result<u32, EngineError> {
    tab_new_inner(&st)
}

pub fn tab_new_inner(st: &Mutex<SessionState>) -> Result<u32, EngineError> {
    let mut t = tabs()?;
    let ts = t.as_mut().unwrap();
    let mut g = lock(st)?;
    let id = ts.next;
    ts.next += 1;
    let current = std::mem::take(&mut *g);
    ts.parked.insert(ts.active, current);
    ts.active = id;
    Ok(id)
}

/// Make tab `id` the active session.
#[tauri::command]
pub fn tab_switch(st: tauri::State<'_, Mutex<SessionState>>, id: u32) -> Result<u32, EngineError> {
    tab_switch_inner(&st, id)
}

pub fn tab_switch_inner(st: &Mutex<SessionState>, id: u32) -> Result<u32, EngineError> {
    let mut t = tabs()?;
    let ts = t.as_mut().unwrap();
    if id == ts.active {
        return Ok(id);
    }
    let target = ts.parked.remove(&id).ok_or_else(|| EngineError::new(ErrorCode::ValidationFailed, format!("no tab {id}")))?;
    let mut g = lock(st)?;
    let current = std::mem::replace(&mut *g, target);
    ts.parked.insert(ts.active, current);
    ts.active = id;
    Ok(id)
}

/// Close tab `id` (the active one is emptied; switch elsewhere first).
#[tauri::command]
pub fn tab_close(st: tauri::State<'_, Mutex<SessionState>>, id: u32) -> Result<(), EngineError> {
    tab_close_inner(&st, id)
}

pub fn tab_close_inner(st: &Mutex<SessionState>, id: u32) -> Result<(), EngineError> {
    let mut t = tabs()?;
    let ts = t.as_mut().unwrap();
    if id == ts.active {
        *lock(st)? = SessionState::default();
    } else {
        ts.parked.remove(&id);
    }
    Ok(())
}

/// Move pages (1-based, active tab) to the end of tab `to`; both sides undoable.
#[tauri::command]
pub fn move_pages_to_tab(st: tauri::State<'_, Mutex<SessionState>>, pages: Vec<u32>, to: u32) -> Result<Vec<PageMeta>, EngineError> {
    move_pages_to_tab_inner(&st, pages, to)
}

pub fn move_pages_to_tab_inner(st: &Mutex<SessionState>, pages: Vec<u32>, to: u32) -> Result<Vec<PageMeta>, EngineError> {
    let mut t = tabs()?;
    let ts = t.as_mut().unwrap();
    let mut g = lock(st)?;
    let src = g.doc.as_ref().ok_or_else(no_doc)?;
    if pages.is_empty() || pages.len() >= collect_pages(src)?.len() {
        return Err(EngineError::new(ErrorCode::ValidationFailed, "a document must keep at least one page"));
    }
    let moved = extract(src, &pages)?;
    let target = ts.parked.get_mut(&to).ok_or_else(|| EngineError::new(ErrorCode::ValidationFailed, format!("no tab {to}")))?;
    if target.doc.is_none() {
        return Err(EngineError::new(ErrorCode::ValidationFailed, "that tab has no document"));
    }
    logged(target, "Pages from another tab", |doc| merge_doc(doc, &moved).map(|_| ()))?;
    logged(&mut g, "Move pages to another tab", |doc| delete_pages(doc, &pages).map(|_| ()))
}

// ---------------------------------------------------------------------------
// OCR: the bundled `stitchup-ocr` helper (Apple Vision, on-device) reads a
// rendered page image; the lines become an invisible text layer.
// ---------------------------------------------------------------------------

fn ocr_helper() -> Option<PathBuf> {
    let mut c = vec![];
    if let Ok(p) = std::env::var("STITCHUP_OCR") {
        c.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            c.push(dir.join("stitchup-ocr")); // inside Stitchup.app/Contents/MacOS
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    c.push(dev.join("stitchup-ocr-universal-apple-darwin"));
    c.push(dev.join(format!("stitchup-ocr-{}-apple-darwin", if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" })));
    c.into_iter().find(|p| p.is_file())
}

/// Recognise the text on one page image (PNG/JPEG, base64). Does not change the document.
#[tauri::command]
pub fn ocr_recognize(b64: String, languages: Option<String>) -> Result<Vec<stitchup_core::ocr::OcrLine>, EngineError> {
    let helper = ocr_helper().ok_or_else(|| EngineError::new(ErrorCode::Unsupported, "the text-recognition helper is missing from this build"))?;
    let bytes = B64.decode(b64.as_bytes()).map_err(|e| EngineError::new(ErrorCode::ValidationFailed, format!("bad image data: {e}")))?;
    let dir = std::env::temp_dir().join("stitchup-ocr");
    std::fs::create_dir_all(&dir)?;
    let img = dir.join(format!("page-{}-{}.png", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    std::fs::write(&img, &bytes)?;
    let mut cmd = std::process::Command::new(helper);
    cmd.arg(&img);
    if let Some(l) = languages.filter(|l| !l.trim().is_empty()) {
        cmd.arg(l);
    }
    let out = cmd.output();
    let _ = std::fs::remove_file(&img);
    let out = out?;
    if !out.status.success() {
        return Err(EngineError::new(ErrorCode::IoError, format!("text recognition failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| EngineError::new(ErrorCode::IoError, format!("text recognition output: {e}")))
}

#[derive(Debug, Deserialize)]
pub struct OcrPage {
    pub page: u32,
    pub rotation: i64,
    pub lines: Vec<stitchup_core::ocr::OcrLine>,
}

/// Write recognised lines into the pages as invisible text — one undo step.
#[tauri::command]
pub fn ocr_apply(st: tauri::State<'_, Mutex<SessionState>>, pages: Vec<OcrPage>) -> Result<Vec<PageMeta>, EngineError> {
    ocr_apply_inner(&st, pages)
}

pub fn ocr_apply_inner(st: &Mutex<SessionState>, pages: Vec<OcrPage>) -> Result<Vec<PageMeta>, EngineError> {
    let mut g = lock(st)?;
    logged(&mut g, "Recognize text", |doc| {
        for p in &pages {
            stitchup_core::ocr::add_text_layer(doc, p.page, p.rotation, &p.lines)?;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Crash recovery: unsaved documents are snapshotted into a private folder
// (never over the user's files); a clean exit removes them. After a crash the
// next launch offers them back.
// ---------------------------------------------------------------------------

pub static RECOVERY_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

fn recovery_dir() -> Result<PathBuf, EngineError> {
    let d = RECOVERY_DIR
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| std::env::temp_dir().join("stitchup-recovery"));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryEntry {
    pub file: String,
    pub name: String,
    pub original_path: Option<String>,
    pub saved_at: u64,
    pub pid: u32,
}

fn manifest_path(dir: &std::path::Path, pid: u32) -> PathBuf {
    dir.join(format!("recovery-{pid}.json"))
}

fn session_name(g: &SessionState) -> (String, Option<String>) {
    match &g.path {
        Some(p) if !g.untitled => (
            p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document.pdf".into()),
            Some(p.to_string_lossy().into_owned()),
        ),
        _ => ("Untitled.pdf".into(), None),
    }
}

/// Snapshot every tab with unsaved changes; returns how many were written.
#[tauri::command]
pub fn save_recovery(st: tauri::State<'_, Mutex<SessionState>>) -> Result<usize, EngineError> {
    save_recovery_inner(&st)
}

pub fn save_recovery_inner(st: &Mutex<SessionState>) -> Result<usize, EngineError> {
    let dir = recovery_dir()?;
    let pid = std::process::id();
    let t = tabs()?;
    let g = lock(st)?;
    let mut sessions: Vec<(u32, &SessionState)> = vec![(t.as_ref().unwrap().active, &*g)];
    for (id, s) in t.as_ref().unwrap().parked.iter() {
        sessions.push((*id, s));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut entries = vec![];
    for (id, s) in sessions {
        let Some(doc) = s.doc.as_ref() else { continue };
        if !s.dirty() {
            continue; // nothing unsaved worth keeping
        }
        let mut bytes = Vec::new();
        doc.clone().save_to(&mut bytes).map_err(|e| EngineError::new(ErrorCode::IoError, e.to_string()))?;
        let file = dir.join(format!("rec_{pid}_{id}.pdf"));
        write_atomic(&file, &bytes)?;
        let (name, original_path) = session_name(s);
        entries.push(RecoveryEntry { file: file.to_string_lossy().into_owned(), name, original_path, saved_at: now, pid });
    }
    // Drop this process's snapshots that are no longer needed.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path().to_string_lossy().into_owned();
            if p.contains(&format!("rec_{pid}_")) && !entries.iter().any(|x| x.file == p) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let n = entries.len();
    if n == 0 {
        let _ = std::fs::remove_file(manifest_path(&dir, pid));
    } else {
        write_atomic(&manifest_path(&dir, pid), serde_json::to_vec_pretty(&entries).unwrap_or_default().as_slice())?;
    }
    Ok(n)
}

/// Clean exit / everything saved: remove this process's snapshots.
#[tauri::command]
pub fn clear_recovery() -> Result<(), EngineError> {
    let dir = recovery_dir()?;
    let pid = std::process::id();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with(&format!("rec_{pid}_")) || n == format!("recovery-{pid}.json") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// Snapshots left behind by a Stitchup that crashed or was killed.
#[tauri::command]
pub fn list_recovery() -> Result<Vec<RecoveryEntry>, EngineError> {
    let dir = recovery_dir()?;
    let me = std::process::id();
    let mut out = vec![];
    for e in std::fs::read_dir(&dir)?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let Some(pid) = n.strip_prefix("recovery-").and_then(|r| r.strip_suffix(".json")).and_then(|p| p.parse::<u32>().ok()) else { continue };
        if pid == me || process_alive(pid) {
            continue;
        }
        if let Ok(list) = serde_json::from_slice::<Vec<RecoveryEntry>>(&std::fs::read(e.path())?) {
            out.extend(list.into_iter().filter(|x| std::path::Path::new(&x.file).exists()));
        }
    }
    Ok(out)
}

/// Open a recovered snapshot as a new, unsaved document in the active tab.
#[tauri::command]
pub fn restore_recovery(st: tauri::State<'_, Mutex<SessionState>>, file: String) -> Result<OpenPayload, EngineError> {
    restore_recovery_inner(&st, file)
}

pub fn restore_recovery_inner(st: &Mutex<SessionState>, file: String) -> Result<OpenPayload, EngineError> {
    let dir = recovery_dir()?;
    let p = PathBuf::from(&file);
    if p.parent() != Some(dir.as_path()) {
        return Err(EngineError::new(ErrorCode::PermissionDenied, "not a recovery file"));
    }
    let payload = combine_documents_inner(st, vec![file.clone()])?;
    let _ = std::fs::remove_file(&p);
    Ok(payload)
}

/// Forget all leftover snapshots from crashed sessions.
#[tauri::command]
pub fn discard_recovery() -> Result<(), EngineError> {
    let dir = recovery_dir()?;
    let me = std::process::id();
    for e in std::fs::read_dir(&dir)?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let pid = n
            .strip_prefix("recovery-")
            .and_then(|r| r.strip_suffix(".json"))
            .or_else(|| n.strip_prefix("rec_").and_then(|r| r.split('_').next()))
            .and_then(|p| p.parse::<u32>().ok());
        if let Some(pid) = pid {
            if pid != me && !process_alive(pid) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

/// Frontend diagnostics → stderr (visible when launched from a terminal).
#[tauri::command]
pub fn log_message(msg: String) {
    eprintln!("[stitchup] {msg}");
}

/// Quit the application cleanly (File ▸ Exit). The session is in-memory only
/// and the source is never modified, so there is nothing to flush first.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}