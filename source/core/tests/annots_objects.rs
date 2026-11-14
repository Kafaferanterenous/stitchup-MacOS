//! Annotations, content objects and true redaction.

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as PdfDoc, Object, Stream, StringFormat};
use stitchup_core::annots::{self, AnnotSpec};
use stitchup_core::objects;

fn s(t: &str) -> Object {
    Object::String(t.as_bytes().to_vec(), StringFormat::Literal)
}

/// Page with two text lines and a 100x80 image at (300, 500).
fn doc() -> PdfDoc {
    let mut doc = PdfDoc::with_version("1.5");
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let img = doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
        vec![128],
    ));
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Td", vec![72.into(), 700.into()]),
        Operation::new("Tj", vec![s("SECRET 1234")]),
        Operation::new("Td", vec![0.into(), (-20).into()]),
        Operation::new("Tj", vec![s("Public line")]),
        Operation::new("ET", vec![]),
        Operation::new("q", vec![]),
        Operation::new("cm", vec![100.into(), 0.into(), 0.into(), 80.into(), 300.into(), 500.into()]),
        Operation::new("Do", vec!["Im1".into()]),
        Operation::new("Q", vec![]),
    ];
    let content = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
    let pages = doc.new_object_id();
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages, "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font }, "XObject" => dictionary! { "Im1" => img } },
    });
    doc.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
    doc.trailer.set("Root", cat);
    doc
}

fn reload(d: &mut PdfDoc) -> PdfDoc {
    let mut b = Vec::new();
    d.save_to(&mut b).unwrap();
    PdfDoc::load_mem(&b).unwrap()
}

const RED: [f64; 3] = [0.9, 0.1, 0.1];

#[test]
fn every_annotation_kind_is_created_with_an_appearance() {
    let mut d = doc();
    let specs = vec![
        AnnotSpec::Highlight { boxes: vec![[72.0, 698.0, 160.0, 712.0]], color: [1.0, 0.9, 0.0] },
        AnnotSpec::Underline { boxes: vec![[72.0, 678.0, 140.0, 692.0]], color: RED },
        AnnotSpec::StrikeOut { boxes: vec![[72.0, 678.0, 140.0, 692.0]], color: RED },
        AnnotSpec::Note { at: [500.0, 750.0], text: "Sprawdź to — ok?".into(), color: [1.0, 0.85, 0.2] },
        AnnotSpec::TextBox { rect: [72.0, 400.0, 300.0, 460.0], text: "Free text that wraps across lines in the box".into(), size: 12.0, color: [0.0, 0.0, 0.0] },
        AnnotSpec::Square { rect: [72.0, 300.0, 200.0, 380.0], color: RED, fill: None, width: 2.0 },
        AnnotSpec::Circle { rect: [220.0, 300.0, 340.0, 380.0], color: RED, fill: Some([1.0, 1.0, 0.8]), width: 1.5 },
        AnnotSpec::Line { from: [72.0, 250.0], to: [300.0, 270.0], color: RED, width: 2.0, arrow: true },
        AnnotSpec::Ink { strokes: vec![vec![[80.0, 200.0], [90.0, 210.0], [100.0, 205.0]]], color: [0.0, 0.0, 1.0], width: 2.0 },
        AnnotSpec::Stamp { rect: [350.0, 300.0, 550.0, 360.0], label: "Approved".into(), color: [0.1, 0.6, 0.2] },
        AnnotSpec::Link { rect: [72.0, 698.0, 160.0, 712.0], uri: Some("example.com".into()), page: None },
        AnnotSpec::Link { rect: [72.0, 678.0, 160.0, 692.0], uri: None, page: Some(1) },
    ];
    for spec in &specs {
        annots::add(&mut d, 1, spec).unwrap();
    }
    let back = reload(&mut d);
    let list = annots::rects(&back, 1);
    assert_eq!(list.len(), specs.len());
    let kinds: Vec<&str> = list.iter().map(|(_, k, _)| k.as_str()).collect();
    for k in ["Highlight", "Underline", "StrikeOut", "Text", "FreeText", "Square", "Circle", "Line", "Ink", "Stamp", "Link"] {
        assert!(kinds.contains(&k), "{k} missing: {kinds:?}");
    }
    // Every non-link annotation has a normal appearance stream.
    for (id, kind, _) in &list {
        let a = back.get_object((*id, 0)).unwrap().as_dict().unwrap();
        if kind != "Link" {
            assert!(a.get(b"AP").is_ok(), "{kind} has no /AP");
        }
    }
    let link = list.iter().find(|(_, k, _)| k == "Link").unwrap();
    let a = back.get_object((link.0, 0)).unwrap().as_dict().unwrap();
    let uri = a.get(b"A").unwrap().as_dict().unwrap().get(b"URI").unwrap().as_str().unwrap();
    assert_eq!(uri, b"https://example.com");
}

