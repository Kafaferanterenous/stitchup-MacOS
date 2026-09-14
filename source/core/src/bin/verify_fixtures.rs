//! Fixture verification — reloads each generated fixture and asserts the
//! expectations from docs/STAGE0_FIXTURES_AND_ACCEPTANCE.md §2 (A-c1..A-c15
//! structural subset). Run: `cargo run --bin verify_fixtures`
//! from the project root, or pass a fixtures dir as argv[1].

use std::path::PathBuf;

use lopdf::Document as PdfDoc;
use stitchup_core::{collect_pages, page_rotation};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fixtures/generated"));
    let mut failures = 0usize;

    macro_rules! check {
        ($name:expr, $cond:expr) => {
            if !$cond {
                failures += 1;
                println!("FAIL  {}: {}", $name, stringify!($cond));
            }
        };
    }

    // F-01 text: 3 pages, US-letter.
    let f01 = bytes(&dir, "F-01_text.pdf", &mut failures);
    if let Some(b) = f01 {
        let doc = open_ok(&dir, "F-01_text.pdf", b, &mut failures);
        if let Ok(d) = doc {
            let pages = collect_pages(&d).expect("collect_pages on F-01");
            check!("F-01 page count", pages.len() == 3);
            check!("F-01 letter width", pages.iter().all(|p| (p.width_pt - 612.0).abs() < 0.5));
            check!("F-01 letter height", pages.iter().all(|p| (p.height_pt - 792.0).abs() < 0.5));
            check!("F-01 rotations 0", pages.iter().all(|p| p.rotation == 0));
            check!("F-01 no annots", pages.iter().all(|p| !p.has_annotations));
        }
    }

    // F-03 rotated: 0/90/180/270.
    let f03 = bytes(&dir, "F-03_rotated.pdf", &mut failures);
    if let Some(b) = f03 {
        let doc = open_ok(&dir, "F-03_rotated.pdf", b, &mut failures);
        if let Ok(d) = doc {
            let pages = collect_pages(&d).expect("collect_pages on F-03");
            let expect = vec![0i64, 90, 180, 270];
            check!("F-03 page count", pages.len() == 4);
            for (meta, want) in pages.iter().zip(&expect) {
                check!("F-03 rotation", meta.rotation == *want);
            }
        }
    }

    // F-04 encrypted: lopdf is lazy about decryption — a plain load keeps the
    // /Encrypt trailer (is_encrypted() true); a password load strips it but
    // records was_encrypted() true. Our contract flags Encrypted in both cases.
    if let Some(b) = bytes(&dir, "F-04_encrypted.pdf", &mut failures) {
        let opts = lopdf::LoadOptions {
            password: None,
            ..Default::default()
        };
        let plain = PdfDoc::load_mem_with_options(&b, opts).expect("F-04 lazy load ok");
        check!("F-04 is_encrypted on plain load", plain.is_encrypted());
        let pwd = lopdf::LoadOptions {
            password: Some("stitchup-user".into()),
            ..Default::default()
        };
        let doc = PdfDoc::load_mem_with_options(&b, pwd).expect("F-04 password load ok");
        check!("F-04 was_encrypted after password load", doc.was_encrypted());
        let pages = collect_pages(&doc).map(|p| p.len()).unwrap_or(0);
        check!("F-04 opens with user password", pages == 1);
        let _ = &pages;
    }

    // F-05 acroform: 1 page, has annotations (widget annots present).
    let f05 = bytes(&dir, "F-05_acroform.pdf", &mut failures);
    if let Some(b) = f05 {
        if let Ok(d) = open_ok(&dir, "F-05_acroform.pdf", b, &mut failures) {
            let pages = collect_pages(&d).expect("collect_pages on F-05");
            check!("F-05 has page", pages.len() == 1);
            check!("F-05 page has annots", pages[0].has_annotations);
        }
    }

    // F-06 annotations: 1 page with a Text annotation.
    let f06 = bytes(&dir, "F-06_annotations.pdf", &mut failures);
    if let Some(b) = f06 {
        if let Ok(d) = open_ok(&dir, "F-06_annotations.pdf", b, &mut failures) {
            let pages = collect_pages(&d).expect("collect_pages on F-06");
            check!("F-06 page has annots", pages[0].has_annotations);
        }
    }

    // F-08 malformed: must not panic; load may succeed (lenient xref) or fail gracefully.
    if let Some(b) = bytes(&dir, "F-08_malformed.pdf", &mut failures) {
        let res = std::panic::catch_unwind(|| PdfDoc::load_mem(&b));
        check!("F-08 no panic on malformed", res.is_ok());
        if let Ok(Ok(doc)) = res {
            let _ = collect_pages(&doc); // must not panic either
            println!("INFO F-08 parsed leniently (recovered) — acceptable");
        }
    }

    // F-10 attachments: 1 page, opens.
    let f10 = bytes(&dir, "F-10_attachments.pdf", &mut failures);
    if let Some(b) = f10 {
        if let Ok(d) = open_ok(&dir, "F-10_attachments.pdf", b, &mut failures) {
            check!("F-10 page count", collect_pages(&d).map(|p| p.len()).unwrap_or(0) == 1);
        }
    }

    // F-11 large: exactly 200 pages.
    let f11 = bytes(&dir, "F-11_large_200p.pdf", &mut failures);
    if let Some(b) = f11 {
        if let Ok(d) = open_ok(&dir, "F-11_large_200p.pdf", b, &mut failures) {
            check!("F-11 page count == 200", collect_pages(&d).map(|p| p.len()).unwrap_or(0) == 200);
        }
    }

    // F-12 blank: 1 page, zero-size content stream tolerated.
    let f12 = bytes(&dir, "F-12_blank.pdf", &mut failures);
    if let Some(b) = f12 {
        if let Ok(d) = open_ok(&dir, "F-12_blank.pdf", b, &mut failures) {
            check!("F-12 page count == 1", collect_pages(&d).map(|p| p.len()).unwrap_or(0) == 1);
        }
    }

    // F-13 xfa: 1 page, opens (warning surface is a core concern, not parse).
    let f13 = bytes(&dir, "F-13_xfa.pdf", &mut failures);
    if let Some(b) = f13 {
        if let Ok(d) = open_ok(&dir, "F-13_xfa.pdf", b, &mut failures) {
            check!("F-13 page count == 1", collect_pages(&d).map(|p| p.len()).unwrap_or(0) == 1);
        }
    }

    // F-14 pdfa-lite: 1 page, opens, Info metadata readable.
    let f14 = bytes(&dir, "F-14_pdfa_lite.pdf", &mut failures);
    if let Some(b) = f14 {
        if let Ok(d) = open_ok(&dir, "F-14_pdfa_lite.pdf", b, &mut failures) {
            let meta = stitchup_core::read_metadata(&d);
            check!("F-14 title metadata", meta.title.as_deref() == Some("PDF/A-lite fixture"));
            check!("F-14 page count == 1", collect_pages(&d).map(|p| p.len()).unwrap_or(0) == 1);
        }
    }

    println!("\n=== verify fixtures: {} failure(s) ===", failures);
    std::process::exit(if failures == 0 { 0 } else { 1 });
}

fn bytes(dir: &PathBuf, name: &str, failures: &mut usize) -> Option<Vec<u8>> {
    let path = dir.join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(e) => {
            *failures += 1;
            println!("FAIL  {name}: missing on disk ({e})");
            None
        }
    }
}

fn open_ok(dir: &PathBuf, name: &str, b: Vec<u8>, failures: &mut usize) -> Result<PdfDoc, ()> {
    match PdfDoc::load_mem(&b) {
        Ok(d) => Ok(d),
        Err(e) => {
            *failures += 1;
            println!("FAIL  {name}: failed to load ({e})");
            let _ = dir;
            Err(())
        }
    }
}

/// Rotations helper kept for parity assertions (also used by tests).
#[allow(dead_code)]
fn _record_rotation(page: &lopdf::Object) -> i64 {
    page_rotation(page)
}