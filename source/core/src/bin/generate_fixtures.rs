//! Fixture generator — emits the sanitized, deterministic Stage 1 fixtures
//! (F-01..F-14) into `fixtures/generated/` (see
//! `docs/STAGE0_FIXTURES_AND_ACCEPTANCE.md` §2). Content is synthetic only.

use std::path::PathBuf;

use lopdf::{dictionary, Document as PdfDoc, Object, Stream};

const OUT_DIR: &str = "fixtures/generated";

fn flush(path: &PathBuf, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!("  {} ({} bytes)", path.file_name().unwrap().to_string_lossy(), size);
}

fn save_bytes(mut doc: PdfDoc) -> Vec<u8> {
    let mut v = Vec::new();
    doc.save_to(&mut v).unwrap();
    v
}

fn main() {
    let out = PathBuf::from(OUT_DIR);
    std::fs::create_dir_all(&out).unwrap();
    println!("=== generating ParseAware fixtures ===");

    flush(&out.join("F-01_text.pdf"), &save_bytes(text_document(3)));
    flush(&out.join("F-02_mixed_text_image.pdf"), &save_bytes(mixed_document()));
    flush(&out.join("F-03_rotated.pdf"), &save_bytes(rotated_document()));
    flush(&out.join("F-04_encrypted.pdf"), &save_bytes(encrypted_document()));
    flush(&out.join("F-05_acroform.pdf"), &save_bytes(acroform_document()));
    flush(&out.join("F-06_annotations.pdf"), &save_bytes(annotations_document()));
    flush(&out.join("F-07_unicode_latin1.pdf"), &save_bytes(text_document_uni()));
    flush(&out.join("F-08_malformed.pdf"), &malformed_bytes());
    flush(&out.join("F-10_attachments.pdf"), &save_bytes(attachments_document()));
    flush(&out.join("F-11_large_200p.pdf"), &save_bytes(text_document(200)));
    flush(&out.join("F-12_blank.pdf"), &save_bytes(blank_document()));
    flush(&out.join("F-13_xfa.pdf"), &save_bytes(xfa_document()));
    flush(&out.join("F-14_pdfa_lite.pdf"), &save_bytes(pdfa_lite_document()));

    // F-09 (true incremental-revision fixture via raw object append + /Prev
    // chain) is deferred: lopdf save() emits a clean single-revision file, and
    // hand-appending a revision here would duplicate machinery better built as
    // a test helper. Documented in dev_notes.
    println!("  [deferred] F-09_incremental.pdf — needs raw append helper");
    println!("=== done ===");
}

// ---------------------------------------------------------------------------
// Low-level helpers (valid structure: Catalog -> Pages -> Page objects)
// ---------------------------------------------------------------------------

/// Start a document: create an empty Pages node and a Catalog; Root in trailer.
fn new_doc() -> (PdfDoc, (u32, u16)) {
    let mut doc = PdfDoc::with_version("1.7");
    let pages_id = doc.new_object_id();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => Object::Name("Pages".into()),
            "Kids" => Object::Array(vec![]),
            "Count" => Object::Integer(0),
        }),
    );
    let cat_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name("Catalog".into()),
        "Pages" => Object::Reference(pages_id),
    }));
    doc.trailer.set("Root", Object::Reference(cat_id));
    (doc, pages_id)
}

/// zlib-compress stream data so a `/Filter /FlateDecode` stream is actually
/// deflated. The raw content must never be stored under a FlateDecode filter —
/// renderers (pdf.js) reject that with "Unknown compression method in flate
/// stream" and draw a blank page.
fn flate(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// Page content stream data for a bare text page using standard Helvetica.
fn text_content(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 80);
    out.extend_from_slice(b"BT /F1 24 Tf 72 720 Td (");
    out.extend_from_slice(text.as_bytes());
    out.extend_from_slice(b") Tj ET");
    out
}

/// The /Font -> F1 (Helvetica) resource subtree as an Object.
fn font_resource() -> Object {
    Object::Dictionary(dictionary! {
        "F1" => Object::Dictionary(dictionary! {
            "Type" => Object::Name("Font".into()),
            "Subtype" => Object::Name("Type1".into()),
            "BaseFont" => Object::Name("Helvetica".into()),
        }),
    })
}

fn font_resources() -> Object {
    Object::Dictionary(dictionary! { "Font" => font_resource() })
}

fn media_box(w: f64, h: f64) -> Object {
    Object::Array(vec![
        Object::Real(0.0),
        Object::Real(0.0),
        Object::Real(w as f32),
        Object::Real(h as f32),
    ])
}

