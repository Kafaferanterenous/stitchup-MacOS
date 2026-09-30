use lopdf::{Document as PdfDoc, LoadOptions};
#[test]
fn dbg_encrypted_save() {
    let bytes = std::fs::read("/Users/groot/opencode_mac/007_2026_09_10_Stitchup/fixtures/generated/F-04_encrypted.pdf").unwrap();
    let mut doc = PdfDoc::load_mem_with_options(&bytes, LoadOptions { password: Some("stitchup-user".into()), ..Default::default() }).unwrap();
    if doc.is_encrypted() { doc.decrypt("stitchup-user").unwrap(); }
    println!("[dbg] is_encrypted after decrypt = {}", doc.is_encrypted());
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let s = String::from_utf8_lossy(&out);
    println!("[dbg] save bytes len = {} /Encrypt refs = {}", out.len(), s.matches("Encrypt").count());
    // try to reopen for real:
    match PdfDoc::load_mem(&out) {
        Ok(rd) => println!("[dbg] reopened clean: pages_enc={}", rd.is_encrypted()),
        Err(e) => println!("[dbg] reopen FAILED: {e}"),
    }
}
