//! Headless acceptance tests for the Tauri IPC session layer.
//!
//! Every `#[tauri::command]` has a session-logic twin (`*_inner`) that takes
//! `&Mutex<SessionState>` and needs no `tauri::App`, window, or display. These
//! tests drive that layer end-to-end (open → mutate → undo/redo → export,
//! plus the encrypted and no-session gates) so the exact surface the frontend
//! adapters call (`ipc.ts`) is verified without a GUI. Live GUI sign-off stays
//! on the platform parity sheets; this is the static pre-filter for it.
//!
//! Fixtures live in `fixtures/generated/` (build with
//! `cargo run -p stitchup-core --bin generate_fixtures`). Each test works in its
//! own temp directory and never writes into the committed fixtures.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use lopdf::Document as PdfDoc;
use serde_json::json;

use stitchup_core::ErrorCode;
use stitchup_lib::commands::{
    cmd_set_metadata_inner, commit_export_inner, merge_document_inner, open_document_inner,
    page_op_inner, redo_inner, render_bytes_inner, source_hash_inner, undo_inner,
    unlock_document_inner, SessionState,
};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    // src-tauri -> app -> source -> repo root, then fixtures/generated.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/generated")
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir().join(name))
        .unwrap_or_else(|e| panic!("fixture {name} unreadable (run generate_fixtures): {e}"))
}

/// Fresh, uniquely-tagged temp dir (tests in one process run in parallel).
fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stitchup-cmd-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Copy a fixture into the workspace; return (path, exact bytes).
fn staged(name: &str, tag: &str) -> (PathBuf, Vec<u8>) {
    let src = workspace(tag).join(name);
    let bytes = fixture_bytes(name);
    std::fs::write(&src, &bytes).unwrap();
    (src, bytes)
}

fn session() -> Mutex<SessionState> {
    Mutex::new(SessionState::default())
}

fn as_path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn decode_b64(s: &str) -> Vec<u8> {
    B64.decode(s).expect("valid base64")
}

/// Serialize the LIVE session document through `render_bytes` and load it —
/// this also exercises `render_bytes_inner` end-to-end.
fn live_doc(st: &Mutex<SessionState>) -> PdfDoc {
    let map = render_bytes_inner(st).expect("render_bytes ok");
    let b64 = map.get("bytes_b64").expect("bytes_b64 present");
    PdfDoc::load_mem(&decode_b64(b64)).expect("serialized document loads")
}

/// Page order as raw `/Kids` object ids (stable across a session, like the
/// core stage-1 test relies on) — lets us observe a reorder through the
/// command layer, which only exposes sequentially-numbered `PageMeta`.
fn kids_order(doc: &PdfDoc) -> Vec<u32> {
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let cat = doc.get_object(root).unwrap();
    let pages = cat.as_dict().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
    let node = doc.get_object(pages).unwrap();
    node.as_dict()
        .unwrap()
        .get(b"Kids")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_reference().unwrap().0)
        .collect()
}

// ---------------------------------------------------------------------------
// open
// ---------------------------------------------------------------------------

#[test]
fn open_payload_shape_and_stable_bytes() {
    let (src, bytes) = staged("F-01_text.pdf", "open-shape");
    let st = session();

    let p = open_document_inner(&st, as_path(&src)).unwrap();
    assert_eq!(p.path, as_path(&src));
    assert_eq!(p.page_count, 3);
    assert_eq!(p.pages.len(), 3);
    assert!(!p.password_required && !p.encrypted);
    assert_eq!(decode_b64(&p.bytes_b64), bytes, "opened bytes are verbatim");
    assert_eq!(p.source_hash, stitchup_core::hash::sha256_hex(&bytes));
    assert_eq!(p.source_size, bytes.len() as u64);

    // the source_hash command agrees with the open payload
    let h = source_hash_inner(&st).unwrap();
    assert_eq!(h.get("source_hash").unwrap(), &p.source_hash);

    // and the on-disk source is untouched
    assert_eq!(std::fs::read(&src).unwrap(), bytes);
}

#[test]
fn reopen_returns_in_memory_state_not_disk() {
    let (src, bytes) = staged("F-01_text.pdf", "reopen");
    let st = session();

    let first = open_document_inner(&st, as_path(&src)).unwrap();
    // mutate the session only (never the disk file)
    page_op_inner(&st, "delete".into(), json!({"pages":[3]})).unwrap();

    let again = open_document_inner(&st, as_path(&src)).unwrap();
    assert_eq!(again.page_count, 2, "reopen must see in-memory mutations, not disk");
    assert_eq!(again.bytes_b64, first.bytes_b64, "opened bytes stay stable across ops");
    assert_eq!(again.source_hash, first.source_hash);

    // disk still holds the pristine 3-page source
    assert_eq!(std::fs::read(&src).unwrap(), bytes);
}

