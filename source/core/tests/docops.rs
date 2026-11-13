//! Whole-document operations: new, watermark, crop, encrypt, compress, attachments.

use lopdf::{Document as PdfDoc, LoadOptions, Object};
use stitchup_core::collect_pages;
use stitchup_core::docops::*;

fn roundtrip(doc: &mut PdfDoc) -> PdfDoc {
    let mut data = Vec::new();
    doc.save_to(&mut data).unwrap();
    PdfDoc::load_mem(&data).unwrap()
}

#[test]
fn new_blank_is_a_valid_one_page_document() {
    let mut d = new_blank(595.0, 842.0).unwrap();
    let back = roundtrip(&mut d);
    let p = collect_pages(&back).unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].width_pt.round(), p[0].height_pt.round()), (595.0, 842.0));
    assert!(new_blank(0.0, 10.0).is_err());
}

fn spec(text: &str, placement: Placement, pages: Vec<u32>) -> WatermarkSpec {
    WatermarkSpec { text: text.into(), size: 48.0, placement, opacity: 0.5, color: [0.5, 0.5, 0.5], pages }
}

#[test]
fn watermark_is_drawn_on_every_page_and_keeps_original_text() {
    let mut d = new_blank(612.0, 792.0).unwrap();
    // second page, rotated
    let mut other = new_blank(400.0, 600.0).unwrap();
    other.get_object_mut(*other.get_pages().get(&1).unwrap()).unwrap().as_dict_mut().unwrap().set("Rotate", 90);
    stitchup_core::merge_doc(&mut d, &other).unwrap();
    assert_eq!(watermark(&mut d, &spec("CONFIDENTIAL", Placement::Center, vec![])).unwrap(), 2);
    assert_eq!(watermark(&mut d, &spec("Draft", Placement::Tiled, vec![2])).unwrap(), 1);
    assert_eq!(watermark(&mut d, &spec("Top", Placement::Top, vec![1])).unwrap(), 1);
    let back = roundtrip(&mut d);
    for n in [1, 2] {
        assert!(back.extract_text(&[n]).unwrap().contains("CONFIDENTIAL"), "page {n}");
    }
    assert!(back.extract_text(&[2]).unwrap().matches("Draft").count() > 3, "tiled repeats");
    assert!(watermark(&mut d, &spec("  ", Placement::Center, vec![])).is_err());
}

#[test]
fn crop_box_changes_the_visible_page_size() {
    let mut d = new_blank(612.0, 792.0).unwrap();
    set_crop_box(&mut d, 1, [50.0, 60.0, 350.0, 460.0]).unwrap();
    let p = collect_pages(&d).unwrap();
    assert_eq!((p[0].width_pt.round(), p[0].height_pt.round()), (300.0, 400.0));
    // Clamped to the MediaBox; degenerate boxes refused.
    set_crop_box(&mut d, 1, [-100.0, -100.0, 9999.0, 9999.0]).unwrap();
    assert_eq!(collect_pages(&d).unwrap()[0].width_pt.round(), 612.0);
    assert!(set_crop_box(&mut d, 1, [10.0, 10.0, 11.0, 11.0]).is_err());
}

#[test]
fn encrypted_copy_needs_the_password_and_leaves_the_original_open() {
    let mut d = new_blank(612.0, 792.0).unwrap();
    watermark(&mut d, &spec("SECRET", Placement::Center, vec![])).unwrap();
    let mut enc = encrypted_copy(&d, "user-pw", "owner-pw").unwrap();
    let mut data = Vec::new();
    enc.save_to(&mut data).unwrap();
    assert!(String::from_utf8_lossy(&data).contains("/Encrypt"));
    // Without the password the text can't be read.
    let no_pw = PdfDoc::load_mem(&data);
    assert!(no_pw.map(|d| d.is_encrypted()).unwrap_or(true));
    let opened = PdfDoc::load_mem_with_options(&data, LoadOptions { password: Some("user-pw".into()), ..Default::default() }).unwrap();
    assert!(opened.extract_text(&[1]).unwrap().contains("SECRET"));
    // Original untouched.
    assert!(!d.is_encrypted());
    assert!(encrypted_copy(&d, "", "").is_err());
}

#[test]
fn compressed_copy_is_smaller_and_equivalent() {
    let mut d = new_blank(612.0, 792.0).unwrap();
    for _ in 0..5 {
        watermark(&mut d, &spec("Repeated watermark text", Placement::Tiled, vec![])).unwrap();
    }
    let mut plain = Vec::new();
    d.save_to(&mut plain).unwrap();
    let small = compressed_bytes(&d).unwrap();
    assert!(small.len() < plain.len(), "{} >= {}", small.len(), plain.len());
    let back = PdfDoc::load_mem(&small).unwrap();
    assert!(back.extract_text(&[1]).unwrap().contains("Repeated"));
}

#[test]
fn attachments_are_listed_and_extracted() {
    let f = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/generated/F-10_attachments.pdf");
    let doc = PdfDoc::load(f).unwrap();
    let list = list_attachments(&doc);
    assert!(!list.is_empty(), "fixture declares an attachment");
    let bytes = attachment_bytes(&doc, 0).unwrap();
    assert!(!bytes.is_empty());
    assert!(list[0].size == 0 || list[0].size == bytes.len() as u64);
    assert!(attachment_bytes(&doc, 99).is_err());
    let _ = Object::Null;
}