#[test]
fn annotations_move_and_delete() {
    let mut d = doc();
    let id = annots::add(&mut d, 1, &AnnotSpec::Square { rect: [100.0, 100.0, 200.0, 150.0], color: RED, fill: None, width: 1.0 }).unwrap();
    let new_id = annots::translate(&mut d, 1, id.0, 10.0, -20.0).unwrap();
    let list = annots::rects(&d, 1);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].0, new_id);
    assert_eq!(list[0].2.map(|v| v.round()), [110.0, 80.0, 210.0, 130.0]);
    // The original object is untouched (undo-safe).
    let old = d.get_object(id).unwrap().as_dict().unwrap().get(b"Rect").unwrap().as_array().unwrap()[0].as_float().unwrap();
    assert_eq!(old.round(), 100.0);
    annots::delete(&mut d, 1, new_id).unwrap();
    assert!(annots::rects(&d, 1).is_empty());
    assert!(annots::delete(&mut d, 1, 999).is_err());
}

#[test]
fn content_objects_list_delete_move_recolor_font() {
    let mut d = doc();
    let objs = objects::list(&d, 1).unwrap();
    let secret = objs.iter().find(|o| o.text == "SECRET 1234").unwrap().clone();
    let image = objs.iter().find(|o| o.kind == "image").unwrap().clone();
    assert_eq!(image.rect.map(|v| v.round()), [300.0, 500.0, 400.0, 580.0]);

    objects::translate(&mut d, 1, &[secret.id.clone(), image.id.clone()], 50.0, -10.0).unwrap();
    let after = objects::list(&d, 1).unwrap();
    let moved = after.iter().find(|o| o.text == "SECRET 1234").unwrap();
    assert!((moved.rect[0] - secret.rect[0] - 50.0).abs() < 0.5, "text moved by dx");
    let public = after.iter().find(|o| o.text == "Public line").unwrap();
    assert!((public.rect[0] - 72.0).abs() < 0.5, "following line stays put: {:?}", public.rect);
    let img = after.iter().find(|o| o.kind == "image").unwrap();
    assert_eq!(img.rect.map(|v| v.round()), [350.0, 490.0, 450.0, 570.0]);

    let id = after.iter().find(|o| o.text == "Public line").unwrap().id.clone();
    objects::recolor(&mut d, 1, &[id], RED).unwrap();
    let id = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Public line").unwrap().id;
    objects::set_font(&mut d, 1, &[id], "Times-Bold", Some(16.0)).unwrap();
    let back = reload(&mut d);
    let fonts: Vec<String> = back.get_page_fonts(*back.get_pages().get(&1).unwrap()).unwrap().values()
        .filter_map(|f| f.get(b"BaseFont").ok().and_then(|n| n.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned())).collect();
    assert!(fonts.contains(&"Times-Bold".to_string()), "{fonts:?}");
    assert!(back.extract_text(&[1]).unwrap().contains("Public line"));

    let id = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "SECRET 1234").unwrap().id;
    objects::delete(&mut d, 1, &[id]).unwrap();
    assert!(!reload(&mut d).extract_text(&[1]).unwrap().contains("SECRET"));
}

#[test]
fn redaction_removes_text_and_images_under_the_box_without_shifting_the_rest() {
    let mut d = doc();
    annots::add(&mut d, 1, &AnnotSpec::Note { at: [80.0, 712.0], text: "secret note".into(), color: RED }).unwrap();
    let before = objects::list(&d, 1).unwrap();
    // Cover "SECRET" (first ~6 glyphs at 12pt Helvetica ≈ 48pt wide) and the image.
    let rep = objects::redact(&mut d, 1, [70.0, 695.0, 120.0, 715.0]).unwrap();
    assert!(rep.glyphs_removed >= 6, "{rep:?}");
    assert_eq!(rep.annotations_removed, 1);
    let rep2 = objects::redact(&mut d, 1, [290.0, 490.0, 410.0, 590.0]).unwrap();
    assert_eq!(rep2.images_removed, 1);
    let text = reload(&mut d).extract_text(&[1]).unwrap();
    assert!(!text.contains("SECRET"), "{text}");
    assert!(text.contains("1234") && text.contains("Public line"), "{text}");
    // "1234" kept its exact position.
    let after = objects::list(&d, 1).unwrap();
    let old = before.iter().find(|o| o.text == "SECRET 1234").unwrap().rect;
    let new = after.iter().find(|o| o.text.contains("1234")).unwrap().rect;
    assert!((new[2] - old[2]).abs() < 0.01, "right edge moved: {old:?} → {new:?}");
    assert!(!after.iter().any(|o| o.kind == "image"));
}