// ---------------------------------------------------------------------------
// structural ops + undo/redo
// ---------------------------------------------------------------------------

#[test]
fn reorder_moves_pages_and_unknown_kind_is_rejected() {
    let (src, _) = staged("F-01_text.pdf", "reorder");
    let st = session();
    open_document_inner(&st, as_path(&src)).unwrap();

    let before = kids_order(&live_doc(&st));
    assert_eq!(before.len(), 3);

    let pages = page_op_inner(&st, "reorder".into(), json!({"new_order":[3,1,2]})).unwrap();
    assert_eq!(pages.len(), 3);

    let after = kids_order(&live_doc(&st));
    assert_eq!(after, vec![before[2], before[0], before[1]], "reorder applied to the live doc");

    match page_op_inner(&st, "frobnicate".into(), json!({})) {
        Err(e) if e.code == ErrorCode::ValidationFailed => {}
        other => panic!("unknown op kind should be validation_failed, got {other:?}"),
    }
}

#[test]
fn mixed_ops_and_undo_redo_roundtrip() {
    // undo with no history is a validation error (fresh session)
    let empty = session();
    match undo_inner(&empty) {
        Err(e) if e.code == ErrorCode::ValidationFailed => {}
        other => panic!("undo with no history should fail validation, got {other:?}"),
    }

    let (src, _) = staged("F-01_text.pdf", "ops");
    let st = session();
    let open = open_document_inner(&st, as_path(&src)).unwrap();
    let initial_rot = open.pages[0].rotation;

    let ins = page_op_inner(
        &st,
        "insert_blank".into(),
        json!({"after":1,"width_pt":300.0,"height_pt":400.0}),
    )
    .unwrap();
    assert_eq!(ins.len(), 4);
    assert_eq!((ins[1].width_pt, ins[1].height_pt), (300.0, 400.0));

    assert_eq!(undo_inner(&st).unwrap().len(), 3, "undo removes the inserted page");
    let redone = redo_inner(&st).unwrap();
    assert_eq!(redone.len(), 4);
    assert_eq!(redone[1].width_pt, 300.0, "redo restores the exact inserted page");

    assert_eq!(page_op_inner(&st, "delete".into(), json!({"pages":[4]})).unwrap().len(), 3);

    let rot = page_op_inner(&st, "rotate".into(), json!({"pages":[1],"cw":true})).unwrap();
    assert_eq!(rot[0].rotation, (initial_rot + 90) % 360);

    assert_eq!(undo_inner(&st).unwrap()[0].rotation, initial_rot, "undo reverts rotation");
    assert_eq!(undo_inner(&st).unwrap().len(), 4, "undo reverts the delete");
    assert_eq!(redo_inner(&st).unwrap().len(), 3, "redo re-applies the delete");
}

#[test]
fn set_metadata_is_logged_and_undoable() {
    let (src, _) = staged("F-01_text.pdf", "meta");
    let st = session();
    let open = open_document_inner(&st, as_path(&src)).unwrap();
    let orig_title = open.metadata.title.clone();

    let pages = cmd_set_metadata_inner(&st, json!({"title":"Stitchup Test","author":"ci"})).unwrap();
    assert_eq!(pages.len(), 3);

    let meta = stitchup_core::model::read_metadata(&live_doc(&st));
    assert_eq!(meta.title.as_deref(), Some("Stitchup Test"));
    assert_eq!(meta.author.as_deref(), Some("ci"));

    undo_inner(&st).unwrap();
    assert_eq!(stitchup_core::model::read_metadata(&live_doc(&st)).title, orig_title);
}

// ---------------------------------------------------------------------------
// merge
// ---------------------------------------------------------------------------

