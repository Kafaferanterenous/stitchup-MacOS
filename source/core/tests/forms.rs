//! AcroForm filling on the generated F-05 fixture.

use lopdf::Document as PdfDoc;
use stitchup_core::forms::{self, FieldValue};

fn fixture() -> PdfDoc {
    PdfDoc::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/generated/F-05_acroform.pdf")).unwrap()
}

#[test]
fn lists_fields_of_every_kind() {
    let fields = forms::list(&fixture());
    let kinds: Vec<&str> = fields.iter().map(|f| f.kind.as_str()).collect();
    println!("{:#?}", fields.iter().map(|f| (&f.name, &f.kind, &f.value, &f.options, f.widgets.len())).collect::<Vec<_>>());
    for k in ["text", "checkbox"] {
        assert!(kinds.contains(&k), "{k} missing: {kinds:?}");
    }
    assert!(fields.iter().all(|f| !f.widgets.is_empty()), "every field has a widget on a page");
}

#[test]
fn filling_sets_values_and_appearances_and_round_trips() {
    let mut d = fixture();
    let fields = forms::list(&d);
    for f in &fields {
        match f.kind.as_str() {
            "text" => forms::set_value(&mut d, &f.name, &FieldValue::Text("Zażółć Kowalski".into())).unwrap(),
            "checkbox" => forms::set_value(&mut d, &f.name, &FieldValue::Bool(true)).unwrap(),
            "radio" => {
                let on = f.widgets.iter().find_map(|w| w.on_state.clone()).unwrap();
                forms::set_value(&mut d, &f.name, &FieldValue::Text(on)).unwrap()
            }
            "choice" if !f.options.is_empty() => forms::set_value(&mut d, &f.name, &FieldValue::Text(f.options.last().unwrap().clone())).unwrap(),
            _ => {}
        }
    }
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let back = PdfDoc::load_mem(&bytes).unwrap();
    for f in forms::list(&back) {
        match f.kind.as_str() {
            "text" => assert_eq!(f.value, "Zażółć Kowalski", "{}", f.name),
            "checkbox" => assert_ne!(f.value, "Off", "{}", f.name),
            _ => {}
        }
    }
    // The filled text is drawn by the widget appearance (visible in any viewer).
    let text_widget = forms::list(&back).into_iter().find(|f| f.kind == "text").unwrap().widgets[0].id;
    let ap = back.get_object(text_widget).unwrap().as_dict().unwrap().get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap().as_reference().unwrap();
    let content = back.get_object(ap).unwrap().as_stream().unwrap().decompressed_content().unwrap_or_else(|_| back.get_object(ap).unwrap().as_stream().unwrap().content.clone());
    assert!(String::from_utf8_lossy(&content).contains("Tj"));

    // Unchecking works too.
    let cb = forms::list(&back).into_iter().find(|f| f.kind == "checkbox").unwrap().name;
    let mut d2 = back.clone();
    forms::set_value(&mut d2, &cb, &FieldValue::Bool(false)).unwrap();
    assert_eq!(forms::list(&d2).into_iter().find(|f| f.name == cb).unwrap().value, "Off");
    assert!(forms::set_value(&mut d2, "no.such.field", &FieldValue::Text("x".into())).is_err());
}