#[test]
fn vector_shapes_are_drawn_into_the_page() {
    let mut d = doc();
    for kind in ["rect", "oval", "line", "arrow"] {
        objects::insert_shape(&mut d, 1, &objects::ShapeSpec { kind: kind.into(), rect: [100.0, 100.0, 200.0, 160.0], stroke: RED, fill: None, width: 2.0 }).unwrap();
    }
    let back = reload(&mut d);
    let content = back.get_page_content(*back.get_pages().get(&1).unwrap());
    let ops = Content::decode(&content).unwrap().operations;
    assert!(ops.iter().filter(|o| o.operator == "S" || o.operator == "B").count() >= 4);
    assert!(objects::insert_shape(&mut d, 1, &objects::ShapeSpec { kind: "star".into(), rect: [0.0; 4], stroke: RED, fill: None, width: 1.0 }).is_err());
}

#[test]
fn images_are_inserted_and_become_selectable_objects() {
    let mut d = doc();
    // 2x2 RGBA with one transparent pixel → SMask.
    let px = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0];
    objects::insert_image(&mut d, 1, [100.0, 100.0, 300.0, 200.0], &objects::ImageData::Rgba { width: 2, height: 2, pixels: px }).unwrap();
    // Minimal baseline JPEG header (SOI + SOF0 3 comps 16x8); content not decoded here.
    let jpg = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x08, 0x00, 0x10, 0x03, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 0xFF, 0xD9];
    let (w, h) = objects::insert_image(&mut d, 1, [300.0, 300.0, 332.0, 316.0], &objects::ImageData::Jpeg { bytes: jpg }).unwrap();
    assert_eq!((w, h), (16, 8));
    let imgs: Vec<_> = objects::list(&d, 1).unwrap().into_iter().filter(|o| o.kind == "image").collect();
    assert_eq!(imgs.len(), 3, "original + two inserted");
    assert!(imgs.iter().any(|o| o.rect.map(|v| v.round()) == [100.0, 100.0, 300.0, 200.0]));
    let back = reload(&mut d);
    let has_smask = back.objects.values().any(|o| o.as_stream().map(|s| s.dict.has(b"SMask")).unwrap_or(false));
    assert!(has_smask, "transparency kept");
    assert!(objects::insert_image(&mut d, 1, [0.0, 0.0, 50.0, 50.0], &objects::ImageData::Jpeg { bytes: vec![1, 2, 3] }).is_err());
}

#[test]
fn text_objects_report_font_size_colour_and_size_only_change_keeps_the_font() {
    let mut d = doc();
    let o = objects::list(&d, 1).unwrap();
    let line = o.iter().find(|o| o.text == "Public line").unwrap().clone();
    assert_eq!(line.font.as_deref(), Some("Helvetica"));
    assert_eq!(line.size, Some(12.0));
    assert_eq!(line.color.as_deref(), Some("#000000"));
    objects::recolor(&mut d, 1, &[line.id.clone()], [1.0, 0.0, 0.0]).unwrap();
    let line = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Public line").unwrap();
    assert_eq!(line.color.as_deref(), Some("#ff0000"));
    // Size only ("" family): same font, new size.
    objects::set_font(&mut d, 1, &[line.id.clone()], "", Some(20.0)).unwrap();
    let line = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Public line").unwrap();
    assert_eq!((line.font.as_deref(), line.size), (Some("Helvetica"), Some(20.0)));
    // Sizes are page sizes even when the text matrix scales the font (Tf 1, Tm 12).
    let mut d = doc();
    let page = *d.get_pages().get(&1).unwrap();
    let mut c = Content::decode(&d.get_page_content(page)).unwrap();
    c.operations[1] = Operation::new("Tf", vec!["F1".into(), 1.into()]);
    c.operations[2] = Operation::new("Tm", vec![12.into(), 0.into(), 0.into(), 12.into(), 72.into(), 700.into()]);
    c.operations[4] = Operation::new("Td", vec![0.into(), Object::Real(-20.0 / 12.0)]);
    d.change_page_content(page, c.encode().unwrap()).unwrap();
    let line = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Public line").unwrap();
    assert_eq!(line.size, Some(12.0));
    objects::set_font(&mut d, 1, &[line.id], "Times-Roman", Some(18.0)).unwrap();
    let line = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Public line").unwrap();
    assert_eq!((line.font.as_deref(), line.size), (Some("Times-Roman"), Some(18.0)));
}

