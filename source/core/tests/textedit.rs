//! In-place text editing + nested page trees.

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as PdfDoc, Object, Stream, StringFormat};
use stitchup_core::textedit::{edit_text, EditMethod, EditRequest};
use stitchup_core::{collect_pages, delete_pages, extract, reorder, split};

fn s(t: &str) -> Object {
    Object::String(t.as_bytes().to_vec(), StringFormat::Literal)
}

/// One-page doc. `font` is the /F1 font dict; `ops` the page content.
fn doc_with(font: lopdf::Dictionary, ops: Vec<Operation>) -> PdfDoc {
    let mut doc = PdfDoc::with_version("1.5");
    let font_id = doc.add_object(font);
    let content = Content { operations: ops }.encode().unwrap();
    let content_id = doc.add_object(Stream::new(dictionary! {}, content));
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font_id } },
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }),
    );
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", cat);
    doc
}

fn helvetica() -> lopdf::Dictionary {
    dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" }
}

fn line(x: i64, y: i64, show: Operation) -> Vec<Operation> {
    vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Td", vec![x.into(), y.into()]),
        show,
        Operation::new("ET", vec![]),
    ]
}

fn req(x: f64, y: f64, old: &str, new: &str) -> EditRequest {
    EditRequest { page: 1, x, y, width: 80.0, font_size: 12.0, old_text: old.into(), new_text: new.into() }
}

fn roundtrip_text(doc: &mut PdfDoc) -> String {
    let mut data = Vec::new();
    doc.save_to(&mut data).unwrap();
    let back = PdfDoc::load_mem(&data).unwrap();
    back.extract_text(&[1]).unwrap()
}

#[test]
fn standard_font_is_edited_in_place_and_old_text_is_gone() {
    let mut ops = line(72, 700, Operation::new("Tj", vec![s("Hello World")]));
    ops.extend(line(72, 650, Operation::new("Tj", vec![s("Second line")])));
    let mut doc = doc_with(helvetica(), ops);
    let out = edit_text(&mut doc, &req(72.0, 700.0, "Hello World", "Hello Café")).unwrap();
    assert_eq!(out.method, EditMethod::InPlace, "{}", out.note);
    let text = roundtrip_text(&mut doc);
    assert!(text.contains("Hello Café"), "{text}");
    assert!(!text.contains("World"), "old text must be removed: {text}");
    assert!(text.contains("Second line"));
}

#[test]
fn picks_the_run_at_the_clicked_position() {
    let mut ops = line(72, 700, Operation::new("Tj", vec![s("Total")]));
    ops.extend(line(72, 600, Operation::new("Tj", vec![s("Total")])));
    let mut doc = doc_with(helvetica(), ops);
    edit_text(&mut doc, &req(72.0, 600.0, "Total", "Sum")).unwrap();
    let text = roundtrip_text(&mut doc);
    assert!(text.contains("Total") && text.contains("Sum"), "{text}");
    // Order is drawing order: first line untouched.
    assert!(text.find("Total").unwrap() < text.find("Sum").unwrap(), "{text}");
}

#[test]
fn tj_array_with_kerning_matches_and_is_replaced() {
    let tj = Operation::new("TJ", vec![Object::Array(vec![s("Inv"), (-40).into(), s("oice"), (-250).into(), s("No")])]);
    let mut doc = doc_with(helvetica(), line(72, 700, tj));
    let out = edit_text(&mut doc, &req(72.0, 700.0, "Invoice No", "Receipt No")).unwrap();
    assert_eq!(out.method, EditMethod::InPlace);
    let text = roundtrip_text(&mut doc);
    assert!(text.contains("Receipt No") && !text.contains("Invoice"), "{text}");
}

#[test]
fn subset_font_without_glyphs_falls_back_to_standard_family_and_removes_original() {
    // Embedded (FontFile2) subset: only the glyphs drawn on the page exist.
    let mut d2 = doc_with(helvetica(), line(72, 700, Operation::new("Tj", vec![s("Hello")])));
    let ff = d2.add_object(Stream::new(dictionary! {}, vec![0u8; 4]));
    let fd = d2.add_object(dictionary! { "Type" => "FontDescriptor", "FontName" => "ABCDEF+Georgia-Bold", "FontFile2" => ff });
    let font_id = d2.objects.iter().find(|(_, o)| o.as_dict().map(|d| d.has(b"BaseFont")).unwrap_or(false)).map(|(k, _)| *k).unwrap();
    d2.objects.insert(font_id, Object::Dictionary(dictionary! {
        "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "ABCDEF+Georgia-Bold",
        "Encoding" => "WinAnsiEncoding", "FontDescriptor" => fd,
        "FirstChar" => 32, "Widths" => vec![Object::Integer(250); 95],
    }));
    // "Hell" glyphs exist (subset), "y" does not.
    let out = edit_text(&mut d2, &req(72.0, 700.0, "Hello", "Holly")).unwrap();
    assert_eq!(out.method, EditMethod::Replaced, "{}", out.note);
    assert!(out.note.contains("Times-Bold"), "serif bold family kept: {}", out.note);
    // Re-using only existing glyphs stays in the original font.
    let out2 = edit_text(&mut d2, &req(72.0, 700.0, "Holly", "Hole")).unwrap();
    assert_eq!(out2.method, EditMethod::InPlace, "{}", out2.note);
}

#[test]
fn unknown_run_falls_back_to_overlay() {
    let mut doc = doc_with(helvetica(), line(72, 700, Operation::new("Tj", vec![s("Hello")])));
    let out = edit_text(&mut doc, &req(300.0, 300.0, "Nowhere", "x")).unwrap();
    assert_eq!(out.method, EditMethod::Overlay);
}