/// Append a page inside a given doc/pages node and return its (num, gen) id.
#[allow(clippy::too_many_arguments)]
fn add_page(
    doc: &mut PdfDoc,
    pages_id: (u32, u16),
    w: f64,
    h: f64,
    rotate: i64,
    content: &[u8],
    resources: Object,
    annots: Option<Object>,
) -> (u32, u16) {
    let cid = doc.add_object(Stream::new(dictionary! {"Filter" => "FlateDecode"}, flate(content)));
    let mut page_dict = dictionary! {
        "Type" => Object::Name("Page".into()),
        "Parent" => Object::Reference(pages_id),
        "Contents" => cid,
        "MediaBox" => media_box(w, h),
        "Resources" => resources,
    };
    if rotate != 0 {
        page_dict.set("Rotate", Object::Integer(rotate));
    }
    if let Some(a) = annots {
        page_dict.set("Annots", a);
    }
    let id = doc.add_object(Object::Dictionary(page_dict));
    register_kid(doc, pages_id, (id.0, id.1));
    (id.0, id.1)
}

/// Append a page object id to the root Pages node Kids (+ increment Count).
fn register_kid(doc: &mut PdfDoc, pages_id: (u32, u16), kid: (u32, u16)) {
    if let Some(pages) = doc.objects.get_mut(&pages_id) {
        if let Some(d) = pages.as_dict_mut().ok() {
            let pushed = match d.get_mut(b"Kids").expect("Pages node has Kids") {
                Object::Array(kids) => {
                    kids.push(Object::Reference((kid.0, kid.1)));
                    kids.len()
                }
                _ => 0,
            };
            d.set("Count", Object::Integer(pushed as i64));
        }
    }
}

/// Register the pages we created; kept for API symmetry.
fn finish(_doc: &mut PdfDoc, _kids: &[(u32, u16)]) {}

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

fn text_document(n_pages: usize) -> PdfDoc {
    let lines = [
        "Hello, Stitchup.",
        "Page two of the text fixture.",
        "The quick brown fox jumps over the lazy dog.",
    ];
    let (mut doc, pages) = new_doc();
    for i in 0..n_pages {
        let line = lines[i % lines.len()];
        add_page(&mut doc, pages, 612.0, 792.0, 0, &text_content(line), font_resources(), None);
    }
    finish(&mut doc, &[]);
    doc
}

fn text_document_uni() -> PdfDoc {
    // WinAnsi-encodable accent set; CJK/embedded font fixture is a later step.
    let s = "Café \u{E9} déjà vu \u{DC}bermut & Stra\u{DF}e. Zoë \u{2014} naïve mañana 100%";
    let (mut doc, pages) = new_doc();
    add_page(&mut doc, pages, 612.0, 792.0, 0, &text_content(s), font_resources(), None);
    finish(&mut doc, &[]);
    doc
}

fn rotated_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    for &r in &[0i64, 90, 180, 270] {
        add_page(&mut doc, pages, 420.0, 594.0, r, &text_content("ROTATED PAGE"), font_resources(), None);
    }
    finish(&mut doc, &[]);
    doc
}

fn blank_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    add_page(&mut doc, pages, 612.0, 792.0, 0, &[], font_resources(), None);
    finish(&mut doc, &[]);
    doc
}

fn mixed_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    // one 8x8 RGB image XObject
    let mut img = Vec::with_capacity(8 * 8 * 3);
    for y in 0..8 {
        for x in 0..8 {
            if (x + y) % 2 == 0 {
                img.extend_from_slice(&[255, 0, 128]);
            } else {
                img.extend_from_slice(&[0, 64, 255]);
            }
        }
    }
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut enc, &img).unwrap();
    let img_bytes = enc.finish().unwrap();
    let img_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => Object::Name("XObject".into()),
            "Subtype" => Object::Name("Image".into()),
            "Width" => Object::Integer(8),
            "Height" => Object::Integer(8),
            "ColorSpace" => Object::Name("DeviceRGB".into()),
            "BitsPerComponent" => Object::Integer(8),
            "Filter" => Object::Name("FlateDecode".into()),
        },
        img_bytes,
    ));
    let resources = dictionary! {
        "Font" => font_resource(),
        "XObject" => Object::Dictionary(dictionary! { "Img1" => Object::Reference((img_id.0, img_id.1)) }),
    };
    let mut content = Vec::new();
    content.extend_from_slice(b"BT /F1 20 Tf 72 720 Td (Mixed content page.) Tj ET\n");
    content.extend_from_slice(b"0.0 0.6 1.0 rg 72 600 300 120 re f\n");
    content.extend_from_slice(b"0.2 0.2 0.2 RG 72 540 300 40 re S\n");
    content.extend_from_slice(b"q 100 0 0 100 250 300 cm /Img1 Do Q");
    add_page(&mut doc, pages, 612.0, 792.0, 0, &content, Object::Dictionary(resources), None);
    finish(&mut doc, &[]);
    doc
}