#[test]
fn installed_font_is_embedded_as_a_subset_and_text_stays_extractable() {
    let Some(face) = stitchup_core::fonts::face_in_family("Arial", true, false).or_else(|| stitchup_core::fonts::system_faces().first()) else { return };
    let mut d = doc();
    let id = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Public line").unwrap().id;
    objects::set_font(&mut d, 1, &[id], &face.postscript, Some(14.0)).unwrap();
    let back = reload(&mut d);
    assert!(back.extract_text(&[1]).unwrap().contains("Public line"), "ToUnicode keeps the text");
    let line = objects::list(&back, 1).unwrap().into_iter().find(|o| o.text.contains("Public line")).unwrap();
    assert!(line.font.as_deref() == Some(face.postscript.replace(' ', "").as_str()) && line.size == Some(14.0), "{:?} {:?}", line.font, line.size);
}

#[test]
fn editing_to_cyrillic_and_greek_uses_an_installed_font_instead_of_question_marks() {
    if stitchup_core::fonts::system_faces().iter().all(|f| !f.cyrillic) {
        return; // no fonts on this machine
    }
    let mut d = doc();
    let req = stitchup_core::textedit::EditRequest {
        page: 1, x: 72.0, y: 680.0, width: 61.0, font_size: 12.0,
        old_text: "Public line".into(), new_text: "Привет, мир — Γειά σου".into(),
    };
    let out = stitchup_core::textedit::edit_text(&mut d, &req).unwrap();
    let back = reload(&mut d);
    let text = back.extract_text(&[1]).unwrap();
    assert!(text.contains("Привет, мир") && text.contains("Γειά σου"), "{text} / {}", out.note);
    assert!(!text.contains('?'), "{text}");
}

#[test]
fn vector_paths_can_be_listed_moved_recoloured_and_deleted() {
    let mut d = doc();
    let page = *d.get_pages().get(&1).unwrap();
    let mut c = Content::decode(&d.get_page_content(page)).unwrap();
    let n = |v: f64| Object::Real(v as f32);
    let extra = vec![
        // White full-page background: not offered as an object.
        Operation::new("q", vec![]), Operation::new("rg", vec![n(1.0), n(1.0), n(1.0)]),
        Operation::new("re", vec![n(0.0), n(0.0), n(612.0), n(792.0)]), Operation::new("f", vec![]), Operation::new("Q", vec![]),
        // A 2 pt blue line.
        Operation::new("q", vec![]), Operation::new("w", vec![n(2.0)]), Operation::new("RG", vec![n(0.0), n(0.0), n(1.0)]),
        Operation::new("m", vec![n(100.0), n(100.0)]), Operation::new("l", vec![n(300.0), n(100.0)]), Operation::new("S", vec![]), Operation::new("Q", vec![]),
        // A red filled rectangle, drawn under a scaling cm.
        Operation::new("q", vec![]), Operation::new("cm", vec![n(2.0), n(0.0), n(0.0), n(2.0), n(0.0), n(0.0)]),
        Operation::new("rg", vec![n(1.0), n(0.0), n(0.0)]), Operation::new("re", vec![n(175.0), n(200.0), n(50.0), n(25.0)]), Operation::new("f", vec![]), Operation::new("Q", vec![]),
    ];
    c.operations.splice(0..0, extra);
    d.change_page_content(page, c.encode().unwrap()).unwrap();

    let paths = |d: &PdfDoc| objects::list(d, 1).unwrap().into_iter().filter(|o| o.kind == "path").collect::<Vec<_>>();
    let p = paths(&d);
    assert_eq!(p.len(), 2, "{p:?}");
    let line = p.iter().find(|o| (o.rect[1] - 99.0).abs() < 1.0).expect("line, 2 pt tall around y=100").clone();
    let rect = p.iter().find(|o| (o.rect[0] - 350.0).abs() < 0.5).expect("rect at 350,400 in page space").clone();
    assert_eq!(rect.rect.map(|v| v.round()), [350.0, 400.0, 450.0, 450.0]);

    // Move the rectangle by (10, -20) in page space despite its 2× cm.
    objects::translate(&mut d, 1, &[rect.id.clone()], 10.0, -20.0).unwrap();
    let moved = paths(&d).into_iter().find(|o| (o.rect[0] - 360.0).abs() < 0.5).expect("moved rect");
    assert_eq!(moved.rect.map(|v| v.round()), [360.0, 380.0, 460.0, 430.0]);
    // Recolour the line, then delete the rectangle; text is untouched.
    let line_id = paths(&d).into_iter().find(|o| (o.rect[1] - 99.0).abs() < 1.0).unwrap().id;
    objects::recolor(&mut d, 1, &[line_id], [0.0, 0.6, 0.0]).unwrap();
    let ops = Content::decode(&d.get_page_content(page)).unwrap().operations;
    assert!(ops.iter().any(|o| o.operator == "RG" && o.operands.iter().filter_map(|v| v.as_float().ok()).collect::<Vec<_>>() == vec![0.0, 0.6, 0.0]), "line recoloured");
    let rid = paths(&d).into_iter().find(|o| (o.rect[0] - 360.0).abs() < 0.5).unwrap().id;
    objects::delete(&mut d, 1, &[rid]).unwrap();
    assert_eq!(paths(&d).len(), 1, "rectangle deleted, line kept");
    assert!(reload(&mut d).extract_text(&[1]).unwrap().contains("SECRET 1234"));
    let _ = line;
}

