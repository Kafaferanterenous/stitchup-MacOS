//! Writes a sample PDF with shaped scripts when STITCHUP_DUMP is set (visual check).
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as PdfDoc, Object, Stream, StringFormat};

#[test]
fn dump_shaped_sample() {
    let Ok(out) = std::env::var("STITCHUP_DUMP") else { return };
    let mut doc = PdfDoc::with_version("1.5");
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let lines = ["Line one", "Line two", "Line three", "Line four", "Line five"];
    let mut ops = vec![Operation::new("BT", vec![]), Operation::new("Tf", vec!["F1".into(), 20.into()]), Operation::new("Td", vec![72.into(), 700.into()])];
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            ops.push(Operation::new("Td", vec![0.into(), (-60).into()]));
        }
        ops.push(Operation::new("Tj", vec![Object::String(l.as_bytes().to_vec(), StringFormat::Literal)]));
    }
    ops.push(Operation::new("ET", vec![]));
    let content = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
    let pages = doc.new_object_id();
    let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } });
    doc.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", cat);
    let texts = ["مرحبا بالعالم — العربية", "שלום עולם", "नमस्ते दुनिया — क्षत्रिय", "สวัสดีชาวโลก", "Привет, Ελλάδα, 你好世界"];
    for (i, (l, t)) in lines.iter().zip(texts).enumerate() {
        let req = stitchup_core::textedit::EditRequest { page: 1, x: 72.0, y: 700.0 - 60.0 * i as f64, width: 80.0, font_size: 20.0, old_text: (*l).into(), new_text: t.into() };
        let n = stitchup_core::textedit::edit_text(&mut doc, &req).unwrap();
        eprintln!("{t}: {}", n.note);
    }
    doc.save(out).unwrap();
}

#[test]
fn dump_signature_sample() {
    let Ok(out) = std::env::var("STITCHUP_SIGN_DUMP") else { return };
    let doc = stitchup_core::docops::new_blank(612.0, 792.0).unwrap();
    let spec = stitchup_core::sign::SignSpec {
        name: "Пётр Иванов · 王小明".into(),
        reason: "Одобрено — Approved".into(),
        location: "Αθήνα".into(),
        contact: String::new(),
        page: Some(1),
        rect: Some([72.0, 600.0, 372.0, 700.0]),
    };
    std::fs::write(out, stitchup_core::sign::sign_document(&doc, &spec).unwrap()).unwrap();
}
