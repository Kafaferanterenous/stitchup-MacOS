//! Digital signatures, verified independently with the system's OpenSSL.

use std::process::Command;

use lopdf::Document as PdfDoc;
use stitchup_core::docops::new_blank;
use stitchup_core::sign::{list_signatures, sign_document, SignSpec};

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

/// (signed data, DER signature) extracted using /ByteRange and /Contents.
fn parts(pdf: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let s = String::from_utf8_lossy(pdf);
    let at = s.find("/ByteRange").unwrap();
    let nums: Vec<usize> = s[at..][s[at..].find('[').unwrap() + 1..s[at..].find(']').unwrap()]
        .split_whitespace()
        .map(|n| n.parse().unwrap())
        .collect();
    let (a, b, c, d) = (nums[0], nums[1], nums[2], nums[3]);
    assert_eq!(a, 0);
    assert_eq!(c + d, pdf.len(), "byte range must cover to the end of file");
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