#[test]
fn existing_annotations_are_recoloured_including_their_appearance() {
    let mut d = doc();
    for spec in [
        AnnotSpec::Highlight { boxes: vec![[72.0, 698.0, 160.0, 712.0]], color: [1.0, 0.9, 0.0] },
        AnnotSpec::Square { rect: [72.0, 300.0, 200.0, 380.0], color: RED, fill: Some([1.0, 1.0, 0.8]), width: 2.0 },
        AnnotSpec::TextBox { rect: [72.0, 400.0, 300.0, 460.0], text: "Box".into(), size: 12.0, color: RED },
        AnnotSpec::Stamp { rect: [350.0, 300.0, 550.0, 360.0], label: "Approved".into(), color: [0.1, 0.6, 0.2] },
    ] {
        annots::add(&mut d, 1, &spec).unwrap();
    }
    let blue = [0.1, 0.3, 0.9];
    for (obj, _, _) in annots::rects(&d, 1) {
        annots::recolor(&mut d, 1, obj, blue).unwrap();
    }
    let back = reload(&mut d);
    let page = *back.get_pages().get(&1).unwrap();
    for a in back.get_page_annotations(page).unwrap() {
        let c: Vec<f64> = a.get(b"C").unwrap().as_array().unwrap().iter().map(|v| v.as_float().unwrap() as f64).collect();
        assert!(c.iter().zip(blue).all(|(x, y)| (x - y).abs() < 0.01), "{c:?}");
        let ap = a.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap();
        let s = back.get_object(ap).unwrap().as_stream().unwrap();
        let ops = Content::decode(&s.decompressed_content().unwrap()).unwrap().operations;
        let has_blue = ops.iter().any(|o| matches!(o.operator.as_str(), "RG" | "rg") && o.operands.iter().zip(blue).all(|(v, b)| (v.as_float().unwrap() as f64 - b).abs() < 0.01));
        let sub = a.get(b"Subtype").unwrap().as_name().unwrap().to_vec();
        assert!(has_blue || sub == b"FreeText", "{} appearance drawn in the new colour", String::from_utf8_lossy(&sub));
        if sub == b"Square" {
            // The fill (/IC) is not the outline colour: kept.
            assert!(ops.iter().any(|o| o.operator == "rg" && o.operands.iter().map(|v| v.as_float().unwrap()).collect::<Vec<_>>() == vec![1.0, 1.0, 0.8]), "fill kept");
        }
    }
}