fn encrypted_document() -> PdfDoc {
    // RC4 (EncryptionVersion::V1, revision 2) with known passwords. Note: this
    // encrypts streams/strings only; the /Root + page tree stay readable, which
    // is exactly what our F-04 acceptance checks target (password prompt path).
    use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions};
    let mut doc = text_document(1);
    // lopdf requires a trailer /ID for standard security handlers (R2/R3 need it).
    let id_obj = Object::String(
        "D0C0A11E5EEDC0DE0000112210000000".into(),
        lopdf::StringFormat::Hexadecimal,
    );
    doc.trailer.set("ID", Object::Array(vec![id_obj.clone(), id_obj]));
    let state = EncryptionState::try_from(EncryptionVersion::V1 {
        document: &doc,
        owner_password: "stitchup-owner",
        user_password: "stitchup-user",
        permissions: Permissions::default(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    doc
}

fn acroform_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    let cid = doc.add_object(Stream::new(dictionary! {"Filter" => "FlateDecode"}, flate(&text_content("Fill me."))));
    let field_def = |doc: &mut PdfDoc, name: &str, ft: &str, rect: Object, page_id: (u32, u16)| {
        doc.add_object(Object::Dictionary(dictionary! {
            "Type" => Object::Name("Annot".into()),
            "Subtype" => Object::Name("Widget".into()),
            "FT" => Object::Name(ft.into()),
            "T" => Object::String(name.into(), lopdf::StringFormat::Literal),
            "Rect" => rect,
            "P" => Object::Reference(page_id),
        }))
    };
    let page_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name("Page".into()),
        "Parent" => Object::Reference(pages),
        "Contents" => cid,
        "MediaBox" => media_box(612.0, 792.0),
        "Resources" => font_resources(),
    }));
    let tf = field_def(&mut doc, "name", "Tx", Object::Array(vec![Object::Real(72.0), Object::Real(600.0), Object::Real(300.0), Object::Real(620.0)]), (page_id.0, page_id.1));
    let cb = field_def(&mut doc, "agree", "Btn", Object::Array(vec![Object::Real(72.0), Object::Real(560.0), Object::Real(120.0), Object::Real(580.0)]), (page_id.0, page_id.1));
    register_kid(&mut doc, pages, (page_id.0, page_id.1));
    if let Some(p) = doc.objects.get_mut(&(page_id.0, page_id.1)).and_then(|o| o.as_dict_mut().ok()) {
        p.set("Annots", Object::Array(vec![Object::Reference(tf), Object::Reference(cb)]));
    }
    let acro_id = doc.add_object(Object::Dictionary(dictionary! {
        "Fields" => Object::Array(vec![Object::Reference(tf), Object::Reference(cb)]),
    }));
    if let Some(cat) = doc.trailer.get(b"Root").ok().and_then(|r| r.as_reference().ok()).and_then(|r| doc.objects.get_mut(&r)).and_then(|o| o.as_dict_mut().ok()) {
        cat.set("AcroForm", Object::Reference(acro_id));
    }
    finish(&mut doc, &[]);
    doc
}

fn annotations_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    let cid = doc.add_object(Stream::new(dictionary! {"Filter" => "FlateDecode"}, flate(&text_content("Has a note."))));
    let note_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name("Annot".into()),
        "Subtype" => Object::Name("Text".into()),
        "Contents" => Object::String("Native note annotation (read-only).".into(), lopdf::StringFormat::Literal),
        "Rect" => Object::Array(vec![Object::Real(72.0), Object::Real(680.0), Object::Real(96.0), Object::Real(704.0)]),
        "Open" => Object::Boolean(false),
    }));
    let page_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name("Page".into()),
        "Parent" => Object::Reference(pages),
        "Contents" => cid,
        "Annots" => Object::Array(vec![Object::Reference(note_id)]),
        "MediaBox" => media_box(612.0, 792.0),
        "Resources" => font_resources(),
    }));
    if let Some(p) = doc.objects.get_mut(&(page_id.0, page_id.1)).and_then(|o| o.as_dict_mut().ok()) {
        p.set("Annots", Object::Array(vec![Object::Reference((note_id.0, note_id.1))]));
    }
    register_kid(&mut doc, pages, (page_id.0, page_id.1));
    finish(&mut doc, &[]);
    doc
}

