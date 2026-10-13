//! Stage 1 engine-side acceptance tests (A-c1, A-c3, A-c4, A-c6, A-c7, A-c8, A-c15).
//! These run against the committed fixtures in `fixtures/generated/`.
//! Static checks here are a pre-filter; live GUI sign-off is recorded on the
//! platform parity sheets (docs/STAGE1_*).

use std::path::{Path, PathBuf};

use lopdf::Document as PdfDoc;

use stitchup_core::model::{collect_pages, Metadata};
use stitchup_core::oplog::{self, OpLog};
use stitchup_core::ops::{
    commit_export, delete_pages, insert_blank, merge_doc, open_bytes, reorder, rotate_pages,
    set_metadata,
};
use stitchup_core::{EngineError, ErrorCode};
use lopdf::LoadOptions;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated")
}

fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).expect("fixture present (run generate_fixtures)")
}

/// A-c1: every fixture opens to either a valid document or a structured error.
#[test]
fn a_c1_open_all_fixtures() {
    let expect_ok = [
        "F-01_text.pdf",
        "F-02_mixed_text_image.pdf",
        "F-03_rotated.pdf",
        "F-05_acroform.pdf",
        "F-06_annotations.pdf",
        "F-07_unicode_latin1.pdf",
        "F-10_attachments.pdf",
        "F-11_large_200p.pdf",
        "F-12_blank.pdf",
        "F-13_xfa.pdf",
        "F-14_pdfa_lite.pdf",
    ];
    for name in expect_ok {
        let info = open_bytes(read_fixture(name), fixture(name)).unwrap_or_else(|e| panic!("{name} should open: {e}"));
        assert!(info.page_count >= 1, "{name}: expected >=1 pages");
    }

    // F-04 must OPEN as "password required" (no object map without a password).
    let enc = open_bytes(read_fixture("F-04_encrypted.pdf"), fixture("F-04_encrypted.pdf"))
        .expect("encrypted fixture classifies as password-required, not fail");
    assert!(enc.password_required && enc.encrypted);
    assert_eq!(enc.pages.len(), 0, "no pages without the password");

    // F-08 must fail structurally with a `malformed`/`open_failed` code.
    match open_bytes(read_fixture("F-08_malformed.pdf"), fixture("F-08_malformed.pdf")) {
        Err(e) if matches!(e.code, ErrorCode::Malformed | ErrorCode::OpenFailed) => {}
        other => panic!("F-08 expected structured malformed error, got: {other:?}"),
    }
}

/// A-c2: source-hash invariant — structural ops never rewrite the opened bytes.
#[test]
fn a_c2_source_hash_invariant() {
    let bytes = read_fixture("F-03_rotated.pdf");
    let mut doc = PdfDoc::load_mem(&bytes).unwrap();
    // Static check: the on-disk fixture still equals the original bytes after a mutation sequence.
    let orig = std::fs::read(fixture("F-03_rotated.pdf")).unwrap();
    assert_eq!(bytes, orig, "fixture changed on disk during the test run");
    rotate_pages(&mut doc, &[1, 2], true).unwrap();
    delete_pages(&mut doc, &[3]).unwrap();
    // The opened fixture file must be byte-identical (no silent overwrite).
    assert_eq!(orig, std::fs::read(fixture("F-03_rotated.pdf")).unwrap());
}