#[test]
fn shapes_inside_a_shared_form_xobject_are_edited_copy_on_write() {
    use lopdf::{dictionary, Stream};
    let mut d = doc();
    let n = |v: f64| Object::Real(v as f32);
    // A form with two shapes, placed with a translate and drawn on the page.
    let form_ops = vec![
        Operation::new("rg", vec![n(1.0), n(0.0), n(0.0)]), Operation::new("re", vec![n(0.0), n(0.0), n(40.0), n(20.0)]), Operation::new("f", vec![]),
        Operation::new("RG", vec![n(0.0), n(0.0), n(1.0)]), Operation::new("m", vec![n(0.0), n(30.0)]), Operation::new("l", vec![n(40.0), n(30.0)]), Operation::new("S", vec![]),
    ];
    let form = d.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![n(-500.0), n(-500.0), n(500.0), n(500.0)], "Matrix" => vec![n(1.0), n(0.0), n(0.0), n(1.0), n(100.0), n(0.0)] },
        Content { operations: form_ops }.encode().unwrap(),
    ));
    let page = *d.get_pages().get(&1).unwrap();
    let mut pd = d.get_object(page).unwrap().as_dict().unwrap().clone();
    let mut res = pd.get(b"Resources").unwrap().as_dict().unwrap().clone();
    let mut xo = res.get(b"XObject").unwrap().as_dict().unwrap().clone();
    xo.set("Fm1", form);
    res.set("XObject", xo);
    pd.set("Resources", res);
    d.objects.insert(page, Object::Dictionary(pd));
    let mut c = Content::decode(&d.get_page_content(page)).unwrap();
    c.operations.extend([Operation::new("q", vec![]), Operation::new("cm", vec![n(1.0), n(0.0), n(0.0), n(1.0), n(0.0), n(200.0)]), Operation::new("Do", vec!["Fm1".into()]), Operation::new("Q", vec![])]);
    d.change_page_content(page, c.encode().unwrap()).unwrap();

    let inner = |d: &PdfDoc| objects::list(d, 1).unwrap().into_iter().filter(|o| o.id.starts_with('f')).collect::<Vec<_>>();
    let shapes = inner(&d);
    assert_eq!(shapes.len(), 2, "{shapes:?}");
    let rect = shapes.iter().find(|o| o.rect[1] < 205.0).unwrap().clone();
    assert_eq!(rect.rect.map(|v| v.round()), [100.0, 200.0, 140.0, 220.0], "page-space box (form Matrix × cm)");

    objects::translate(&mut d, 1, &[rect.id.clone()], 50.0, 10.0).unwrap();
    let moved = inner(&d).into_iter().find(|o| (o.rect[0] - 150.0).abs() < 0.5).expect("moved inside the form");
    assert_eq!(moved.rect.map(|v| v.round()), [150.0, 210.0, 190.0, 230.0]);
    // The original form object is untouched (copy-on-write).
    let orig = Content::decode(&d.get_object(form).unwrap().as_stream().unwrap().decompressed_content().unwrap()).unwrap();
    assert_eq!(orig.operations.len(), 7, "shared form not modified");
    let line = inner(&d).into_iter().find(|o| o.rect[1] > 225.0).unwrap();
    objects::recolor(&mut d, 1, &[line.id.clone()], [0.0, 0.5, 0.0]).unwrap();
    let line = inner(&d).into_iter().find(|o| o.rect[1] > 225.0).unwrap();
    objects::delete(&mut d, 1, &[line.id]).unwrap();
    assert_eq!(inner(&d).len(), 1, "line deleted, rectangle kept");
    let back = reload(&mut d);
    assert_eq!(objects::list(&back, 1).unwrap().into_iter().filter(|o| o.id.starts_with('f')).count(), 1, "survives save/reload");
}

/// Glyph codes shown by TJ arrays on page 1 that use 2-byte (embedded) fonts.
fn shown_cids(d: &PdfDoc) -> Vec<Vec<u16>> {
    let page = *d.get_pages().get(&1).unwrap();
    let ops = Content::decode(&d.get_page_content(page)).unwrap().operations;
    ops.iter()
        .filter(|o| o.operator == "TJ")
        .map(|o| {
            o.operands[0]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|e| match e {
                    Object::String(b, StringFormat::Hexadecimal) if b.len() == 2 => Some(u16::from_be_bytes([b[0], b[1]])),
                    _ => None,
                })
                .collect::<Vec<u16>>()
        })
        .filter(|v| !v.is_empty())
        .collect()
}

fn edit_public_line(d: &mut PdfDoc, new_text: &str) -> String {
    let req = stitchup_core::textedit::EditRequest {
        page: 1, x: 72.0, y: 680.0, width: 61.0, font_size: 12.0,
        old_text: "Public line".into(), new_text: new_text.into(),
    };
    stitchup_core::textedit::edit_text(d, &req).unwrap().note
}

#[test]
fn arabic_is_shaped_into_joining_forms_and_stays_extractable() {
    if !stitchup_core::fonts::system_faces().iter().any(|f| f.family == "Geeza Pro" || f.family == "Arial") {
        return;
    }
    let mut d = doc();
    let note = edit_public_line(&mut d, "ببب");
    let cids = shown_cids(&d).concat();
    assert_eq!(cids.len(), 3, "{cids:?} ({note})");
    let distinct: std::collections::HashSet<_> = cids.iter().collect();
    assert!(distinct.len() >= 2, "initial / medial / final forms differ: {cids:?} ({note})");
    let text = reload(&mut d).extract_text(&[1]).unwrap();
    assert_eq!(text.matches('ب').count(), 3, "{text}");
}