fn malformed_bytes() -> Vec<u8> {
    // A syntactically valid file whose Catalog points at a Pages node that does
    // not exist: the parser accepts the bytes, but page collection must fail
    // with a structured `malformed` error (no crash, no hang).
    let (mut doc, _pages) = new_doc();
    let cat_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name("Catalog".into()),
        "Pages" => Object::Reference((0xFFFFF0, 0)),
    }));
    doc.trailer.set("Root", Object::Reference(cat_id));
    let mut out = save_bytes(doc);
    // Append a garbage tail so the xref/startxref region is also non-conforming.
    out.extend_from_slice(b"\nnot_a_valid_tail_terminator 0 0 R\n");
    out
}

fn attachments_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    let payload = b"stitchup fixture attachment payload".to_vec();
    let file_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => Object::Name("EmbeddedFile".into()),
            "Params" => Object::Dictionary(dictionary! { "Size" => Object::Integer(payload.len() as i64) }),
            "Filter" => Object::Name("FlateDecode".into()),
        },
        flate(&payload),
    ));
    let fs_id = doc.add_object(Object::Dictionary(dictionary! {
        "Type" => Object::Name("Filespec".into()),
        "F" => Object::String("fixture.txt".into(), lopdf::StringFormat::Literal),
        "UF" => Object::String("fixture.txt".into(), lopdf::StringFormat::Literal),
        "EF" => Object::Dictionary(dictionary! { "F" => Object::Reference((file_id.0, file_id.1)) }),
    }));
    let names_id = doc.add_object(Object::Dictionary(dictionary! {
        "EmbeddedFiles" => Object::Dictionary(dictionary! {
            "Names" => Object::Array(vec![
                Object::String("fixture.txt".into(), lopdf::StringFormat::Literal),
                Object::Reference((fs_id.0, fs_id.1)),
            ]),
        }),
    }));
    add_page(&mut doc, pages, 612.0, 792.0, 0, &text_content("Has attachment."), font_resources(), None);
    if let Some(cat) = doc.trailer.get(b"Root").ok().and_then(|r| r.as_reference().ok()).and_then(|r| doc.objects.get_mut(&r)).and_then(|o| o.as_dict_mut().ok()) {
        cat.set("Names", Object::Reference((names_id.0, names_id.1)));
    }
    finish(&mut doc, &[]);
    doc
}

fn xfa_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    add_page(&mut doc, pages, 612.0, 792.0, 0, &text_content("XFA present"), font_resources(), None);
    let payload = "<?xml version=\"1.0\"?><xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\"><template/></xdp:xdp>";
    let acro_id = doc.add_object(Object::Dictionary(dictionary! {
        "Fields" => Object::Array(vec![]),
        "XFA" => Object::String(payload.into(), lopdf::StringFormat::Literal),
    }));
    if let Some(cat) = doc.trailer.get(b"Root").ok().and_then(|r| r.as_reference().ok()).and_then(|r| doc.objects.get_mut(&r)).and_then(|o| o.as_dict_mut().ok()) {
        cat.set("AcroForm", Object::Reference((acro_id.0, acro_id.1)));
    }
    finish(&mut doc, &[]);
    doc
}

fn pdfa_lite_document() -> PdfDoc {
    let (mut doc, pages) = new_doc();
    add_page(&mut doc, pages, 612.0, 792.0, 0, &text_content("PDF/A-lite"), font_resources(), None);
    let info_id = doc.new_object_id();
    doc.objects.insert(info_id, Object::Dictionary(dictionary! {
        "Title" => Object::String("PDF/A-lite fixture".into(), lopdf::StringFormat::Literal),
        "Creator" => Object::String("Stitchup fixture generator".into(), lopdf::StringFormat::Literal),
    }));
    doc.trailer.set("Info", Object::Reference(info_id));
    let xmp = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">PDF/A-lite fixture</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>".to_string();
    let m_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => Object::Name("Metadata".into()),
            "Subtype" => Object::Name("XML".into()),
            "Filter" => Object::Name("FlateDecode".into()),
        },
        flate(xmp.as_bytes()),
    ));
    if let Some(cat) = doc.trailer.get(b"Root").ok().and_then(|r| r.as_reference().ok()).and_then(|r| doc.objects.get_mut(&r)).and_then(|o| o.as_dict_mut().ok()) {
        cat.set("Metadata", Object::Reference((m_id.0, m_id.1)));
    }
    finish(&mut doc, &[]);
    doc
}