use lopdf::content::Content;
use stitchup_core::docops::new_blank;
use stitchup_core::ocr::{add_text_layer, OcrLine};

fn line(text: &str, x: f64, y: f64, w: f64, h: f64) -> OcrLine {
    OcrLine { text: text.into(), x, y, w, h, confidence: 1.0 }
}

/// (operator, operands as f64) for the page's content.
fn ops(doc: &lopdf::Document) -> Vec<(String, Vec<f64>)> {
    let page = *doc.get_pages().get(&1).unwrap();
    let c = Content::decode(&doc.get_page_content(page)).unwrap();
    c.operations.into_iter().map(|o| (o.operator, o.operands.iter().filter_map(|v| v.as_float().ok().map(|f| f as f64)).collect())).collect()
}

#[test]
fn invisible_text_layer_is_extractable_and_placed() {
    let mut doc = new_blank(612.0, 792.0).unwrap();
    let n = add_text_layer(&mut doc, 1, 0, &[line("Invoice 2026", 0.1, 0.8, 0.4, 0.03), line("Zażółć gęślą jaźń", 0.1, 0.5, 0.5, 0.03)]).unwrap();
    assert_eq!(n, 2);
    let mut bytes = vec![];
    doc.save_to(&mut bytes).unwrap();
    let doc = lopdf::Document::load_mem(&bytes).unwrap();
    let text = doc.extract_text(&[1]).unwrap();
    assert!(text.contains("Invoice 2026"), "{text}");
    let o = ops(&doc);
    assert!(o.iter().any(|(op, a)| op == "Tr" && a == &[3.0]), "render mode 3 (invisible)");
    let tm = &o.iter().find(|(op, _)| op == "Tm").unwrap().1;
    assert!((tm[4] - 61.2).abs() < 1.0 && (tm[5] - 792.0 * 0.806).abs() < 2.0, "placed at {tm:?}");
}

#[test]
fn rotated_page_maps_back_to_user_space() {
    let mut doc = new_blank(612.0, 792.0).unwrap();
    // Displayed rotated 90° clockwise: the displayed top-left is the
    // user-space bottom-left corner.
    add_text_layer(&mut doc, 1, 90, &[line("Sideways", 0.0, 0.9, 0.3, 0.05)]).unwrap();
    let tm = ops(&doc).into_iter().find(|(op, _)| op == "Tm").unwrap().1;
    assert!(tm[0].abs() < 1e-3 && (tm[1] - 1.0).abs() < 1e-3, "text runs up the page: {tm:?}");
    assert!(tm[4] < 612.0 * 0.15 && tm[5] < 10.0, "near the bottom-left: {tm:?}");
}