#[test]
fn devanagari_conjuncts_and_vowel_signs_are_shaped() {
    if !stitchup_core::fonts::system_faces().iter().any(|f| f.family.contains("Devanagari")) {
        return;
    }
    let mut d = doc();
    let note = edit_public_line(&mut d, "क्षि");
    let cids = shown_cids(&d).concat();
    assert!(!cids.is_empty() && cids.len() < 4, "4 characters → fewer glyphs (conjunct): {cids:?} ({note})");
    let text = reload(&mut d).extract_text(&[1]).unwrap();
    assert!(text.contains('क') && text.contains('ष'), "{text}");
}

#[test]
fn nested_groups_and_text_inside_groups_are_editable() {
    use lopdf::{dictionary, Stream};
    let mut d = doc();
    let n = |v: f64| Object::Real(v as f32);
    let font = d.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Roman", "Encoding" => "WinAnsiEncoding" });
    // Inner group: a filled square. Outer group: a caption (own font) + the inner group, shifted.
    let inner = d.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![n(-999.0), n(-999.0), n(999.0), n(999.0)] },
        Content { operations: vec![Operation::new("rg", vec![n(0.0), n(0.5), n(0.0)]), Operation::new("re", vec![n(0.0), n(0.0), n(30.0), n(30.0)]), Operation::new("f", vec![])] }.encode().unwrap(),
    ));
    let outer = d.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![n(-999.0), n(-999.0), n(999.0), n(999.0)],
            "Resources" => dictionary! { "Font" => dictionary! { "FT" => font }, "XObject" => dictionary! { "In" => inner } },
        },
        Content { operations: vec![
            Operation::new("BT", vec![]), Operation::new("Tf", vec!["FT".into(), 10.into()]), Operation::new("Td", vec![0.into(), 50.into()]),
            Operation::new("Tj", vec![Object::String(b"Caption inside a group".to_vec(), StringFormat::Literal)]), Operation::new("ET", vec![]),
            Operation::new("q", vec![]), Operation::new("cm", vec![n(1.0), n(0.0), n(0.0), n(1.0), n(10.0), n(0.0)]), Operation::new("Do", vec!["In".into()]), Operation::new("Q", vec![]),
        ] }.encode().unwrap(),
    ));
    let page = *d.get_pages().get(&1).unwrap();
    let mut pd = d.get_object(page).unwrap().as_dict().unwrap().clone();
    let mut res = pd.get(b"Resources").unwrap().as_dict().unwrap().clone();
    let mut xo = res.get(b"XObject").unwrap().as_dict().unwrap().clone();
    xo.set("Out", outer);
    res.set("XObject", xo);
    pd.set("Resources", res);
    d.objects.insert(page, Object::Dictionary(pd));
    let mut c = Content::decode(&d.get_page_content(page)).unwrap();
    c.operations.extend([Operation::new("q", vec![]), Operation::new("cm", vec![n(1.0), n(0.0), n(0.0), n(1.0), n(200.0), n(300.0)]), Operation::new("Do", vec!["Out".into()]), Operation::new("Q", vec![])]);
    d.change_page_content(page, c.encode().unwrap()).unwrap();

    let objs = objects::list(&d, 1).unwrap();
    let caption = objs.iter().find(|o| o.text == "Caption inside a group").expect("text inside the group is listed").clone();
    assert!(caption.id.starts_with('f') && caption.font.as_deref() == Some("Times-Roman"), "{caption:?}");
    assert!((caption.rect[0] - 200.0).abs() < 0.5 && (caption.rect[1] - 347.0).abs() < 2.0, "{:?}", caption.rect);
    let square = objs.iter().find(|o| o.kind == "path" && o.id.matches('.').count() == 1).expect("shape two levels deep").clone();
    assert_eq!(square.rect.map(|v| v.round()), [210.0, 300.0, 240.0, 330.0], "{square:?}");

    // Edit both: move the nested square, recolour + re-font the caption.
    objects::translate(&mut d, 1, &[square.id], 0.0, -100.0).unwrap();
    let cap = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Caption inside a group").unwrap();
    objects::recolor(&mut d, 1, &[cap.id], [0.8, 0.0, 0.0]).unwrap();
    let cap = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Caption inside a group").unwrap();
    objects::set_font(&mut d, 1, &[cap.id], "Helvetica-Bold", Some(14.0)).unwrap();

    let back = reload(&mut d);
    let objs = objects::list(&back, 1).unwrap();
    let sq = objs.iter().find(|o| o.kind == "path" && o.id.matches('.').count() == 1).unwrap();
    assert_eq!(sq.rect.map(|v| v.round()), [210.0, 200.0, 240.0, 230.0], "nested square moved");
    let cap = objs.iter().find(|o| o.text == "Caption inside a group").unwrap();
    assert_eq!((cap.font.as_deref(), cap.size, cap.color.as_deref()), (Some("Helvetica-Bold"), Some(14.0), Some("#cc0000")), "{cap:?}");
    // Originals untouched (copy-on-write at both levels).
    for id in [outer, inner] {
        let s = back.get_object(id).unwrap().as_stream().unwrap();
        let ops = Content::decode(&s.decompressed_content().unwrap()).unwrap().operations;
        assert!(!ops.iter().any(|o| o.operator == "Tf" && o.operands.first().and_then(|x| x.as_name().ok()) != Some(b"FT".as_slice())), "original group unchanged");
    }
    assert!(back.extract_text(&[1]).unwrap().contains("SECRET 1234"));
}