#[test]
fn merge_appends_pages_and_refuses_self() {
    let (src, _) = staged("F-01_text.pdf", "merge");
    let dir = src.parent().unwrap().to_path_buf();
    let other = dir.join("F-12_blank.pdf");
    std::fs::write(&other, fixture_bytes("F-12_blank.pdf")).unwrap();

    let st = session();
    open_document_inner(&st, as_path(&src)).unwrap();

    let merged = merge_document_inner(&st, as_path(&other)).unwrap();
    assert_eq!(merged.len(), 4, "F-01 (3) + F-12 (1)");

    match merge_document_inner(&st, as_path(&src)) {
        Err(e) if e.code == ErrorCode::PermissionDenied => {}
        other => panic!("self-merge must be permission_denied, got {other:?}"),
    }
    match merge_document_inner(&st, as_path(&dir.join("nope.pdf"))) {
        Err(e) if e.code == ErrorCode::IoError => {}
        other => panic!("missing merge source should be io_error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// commit / export data-safety
// ---------------------------------------------------------------------------

#[test]
fn commit_export_data_safety() {
    let (src, bytes) = staged("F-01_text.pdf", "commit");
    let dir = src.parent().unwrap().to_path_buf();
    let st = session();
    open_document_inner(&st, as_path(&src)).unwrap();

    // 1. never save over the opened source
    match commit_export_inner(&st, as_path(&src), true) {
        Err(e) if e.code == ErrorCode::PermissionDenied => {}
        other => panic!("export over the source must be permission_denied, got {other:?}"),
    }

    // 2. refuse to clobber an existing target without overwrite
    let target = dir.join("out.pdf");
    std::fs::write(&target, b"pre-existing").unwrap();
    match commit_export_inner(&st, as_path(&target), false) {
        Err(e) if e.code == ErrorCode::PermissionDenied => {}
        other => panic!("existing target without overwrite must be refused, got {other:?}"),
    }

    // 3. happy path (overwrite enabled)
    let res = commit_export_inner(&st, as_path(&target), true).unwrap();
    assert_eq!(res.published_path, target);
    let written = std::fs::read(&target).unwrap();
    assert_eq!(res.size_bytes, written.len() as u64);
    assert_eq!(res.new_sha256, stitchup_core::hash::sha256_hex(&written));
    assert_ne!(res.new_sha256, stitchup_core::hash::sha256_hex(&bytes));
    assert_eq!(PdfDoc::load_mem(&written).unwrap().get_pages().len(), 3);

    // 4. source remains byte-identical
    assert_eq!(std::fs::read(&src).unwrap(), bytes);

    // 5. no temp litter left behind
    let litter: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".stitchup-tmp-"))
        .collect();
    assert!(litter.is_empty(), "temp litter: {litter:?}");
}

// ---------------------------------------------------------------------------
// encrypted session (A-c6) and no-session gates
// ---------------------------------------------------------------------------

#[test]
fn encrypted_session_is_gated_until_unlock() {
    let (src, bytes) = staged("F-04_encrypted.pdf", "enc");
    let st = session();

    let locked = open_document_inner(&st, as_path(&src)).unwrap();
    assert!(locked.password_required && locked.encrypted);
    assert_eq!(locked.page_count, 0);
    assert!(locked.pages.is_empty());
    assert_eq!(decode_b64(&locked.bytes_b64), bytes);
    let orig_hash = locked.source_hash.clone();

    match page_op_inner(&st, "delete".into(), json!({"pages":[1]})) {
        Err(e) if e.code == ErrorCode::PasswordRequired => {}
        other => panic!("op while locked must be password_required, got {other:?}"),
    }
    match render_bytes_inner(&st) {
        Err(e) if e.code == ErrorCode::PasswordRequired => {}
        other => panic!("render while locked must be password_required, got {other:?}"),
    }

    match unlock_document_inner(&st, "nope".into()) {
        Err(e) if e.code == ErrorCode::Encrypted => {}
        other => panic!("wrong password must be encrypted, got {other:?}"),
    }

    let open = unlock_document_inner(&st, "stitchup-user".into()).unwrap();
    assert!(!open.password_required);
    assert_eq!(open.page_count, 1);
    assert_eq!(open.source_hash, orig_hash, "unlock does not change the source identity");

    let initial_rot = open.pages[0].rotation;
    let rot = page_op_inner(&st, "rotate".into(), json!({"pages":[1],"cw":true})).unwrap();
    assert_eq!(rot[0].rotation, (initial_rot + 90) % 360);
    assert!(render_bytes_inner(&st).is_ok(), "render is live once unlocked");

    assert_eq!(std::fs::read(&src).unwrap(), bytes, "source never rewritten");
}

#[test]
fn no_session_gates() {
    let st = session();

    match page_op_inner(&st, "delete".into(), json!({"pages":[1]})) {
        Err(e) if e.code == ErrorCode::OpenFailed => {}
        other => panic!("op with no document should be open_failed, got {other:?}"),
    }
    match render_bytes_inner(&st) {
        Err(e) if e.code == ErrorCode::OpenFailed => {}
        other => panic!("render with no document should be open_failed, got {other:?}"),
    }
    match source_hash_inner(&st) {
        Err(e) if e.code == ErrorCode::OpenFailed => {}
        other => panic!("source_hash with no document should be open_failed, got {other:?}"),
    }
    match open_document_inner(&st, "/definitely/not/here.pdf".into()) {
        Err(e) if e.code == ErrorCode::IoError => {}
        other => panic!("opening a missing file should be io_error, got {other:?}"),
    }
}
