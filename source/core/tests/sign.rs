//! Digital signatures, verified independently with the system's OpenSSL.

use std::process::Command;

use lopdf::Document as PdfDoc;
use stitchup_core::docops::new_blank;
use stitchup_core::sign::{list_signatures, sign_bytes, sign_document, SignSpec};

fn spec(visible: bool) -> SignSpec {
    SignSpec {
        name: "Jan Kowalski".into(),
        reason: "Zatwierdzam".into(),
        location: "Łódź".into(),
        contact: String::new(),
        page: Some(1),
        rect: visible.then_some([350.0, 40.0, 570.0, 110.0]),
    }
}

/// (signed data, DER signature) for the `nth` signature (by /ByteRange order).
fn parts_n(pdf: &[u8], nth: usize) -> (Vec<u8>, Vec<u8>) {
    let s = String::from_utf8_lossy(pdf);
    let at = s.match_indices("/ByteRange").nth(nth).unwrap().0;
    let nums: Vec<usize> = s[at..][s[at..].find('[').unwrap() + 1..s[at..].find(']').unwrap()]
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let (a, b, c, d) = (nums[0], nums[1], nums[2], nums[3]);
    assert_eq!(a, 0);
    assert!(c + d <= pdf.len(), "byte range within the file");
    let mut data = pdf[a..a + b].to_vec();
    data.extend_from_slice(&pdf[c..c + d]);
    let hex = std::str::from_utf8(&pdf[b + 1..c - 1]).unwrap();
    let mut der: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    // DER length of the outer SEQUENCE (long form) → trim the zero padding.
    let len = match der[1] {
        n if n < 0x80 => 2 + n as usize,
        n => {
            let k = (n & 0x7f) as usize;
            2 + k + der[2..2 + k].iter().fold(0usize, |acc, b| acc << 8 | *b as usize)
        }
    };
    der.truncate(len);
    (data, der)
}

fn parts(pdf: &[u8]) -> (Vec<u8>, Vec<u8>) {
    parts_n(pdf, 0)
}

fn openssl_verifies(data: &[u8], der: &[u8]) -> bool {
    let dir = std::env::temp_dir().join(format!("stitchup-sign-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("data.bin"), data).unwrap();
    std::fs::write(dir.join("sig.der"), der).unwrap();
    let out = Command::new("openssl")
        .args(["smime", "-verify", "-binary", "-inform", "DER", "-noverify", "-purpose", "any"])
        .arg("-in").arg(dir.join("sig.der"))
        .arg("-content").arg(dir.join("data.bin"))
        .args(["-out", "/dev/null"])
        .output()
        .expect("openssl available");
    out.status.success()
}

#[test]
fn signed_pdf_verifies_with_openssl_and_tampering_breaks_it() {
    let doc = new_blank(612.0, 792.0).unwrap();
    let signed = sign_document(&doc, &spec(true)).unwrap();
    let (data, der) = parts(&signed);
    assert!(openssl_verifies(&data, &der), "valid signature rejected");

    let mut tampered = data.clone();
    let i = tampered.len() / 2;
    tampered[i] ^= 0x01;
    assert!(!openssl_verifies(&tampered, &der), "tampered data accepted");

    // Still a valid PDF with one signature named after the signer.
    let back = PdfDoc::load_mem(&signed).unwrap();
    let sigs = list_signatures(&back);
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].0, "Jan Kowalski");
    assert!(sigs[0].1.starts_with("D:20"));
}

#[test]
fn invisible_signature_and_second_signature_get_unique_fields() {
    let doc = new_blank(612.0, 792.0).unwrap();
    let once = sign_document(&doc, &spec(false)).unwrap();
    let back = PdfDoc::load_mem(&once).unwrap();
    let twice = sign_document(&back, &spec(true)).unwrap();
    let fields = stitchup_core::docops::list_field_names(&PdfDoc::load_mem(&twice).unwrap());
    assert!(fields.contains(&"Signature1".to_string()) && fields.contains(&"Signature2".to_string()), "{fields:?}");
    assert!(sign_document(&doc, &SignSpec { name: "  ".into(), ..spec(false) }).is_err());
}

#[test]
fn second_signature_is_incremental_and_both_still_verify() {
    let doc = new_blank(612.0, 792.0).unwrap();
    let first = sign_document(&doc, &spec(true)).unwrap();
    let second = sign_bytes(first.clone(), &SignSpec { name: "Anna Nowak".into(), rect: Some([40.0, 40.0, 260.0, 100.0]), ..spec(true) }).unwrap();
    assert!(second.starts_with(&first), "the first revision is kept byte-for-byte");
    let (d1, s1) = parts_n(&second, 0);
    let (d2, s2) = parts_n(&second, 1);
    assert!(openssl_verifies(&d1, &s1), "first signature still valid");
    assert!(openssl_verifies(&d2, &s2), "second signature valid");
    assert_eq!(d2.len() + 2 * 12_000 + 2, second.len(), "second signature covers the whole file");
    let back = PdfDoc::load_mem(&second).unwrap();
    let names: Vec<String> = list_signatures(&back).into_iter().map(|s| s.0).collect();
    assert!(names.contains(&"Jan Kowalski".into()) && names.contains(&"Anna Nowak".into()), "{names:?}");
}

#[test]
fn edited_signed_pdf_saved_incrementally_keeps_the_first_signature_and_can_be_signed_again() {
    let doc = new_blank(612.0, 792.0).unwrap();
    let first = sign_document(&doc, &spec(true)).unwrap();
    // Open (as the app does), edit: add a sticky note.
    let mut d = PdfDoc::load_mem(&first).unwrap();
    let _ = stitchup_core::ops::flatten_page_tree(&mut d);
    stitchup_core::annots::add(&mut d, 1, &stitchup_core::annots::AnnotSpec::Note { at: [300.0, 600.0], text: "Checked".into(), color: [1.0, 0.85, 0.2] }).unwrap();
    let edited = stitchup_core::docops::incremental_update(&first, &d).unwrap();
    assert!(edited.starts_with(&first) && edited.len() > first.len(), "appended, not rewritten");
    let (d1, s1) = parts_n(&edited, 0);
    assert!(openssl_verifies(&d1, &s1), "first signature still valid after the edit");
    let back = PdfDoc::load_mem(&edited).unwrap();
    let page = *back.get_pages().get(&1).unwrap();
    let annots = back.get_page_annotations(page).unwrap();
    assert!(annots.iter().any(|a| a.get(b"Subtype").and_then(|s| s.as_name()).ok() == Some(b"Text".as_slice())), "the note is in the saved file");
    // Second signer on top of the edit.
    let both = sign_bytes(edited.clone(), &SignSpec { name: "Anna Nowak".into(), rect: Some([40.0, 40.0, 260.0, 100.0]), ..spec(true) }).unwrap();
    let (d1, s1) = parts_n(&both, 0);
    let (d2, s2) = parts_n(&both, 1);
    assert!(openssl_verifies(&d1, &s1) && openssl_verifies(&d2, &s2), "both signatures valid");
    // Unchanged document → same bytes.
    let same = PdfDoc::load_mem(&first).unwrap();
    assert_eq!(stitchup_core::docops::incremental_update(&first, &same).unwrap(), first);
}