#[test]
fn text_inside_a_group_is_edited_in_place_not_covered() {
    use lopdf::{dictionary, Stream};
    use stitchup_core::textedit::{edit_text, EditMethod, EditRequest};
    let mut d = doc();
    let n = |v: f64| Object::Real(v as f32);
    let font = d.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Times-Roman", "Encoding" => "WinAnsiEncoding" });
    let group = d.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![n(-999.0), n(-999.0), n(999.0), n(999.0)],
            "Matrix" => vec![n(1.0), n(0.0), n(0.0), n(1.0), n(50.0), n(0.0)],
            "Resources" => dictionary! { "Font" => dictionary! { "FT" => font } },
        },
        Content { operations: vec![
            Operation::new("BT", vec![]), Operation::new("Tf", vec!["FT".into(), 10.into()]), Operation::new("Td", vec![0.into(), 50.into()]),
            Operation::new("Tj", vec![Object::String(b"Caption inside a group".to_vec(), StringFormat::Literal)]), Operation::new("ET", vec![]),
        ] }.encode().unwrap(),
    ));
    let page = *d.get_pages().get(&1).unwrap();
    let mut pd = d.get_object(page).unwrap().as_dict().unwrap().clone();
    let mut res = pd.get(b"Resources").unwrap().as_dict().unwrap().clone();
    let mut xo = res.get(b"XObject").unwrap().as_dict().unwrap().clone();
    xo.set("G", group);
    res.set("XObject", xo);
    pd.set("Resources", res);
    d.objects.insert(page, Object::Dictionary(pd));
    let mut c = Content::decode(&d.get_page_content(page)).unwrap();
    c.operations.extend([Operation::new("q", vec![]), Operation::new("cm", vec![n(1.0), n(0.0), n(0.0), n(1.0), n(150.0), n(300.0)]), Operation::new("Do", vec!["G".into()]), Operation::new("Q", vec![])]);
    d.change_page_content(page, c.encode().unwrap()).unwrap();

    let cap = objects::list(&d, 1).unwrap().into_iter().find(|o| o.text == "Caption inside a group").unwrap();
    let w = cap.rect[2] - cap.rect[0];
    // Page-space baseline: 150 + 50 = 200, 300 + 50 = 350.
    let mk = |old: &str, new: &str| EditRequest { page: 1, x: 200.0, y: 350.0, width: w, font_size: 10.0, old_text: old.into(), new_text: new.into() };
    let out = edit_text(&mut d, &mk("Caption inside a group", "Edited caption")).unwrap();
    assert_eq!(out.method, EditMethod::InPlace, "{}", out.note);
    // Letters Times-Roman (WinAnsi) lacks → replaced inside the group, still in place.
    let out2 = edit_text(&mut d, &mk("Edited caption", "Подпись")).unwrap();
    assert_eq!(out2.method, EditMethod::Replaced, "{}", out2.note);
    let back = reload(&mut d);
    // (lopdf's extract_text skips form XObjects: read the listed objects instead.)
    let text: String = objects::list(&back, 1).unwrap().into_iter().map(|o| o.text).collect::<Vec<_>>().join(" | ");
    assert!(text.contains("Подпись") && !text.contains("Caption inside") && !text.contains("Edited caption"), "{text}");
    let obj = objects::list(&back, 1).unwrap().into_iter().find(|o| o.text.contains("Подпись")).expect("new text listed");
    assert!(obj.id.starts_with('f') && (obj.rect[0] - 200.0).abs() < 1.0, "drawn inside the group at the same place: {obj:?}");
    // No white cover-up on the page, original group untouched.
    let ops = Content::decode(&back.get_page_content(page)).unwrap().operations;
    assert!(!ops.iter().any(|o| o.operator == "re"), "no overlay patch");
    let s = back.get_object(group).unwrap().as_stream().unwrap();
    assert!(String::from_utf8_lossy(&s.decompressed_content().unwrap()).contains("Caption inside a group"), "shared group unchanged");
}