#[test]
fn substring_of_a_longer_op_is_replaced() {
    let mut doc = doc_with(helvetica(), line(72, 700, Operation::new("Tj", vec![s("Name: John Smith")])));
    // Renderer split the op; the clicked item starts after "Name: ".
    let x = 72.0 + 0.0; // origin search is by position along the baseline
    let mut r = req(x, 700.0, "John Smith", "Jane Doe");
    r.x = 72.0 + 12.0 * 0.55 * 6.0; // approx advance of "Name: "
    let out = edit_text(&mut doc, &r).unwrap();
    assert_eq!(out.method, EditMethod::InPlace, "{}", out.note);
    let text = roundtrip_text(&mut doc);
    assert!(text.contains("Name: Jane Doe"), "{text}");
}

/// Two-level tree with MediaBox/Resources inherited from the intermediate node.
fn nested_doc(n: usize) -> PdfDoc {
    let mut doc = PdfDoc::with_version("1.5");
    let font_id = doc.add_object(helvetica());
    let root = doc.new_object_id();
    let mid_a = doc.new_object_id();
    let mid_b = doc.new_object_id();
    let mut kids_a = vec![];
    let mut kids_b = vec![];
    for i in 0..n {
        let parent = if i < n / 2 { mid_a } else { mid_b };
        let c = Content { operations: line(72, 700, Operation::new("Tj", vec![s(&format!("Page {}", i + 1))])) };
        let cid = doc.add_object(Stream::new(dictionary! {}, c.encode().unwrap()));
        let pid = doc.add_object(dictionary! { "Type" => "Page", "Parent" => parent, "Contents" => cid });
        if i < n / 2 { kids_a.push(pid.into()) } else { kids_b.push(pid.into()) }
    }
    let res = dictionary! { "Font" => dictionary! { "F1" => font_id } };
    let la = kids_a.len() as i64;
    let lb = kids_b.len() as i64;
    doc.objects.insert(mid_a, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Parent" => root, "Kids" => kids_a, "Count" => la,
        "MediaBox" => vec![0.into(), 0.into(), 420.into(), 594.into()], "Resources" => res.clone(),
    }));
    doc.objects.insert(mid_b, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Parent" => root, "Kids" => kids_b, "Count" => lb,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => res, "Rotate" => 90,
    }));
    doc.objects.insert(root, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Kids" => vec![mid_a.into(), mid_b.into()], "Count" => n as i64,
    }));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    doc.trailer.set("Root", cat);
    doc
}

#[test]
fn nested_page_tree_opens_with_inherited_sizes() {
    let doc = nested_doc(4);
    let pages = collect_pages(&doc).unwrap();
    assert_eq!(pages.len(), 4);
    assert_eq!((pages[0].width_pt, pages[0].height_pt, pages[0].rotation), (420.0, 594.0, 0));
    assert_eq!((pages[3].width_pt, pages[3].height_pt, pages[3].rotation), (612.0, 792.0, 90));
}

#[test]
fn nested_page_tree_reorder_and_delete_act_on_pages() {
    let mut doc = nested_doc(4);
    reorder(&mut doc, &[4, 3, 2, 1]).unwrap();
    let pages = collect_pages(&doc).unwrap();
    assert_eq!(pages.len(), 4);
    assert_eq!(pages[0].rotation, 90, "old page 4 keeps its inherited rotation");
    assert_eq!(pages[3].width_pt, 420.0, "old page 1 keeps its inherited size");
    let mut data = Vec::new();
    doc.save_to(&mut data).unwrap();
    let back = PdfDoc::load_mem(&data).unwrap();
    assert!(back.extract_text(&[1]).unwrap().contains("Page 4"));

    let mut doc = nested_doc(4);
    delete_pages(&mut doc, &[2]).unwrap();
    assert_eq!(collect_pages(&doc).unwrap().len(), 3, "delete removes ONE page, not a group");
}

#[test]
fn extract_and_split_produce_small_standalone_docs() {
    let doc = nested_doc(6);
    let mut out = extract(&doc, &[2, 3]).unwrap();
    assert_eq!(collect_pages(&out).unwrap().len(), 2);
    let mut data = Vec::new();
    out.save_to(&mut data).unwrap();
    let back = PdfDoc::load_mem(&data).unwrap();
    assert!(back.extract_text(&[1]).unwrap().contains("Page 2"));
    // Pruned: no leftover content streams from the other 4 pages.
    let streams = out.objects.values().filter(|o| o.as_stream().is_ok()).count();
    assert_eq!(streams, 2, "only the kept pages' content streams remain");

    let parts = split(&doc, 4).unwrap();
    assert_eq!(parts.iter().map(|d| d.get_pages().len()).collect::<Vec<_>>(), vec![4, 2]);
}

#[test]
fn polish_letters_survive_the_standard_font_fallback() {
    // Helvetica (WinAnsi) can't encode ą/ł…, so the edit falls back — and the
    // fallback must keep every letter, not turn them into '?'.
    let mut doc = doc_with(helvetica(), line(72, 700, Operation::new("Tj", vec![s("Hello")])));
    let out = edit_text(&mut doc, &req(72.0, 700.0, "Hello", "Zażółć gęślą jaźń ŁÓDŹ")).unwrap();
    assert!(!out.note.contains("unsupported"), "{}", out.note);
    let text = roundtrip_text(&mut doc);
    assert!(text.contains("Zażółć gęślą jaźń ŁÓDŹ"), "{text}");
    assert!(!text.contains("Hello"), "{text}");
}

#[test]
fn std14_encoding_reports_scripts_it_cannot_draw() {
    let e = stitchup_core::textedit::std14_encode("Łódź Привет");
    assert_eq!(e.unsupported.iter().collect::<String>(), "Привет");
    assert!(e.diffs.iter().any(|(_, n)| *n == "Lslash"));
}