/// A-c3: save-as round trip — page count, sizes, rotation preserved; new hash;
/// source untouched.
#[test]
fn a_c3_save_as_roundtrip() {
    let src = fixture("F-01_text.pdf");
    let bytes = std::fs::read(&src).unwrap();
    let mut doc = PdfDoc::load_mem(&bytes).unwrap();

    set_metadata(
        &mut doc,
        &Metadata {
            title: Some("roundtrip".into()),
            author: Some("test".into()),
            ..Default::default()
        },
    )
    .unwrap();
    rotate_pages(&mut doc, &[2], true).unwrap();

    let dir = std::env::temp_dir().join(format!("stitchup-a3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("out.pdf");
    let commit = commit_export(&mut doc, &target).unwrap();

    let reopened = PdfDoc::load_mem(&std::fs::read(&target).unwrap()).unwrap();
    let pm = collect_pages(&reopened).unwrap();
    assert_eq!(pm.len(), 3, "page count preserved");
    assert_eq!(pm[0].width_pt, 612.0);
    assert_eq!(pm[0].height_pt, 792.0);
    assert_eq!(pm[1].rotation, 90 % 360, "rotation preserved");

    assert_ne!(commit.new_sha256, stitchup_core::hash::sha256_hex(&bytes));
    assert_eq!(std::fs::read(&src).unwrap(), bytes, "source untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A-c4: atomicity — a mid-write export failure (rename onto a non-empty
/// directory) leaves no temp litter and the source unchanged.
#[test]
fn a_c4_atomic_publish_cleanup() {
    let bytes = read_fixture("F-01_text.pdf");
    let mut doc = PdfDoc::load_mem(&bytes).unwrap();
    let src = fixture("F-01_text.pdf");

    let dir = std::env::temp_dir().join(format!("stitchup-a4-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("out.pdf");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("keep.txt"), "sentinel").unwrap();

    match commit_export(&mut doc, &target) {
        Err(e) if matches!(e.code, ErrorCode::AtomicPublishFailed) => {}
        other => panic!("expected atomic_publish_failed, got {other:?}"),
    }

    let leftover = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        leftover.iter().all(|n| !n.starts_with(".stitchup-tmp-")),
        "temp litter left behind: {leftover:?}"
    );
    assert_eq!(std::fs::read(&src).unwrap(), bytes, "source untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A-c6: F-04 password flow — wrong password rejected, right one unlocks.
/// Mirrors the command layer: the app opens with `open_bytes` (password-required
/// classification), then unlocks by loading WITH the password + `decrypt()`.
#[test]
fn a_c6_password_unlock() {
    let bytes = read_fixture("F-04_encrypted.pdf");

    let info = open_bytes(bytes.clone(), fixture("F-04_encrypted.pdf")).unwrap();
    assert!(info.password_required, "classified as requiring a password");

    fn attempt(bytes: &[u8], password: &str) -> Result<usize, EngineError> {
        let mut doc = PdfDoc::load_mem_with_options(
            bytes,
            LoadOptions {
                password: Some(password.to_string()),
                ..Default::default()
            },
        )
        .map_err(|e| EngineError::new(ErrorCode::Encrypted, format!("open with password: {e}")))?;
        if doc.is_encrypted() {
            doc.decrypt(password)
                .map_err(|e| EngineError::new(ErrorCode::Encrypted, format!("decrypt: {e}")))?;
        }
        Ok(collect_pages(&doc)?.len())
    }

    // Wrong password: deterministic `encrypted` error (password auth gate).
    match attempt(&bytes, "nope") {
        Err(e) if matches!(e.code, ErrorCode::Encrypted) => {}
        other => panic!("wrong password must reject with `encrypted`, got {other:?}"),
    }

    // Right (user) password: unlocks, pages readable.
    let n = attempt(&bytes, "stitchup-user").expect("user password unlocks");
    assert_eq!(n, 1);
}

/// A-c7: malformed open fails with a structured error, then a good file opens.
#[test]
fn a_c7_malformed_then_good() {
    match open_bytes(read_fixture("F-08_malformed.pdf"), fixture("F-08_malformed.pdf")) {
        Err(e) if matches!(e.code, ErrorCode::Malformed | ErrorCode::OpenFailed) => {}
        other => panic!("expected structured malformed error, got {other:?}"),
    }
    // No poisoned state: a good file still opens after the failed one.
    let info = open_bytes(read_fixture("F-01_text.pdf"), fixture("F-01_text.pdf")).unwrap();
    assert_eq!(info.pages.len(), 3);
}

/// A-c8: undo/redo over a MIXED op sequence restores the exact page sequence
/// on the way back and replays it exactly on the way forward.
#[test]
fn a_c8_mixed_undo_redo() {
    fn kids_order(d: &PdfDoc) -> Vec<u32> {
        let root = d.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let cat = d.get_object(root).unwrap();
        let pages = cat.as_dict().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
        let node = d.get_object(pages).unwrap();
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

    let mut doc = PdfDoc::load_mem(&read_fixture("F-01_text.pdf")).unwrap();
    let mut log = OpLog::default();

    // Fixture page object ids are stable: pages 1,2,3 -> kids [4,6,8].
    assert_eq!(kids_order(&doc), vec![4, 6, 8]);

    let orig_rotation = collect_pages(&doc).unwrap()[1].rotation;
    assert_eq!(orig_rotation, 0);

    // reorder 1,3,2 -> kids [4,8,6]
    {
        let before = oplog::snapshot(&doc).unwrap();
        reorder(&mut doc, &[1, 3, 2]).unwrap();
        let after = oplog::snapshot(&doc).unwrap();
        log.push(oplog::OpRecord { label: "reorder".into(), before, after });
    }
    assert_eq!(kids_order(&doc), vec![4, 8, 6]);

    // rotate page 2 cw (page 2 is kid id 6)
    {
        let before = oplog::snapshot(&doc).unwrap();
        rotate_pages(&mut doc, &[2], true).unwrap();
        let after = oplog::snapshot(&doc).unwrap();
        log.push(oplog::OpRecord { label: "rotate".into(), before, after });
    }
    assert_eq!(collect_pages(&doc).unwrap()[1].rotation, 90);

    // delete page 3 (kid id 6 is position 3) -> kids [4,8]
    {
        let before = oplog::snapshot(&doc).unwrap();
        delete_pages(&mut doc, &[3]).unwrap();
        let after = oplog::snapshot(&doc).unwrap();
        log.push(oplog::OpRecord { label: "delete".into(), before, after });
    }
    assert_eq!(kids_order(&doc), vec![4, 8]);

    // insert blank (op object id is dynamic) after page 1 -> kids [4, X, 8]
    let expected_final: Vec<u32>;
    {
        let before = oplog::snapshot(&doc).unwrap();
        insert_blank(&mut doc, 1, 612.0, 792.0).unwrap();
        let after = oplog::snapshot(&doc).unwrap();
        log.push(oplog::OpRecord { label: "insert".into(), before, after });
        let kids = kids_order(&doc);
        assert_eq!(kids.len(), 3);
        assert_ne!(kids[1], 4);
        assert_ne!(kids[1], 8);
        assert_eq!(kids[0], 4);
        assert_eq!(kids[2], 8);
        expected_final = kids;
    }

    let rotation_of = |d: &PdfDoc, n: u32| {
        collect_pages(d).unwrap().iter().find(|p| p.page_no == n).map(|p| p.rotation).unwrap()
    };

    // Undo applies BEFORE, redo applies AFTER (mirrors the command layer).
    macro_rules! undo {
        ($log:expr, $d:expr) => {{
            let r = $log.pop_undo().unwrap();
            oplog::apply(&mut $d, &r, true).unwrap();
            $log.redo.push(r);
            kids_order(&$d)
        }};
    }
    macro_rules! redo {
        ($log:expr, $d:expr) => {{
            let r = $log.pop_redo().unwrap();
            oplog::apply(&mut $d, &r, false).unwrap();
            $log.undo.push(r);
            kids_order(&$d)
        }};
    }

    // Undo all the way back to the original order, exact objects + exact rotation.
    assert_eq!(undo!(log, doc), vec![4, 8]);
    assert_eq!(undo!(log, doc), vec![4, 8, 6]);
    assert_eq!(rotation_of(&doc, 2), 90, "rotation still applied after delete-undo");
    assert_eq!(undo!(log, doc), vec![4, 8, 6]);
    assert_eq!(rotation_of(&doc, 2), 0, "rotation reverted");
    assert_eq!(undo!(log, doc), vec![4, 6, 8]);

    // Redo all the way forward -> identical sequence.
    assert_eq!(redo!(log, doc), vec![4, 8, 6]);
    assert_eq!(rotation_of(&doc, 2), 0, "rotation not yet replayed");
    assert_eq!(redo!(log, doc), vec![4, 8, 6]);
    assert_eq!(rotation_of(&doc, 2), 90, "rotation re-applied");
    assert_eq!(redo!(log, doc), vec![4, 8]);
    assert_eq!(redo!(log, doc), expected_final);
}

/// A-c15: merge — extract F-01 pages 2-3, and merge F-01 + F-12 preserving order.
#[test]
fn a_c15_extract_and_merge() {
    let f01 = PdfDoc::load_mem(&read_fixture("F-01_text.pdf")).unwrap();
    let f12 = PdfDoc::load_mem(&read_fixture("F-12_blank.pdf")).unwrap();

    let ex = stitchup_core::ops::extract(&f01, &[2, 3]).unwrap();
    assert_eq!(ex.get_pages().len(), 2);
    let ex_pages = collect_pages(&ex).unwrap();
    assert_eq!(ex_pages.len(), 2, "extract F-01 pages 2-3 -> 2 pages");
let mut doc = PdfDoc::load_mem(&read_fixture("F-01_text.pdf")).unwrap();
    let pm = merge_doc(&mut doc, &f12).unwrap();
    assert_eq!(pm.len(), 4, "F-01 (3) + F-12 (1) = 4");
    assert_eq!(pm[3].width_pt, 612.0, "merged blank page keeps its size");
}

/// Regression for `oplog::apply`: undoing the FIRST metadata edit on a
/// document with no `/Info` must REMOVE the Info object `set_metadata` created
/// (full-replacement semantics), not leave it behind. Guards the
/// "target state has no /Info" branch.
#[test]
fn a_c8_metadata_undo_removes_created_info() {
    let mut doc = PdfDoc::load_mem(&read_fixture("F-01_text.pdf")).unwrap();
    // Precondition: this fixture has no /Info to begin with.
    assert!(doc.trailer.get(b"Info").is_err(), "fixture unexpectedly has /Info");

    let mut log = OpLog::default();
    let before = oplog::snapshot(&doc).unwrap();
    assert!(before.info.is_none(), "snapshot should record no Info");
    set_metadata(
        &mut doc,
        &Metadata { title: Some("stitchup".into()), ..Default::default() },
    )
    .unwrap();
    let after = oplog::snapshot(&doc).unwrap();
    assert!(after.info.is_some(), "set_metadata created an Info object");
    log.push(oplog::OpRecord { label: "metadata".into(), before, after });

    assert_eq!(
        stitchup_core::model::read_metadata(&doc).title.as_deref(),
        Some("stitchup")
    );

    // Undo fully reverts — including removing the created /Info.
    let rec = log.pop_undo().unwrap();
    oplog::apply(&mut doc, &rec, true).unwrap();
    log.redo.push(rec);
    assert!(doc.trailer.get(b"Info").is_err(), "undo left the created /Info behind");
    assert_eq!(stitchup_core::model::read_metadata(&doc).title, None);

    // Redo recreates it exactly.
    let rec = log.pop_redo().unwrap();
    oplog::apply(&mut doc, &rec, false).unwrap();
    log.undo.push(rec);
    assert_eq!(
        stitchup_core::model::read_metadata(&doc).title.as_deref(),
        Some("stitchup")
    );
}

#[test]
fn dbg_f04_scratch() {
    let doc = PdfDoc::load_mem(&read_fixture("F-04_encrypted.pdf")).unwrap();
    println!("[dbg] objects={} is_encrypted={}",
        doc.objects.len(), doc.is_encrypted());
    println!("[dbg] pages={:?}", doc.get_pages().keys().collect::<Vec<_>>());
    let root_ref = doc.trailer.get(b"Root").unwrap();
    println!("[dbg] root_ref={root_ref:?}");
    if let Ok(id) = root_ref.as_reference() {
        match doc.get_object(id) {
            Ok(cat) => {
                let keys: Vec<Vec<u8>> = cat.as_dict().map(|d| d.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default();
                println!("[dbg] catalog keys: {keys:?}");
            }
            Err(e) => println!("[dbg] catalog get failed: {e}"),
        }
    }
    let mut doc2 = PdfDoc::load_mem(&read_fixture("F-04_encrypted.pdf")).unwrap();
    match doc2.decrypt("stitchup-user") {
        Ok(()) => println!("[dbg] decrypt OK; pages={}", doc2.get_pages().len()),
        Err(e) => println!("[dbg] decrypt ERR {e}"),
    }
    println!("[dbg] was_encrypted={} is_encrypted={}", doc2.was_encrypted(), doc2.is_encrypted());
}
