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
