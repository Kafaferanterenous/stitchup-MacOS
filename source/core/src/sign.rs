//! Digital signatures (PDF 32000 §12.8): a detached PKCS#7 / CMS signature
//! (SHA-256, RSA-2048) over the whole file, `adbe.pkcs7.detached`, signed with
//! a freshly generated self-signed certificate (like the Windows build).
//!
//! The document is serialized with a fixed-size zero-filled `/Contents`
//! placeholder and a fixed-width `/ByteRange`; the byte range is then patched
//! in place, the two covered ranges hashed, and the CMS blob written into the
//! placeholder — so the output is byte-identical except inside `/Contents`.

use std::str::FromStr;
use std::time::{Duration, SystemTime};

use cms::builder::{create_signing_time_attribute, SignedDataBuilder, SignerInfoBuilder};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::signed_data::{EncapsulatedContentInfo, SignerIdentifier};
use der::Encode;
use lopdf::{dictionary, Dictionary, Document as PdfDoc, Object, ObjectId, Stream, StringFormat};
use rand_core::OsRng;
use rsa::pkcs1v15::SigningKey;
use rsa::RsaPrivateKey;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned};
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::Validity;

use crate::error::{EngineError, ErrorCode};

/// Bytes reserved for the CMS blob (hex-encoded → twice this in the file).
const SIG_SPACE: usize = 12_000;
const BR_PLACEHOLDER: [i64; 4] = [0, 1_111_111_111, 2_222_222_222, 3_333_333_333];

#[derive(Debug, Clone, Deserialize)]
pub struct SignSpec {
    /// Signer's name (certificate CN and /Name).
    pub name: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub contact: String,
    /// Visible signature box: 1-based page + user-space rect. None = invisible.
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub rect: Option<[f64; 4]>,
}

fn e<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> EngineError + '_ {
    move |err| EngineError::new(ErrorCode::Unsupported, format!("{what}: {err}"))
}

fn pdf_date(t: SystemTime) -> String {
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (H. Hinnant), UTC.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("D:{y:04}{m:02}{d:02}{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Human date for the visible stamp, e.g. "2026-11-13 14:05 UTC".
fn human_date(t: SystemTime) -> String {
    let d = pdf_date(t);
    format!("{}-{}-{} {}:{} UTC", &d[2..6], &d[6..8], &d[8..10], &d[10..12], &d[12..14])
}

/// Self-signed RSA-2048 identity for `name`.
fn identity(name: &str) -> Result<(SigningKey<Sha256>, x509_cert::Certificate), EngineError> {
    let key = RsaPrivateKey::new(&mut OsRng, 2048).map_err(e("key generation"))?;
    let signer = SigningKey::<Sha256>::new(key);
    let spki = SubjectPublicKeyInfoOwned::from_key(signer.as_ref().to_public_key()).map_err(e("public key"))?;
    let cn = name.replace([',', '+', '=', '"', '\\', '<', '>', ';'], " ");
    let subject = Name::from_str(&format!("CN={},O=Stitchup self-signed", cn.trim())).map_err(e("certificate name"))?;
    let mut serial = [0u8; 16];
    getrandom::fill(&mut serial).map_err(e("randomness"))?;
    serial[0] &= 0x7f; // positive INTEGER
    let builder = CertificateBuilder::new(
        Profile::Root,
        SerialNumber::new(&serial).map_err(e("serial"))?,
        Validity::from_now(Duration::from_secs(5 * 365 * 86_400)).map_err(e("validity"))?,
        subject,
        spki,
        &signer,
    )
    .map_err(e("certificate"))?;
    let cert = builder.build::<rsa::pkcs1v15::Signature>().map_err(e("certificate signing"))?;
    Ok((signer, cert))
}

/// Detached CMS SignedData over `digest` (SHA-256 of the signed byte ranges).
fn cms_detached(signer: &SigningKey<Sha256>, cert: &x509_cert::Certificate, digest: &[u8]) -> Result<Vec<u8>, EngineError> {
    let content = EncapsulatedContentInfo { econtent_type: const_oid::db::rfc5911::ID_DATA, econtent: None };
    let digest_alg = AlgorithmIdentifierOwned { oid: const_oid::db::rfc5912::ID_SHA_256, parameters: None };
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer: cert.tbs_certificate.issuer.clone(),
        serial_number: cert.tbs_certificate.serial_number.clone(),
    });
    let mut info = SignerInfoBuilder::new(signer, sid, digest_alg.clone(), &content, Some(digest)).map_err(e("signer info"))?;
    info.add_signed_attribute(create_signing_time_attribute().map_err(e("signing time"))?).map_err(e("signing time"))?;
    let ci = SignedDataBuilder::new(&content)
        .add_digest_algorithm(digest_alg)
        .map_err(e("digest"))?
        .add_certificate(CertificateChoices::Certificate(cert.clone()))
        .map_err(e("certificate"))?
        .add_signer_info::<SigningKey<Sha256>, rsa::pkcs1v15::Signature>(info)
        .map_err(e("signature"))?
        .build()
        .map_err(e("signed data"))?;
    ci.to_der().map_err(e("encoding"))
}

fn text(s: &str) -> Object {
    lopdf::text_string(s)
}

/// Visible appearance: bordered box with "Digitally signed by …", date, reason.
fn appearance(doc: &mut PdfDoc, w: f64, h: f64, lines: &[String]) -> ObjectId {
    use lopdf::content::{Content, Operation};
    let r = |v: f64| Object::Real(v as f32);
    let size = (h / (lines.len() as f64 + 0.8) * 0.8).clamp(5.0, 14.0);
    let mut ops = vec![
        Operation::new("q", vec![]),
        Operation::new("rg", vec![r(0.94), r(0.96), r(1.0)]),
        Operation::new("RG", vec![r(0.1), r(0.3), r(0.7)]),
        Operation::new("w", vec![r(1.0)]),
        Operation::new("re", vec![r(0.5), r(0.5), r(w - 1.0), r(h - 1.0)]),
        Operation::new("B", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("rg", vec![r(0.05), r(0.15), r(0.4)]),
    ];
    let mut diffs = Vec::new();
    let mut encoded = Vec::new();
    for l in lines {
        let t = crate::textedit::std14_encode(l);
        for d in &t.diffs {
            if !diffs.contains(d) {
                diffs.push(*d);
            }
        }
        encoded.push(t.bytes);
    }
    // One font for all lines: re-encode with the union of diffs.
    let mut y = h - size * 1.25;
    for (i, l) in lines.iter().enumerate() {
        let font_size = if i == 0 { size * 1.05 } else { size * 0.9 };
        let font = if i == 0 { "SB" } else { "SR" };
        let bytes = encode_with(l, &diffs).unwrap_or_else(|| encoded[i].clone());
        ops.push(Operation::new("Tf", vec![Object::Name(font.as_bytes().to_vec()), r(font_size)]));
        ops.push(Operation::new("Tm", vec![r(1.0), r(0.0), r(0.0), r(1.0), r(size * 0.5), r(y)]));
        ops.push(Operation::new("Tj", vec![Object::String(bytes, StringFormat::Literal)]));
        y -= size * 1.15;
    }
    ops.extend([Operation::new("ET", vec![]), Operation::new("Q", vec![])]);
    let data = Content { operations: ops }.encode().unwrap_or_default();
    doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), r(w), r(h)],
            "Resources" => dictionary! { "Font" => dictionary! {
                "SB" => crate::textedit::std14_font("Helvetica-Bold", &diffs),
                "SR" => crate::textedit::std14_font("Helvetica", &diffs),
            } },
        },
        data,
    ))
}

/// Encode `s` using a fixed /Differences table (all lines share one font).
fn encode_with(s: &str, diffs: &[(u8, &'static str)]) -> Option<Vec<u8>> {
    let t = crate::textedit::std14_encode(s);
    let mut out = Vec::with_capacity(t.bytes.len());
    for b in t.bytes {
        if (1..32).contains(&b) {
            let name = t.diffs.iter().find(|(c, _)| *c == b)?.1;
            out.push(diffs.iter().find(|(_, n)| *n == name)?.0);
        } else {
            out.push(b);
        }
    }
    Some(out)
}

/// Produce signed file bytes for `doc` (serialized first; `doc` is not modified).
pub fn sign_document(doc: &PdfDoc, spec: &SignSpec) -> Result<Vec<u8>, EngineError> {
    let mut d = doc.clone();
    d.trailer.remove(b"Encrypt");
    let mut base = Vec::new();
    d.save_to(&mut base)?;
    sign_bytes(base, spec)
}

/// Sign an existing PDF file's bytes as an INCREMENTAL UPDATE: `base` is kept
/// byte-for-byte and the signature objects are appended after it, so any
/// signatures already in `base` stay valid.
pub fn sign_bytes(base: Vec<u8>, spec: &SignSpec) -> Result<Vec<u8>, EngineError> {
    let name = spec.name.trim();
    if name.is_empty() {
        return Err(crate::err!(ErrorCode::ValidationFailed, "enter the signer's name"));
    }
    let now = SystemTime::now();
    let prev = PdfDoc::load_mem(&base)?;
    if prev.is_encrypted() {
        return Err(crate::err!(ErrorCode::Unsupported, "this PDF is password-protected; unlock it, save a copy, then sign"));
    }
    let pages = prev.get_pages();
    let page_no = spec.page.unwrap_or(1);
    let page_id = *pages
        .get(&page_no)
        .ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no page {page_no}"))?;
    let existing = crate::docops::list_field_names(&prev);
    let mut inc = lopdf::IncrementalDocument::create_from(base.clone(), prev.clone());
    let d = &mut inc.new_document;

    // Signature dictionary with fixed-size placeholders.
    let mut sig = dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => "adbe.pkcs7.detached",
        "ByteRange" => BR_PLACEHOLDER.iter().map(|v| Object::Integer(*v)).collect::<Vec<_>>(),
        "Contents" => Object::String(vec![0u8; SIG_SPACE], StringFormat::Hexadecimal),
        "M" => Object::string_literal(pdf_date(now)),
        "Name" => text(name),
    };
    for (k, v) in [("Reason", &spec.reason), ("Location", &spec.location), ("ContactInfo", &spec.contact)] {
        if !v.trim().is_empty() {
            sig.set(k, text(v.trim()));
        }
    }
    let sig_id = d.add_object(sig);

    // Signature field + widget (visible box or zero-size).
    let rect = spec.rect.unwrap_or([0.0, 0.0, 0.0, 0.0]);
    let (w, h) = ((rect[2] - rect[0]).abs(), (rect[3] - rect[1]).abs());
    let field_name = (1..).map(|i| format!("Signature{i}")).find(|n| !existing.contains(n)).unwrap();
    let mut widget = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Sig",
        "T" => text(&field_name),
        "V" => sig_id,
        "F" => 132,
        "P" => page_id,
        "Rect" => rect.iter().map(|v| Object::Real(*v as f32)).collect::<Vec<_>>(),
    };
    if w >= 4.0 && h >= 4.0 {
        let mut lines = vec![format!("Digitally signed by {name}"), format!("Date: {}", human_date(now))];
        if !spec.reason.trim().is_empty() {
            lines.push(format!("Reason: {}", spec.reason.trim()));
        }
        if !spec.location.trim().is_empty() {
            lines.push(format!("Location: {}", spec.location.trim()));
        }
        let ap = appearance(d, w, h, &lines);
        widget.set("AP", dictionary! { "N" => ap });
    }
    let widget_id = d.add_object(widget);

    // Updated page (/Annots) and catalog (/AcroForm) go into the new section.
    let mut page = prev.get_object(page_id)?.as_dict()?.clone();
    let mut annots = match page.get(b"Annots") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => prev.get_object(*r).and_then(Object::as_array).cloned().unwrap_or_default(),
        _ => vec![],
    };
    annots.push(Object::Reference(widget_id));
    page.set("Annots", annots);
    d.objects.insert(page_id, Object::Dictionary(page));

    let root_id = prev.trailer.get(b"Root")?.as_reference()?;
    let mut catalog = prev.get_object(root_id)?.as_dict()?.clone();
    let (form_ref, mut form): (Option<ObjectId>, Dictionary) = match catalog.get(b"AcroForm") {
        Ok(Object::Dictionary(f)) => (None, f.clone()),
        Ok(Object::Reference(r)) => (Some(*r), prev.get_object(*r).and_then(Object::as_dict).cloned().unwrap_or_default()),
        _ => (None, Dictionary::new()),
    };
    let mut fields = match form.get(b"Fields") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => prev.get_object(*r).and_then(Object::as_array).cloned().unwrap_or_default(),
        _ => vec![],
    };
    fields.push(Object::Reference(widget_id));
    form.set("Fields", fields);
    form.set("SigFlags", 3);
    match form_ref {
        Some(r) => {
            d.objects.insert(r, Object::Dictionary(form));
        }
        None => catalog.set("AcroForm", Object::Dictionary(form)),
    }
    d.objects.insert(root_id, Object::Dictionary(catalog));

    let mut bytes = Vec::new();
    inc.save_to(&mut bytes)?;

    // Locate the placeholders in the APPENDED part only.
    let tail = base.len();
    let zeros = vec![b'0'; SIG_SPACE * 2];
    let start = find(&bytes[tail..], &zeros)
        .map(|i| i + tail)
        .and_then(|i| i.checked_sub(1))
        .filter(|&i| bytes[i] == b'<')
        .ok_or_else(|| crate::err!(ErrorCode::IoError, "signature placeholder not found"))?;
    let end = start + SIG_SPACE * 2 + 2; // past '>'
    let br_text = BR_PLACEHOLDER.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    let br_at = find(&bytes[tail..], br_text.as_bytes())
        .map(|i| i + tail)
        .ok_or_else(|| crate::err!(ErrorCode::IoError, "byte range placeholder not found"))?;
    let br = format!("0 {} {} {}", start, end, bytes.len() - end);
    if br.len() > br_text.len() {
        return Err(crate::err!(ErrorCode::IoError, "document too large to sign"));
    }
    let padded = format!("{br:<width$}", width = br_text.len());
    bytes[br_at..br_at + padded.len()].copy_from_slice(padded.as_bytes());

    // Hash both ranges, sign, write the CMS blob into the placeholder.
    let mut h = Sha256::new();
    h.update(&bytes[..start]);
    h.update(&bytes[end..]);
    let digest = h.finalize();
    let (signer, cert) = identity(name)?;
    let blob = cms_detached(&signer, &cert, &digest)?;
    if blob.len() > SIG_SPACE {
        return Err(crate::err!(ErrorCode::IoError, "signature larger than reserved space"));
    }
    let hex: Vec<u8> = blob.iter().flat_map(|b| format!("{b:02X}").into_bytes()).collect();
    bytes[start + 1..start + 1 + hex.len()].copy_from_slice(&hex);
    Ok(bytes)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Signatures found in a PDF (for the "signed by" info and tests).
pub fn list_signatures(doc: &PdfDoc) -> Vec<(String, String)> {
    let mut out = vec![];
    for (_, obj) in doc.objects.iter() {
        if let Ok(d) = obj.as_dict() {
            if d.get(b"Type").and_then(Object::as_name).ok() == Some(b"Sig") {
                let get = |k: &[u8]| d.get(k).ok().and_then(|o| lopdf::decode_text_string(o).ok()).unwrap_or_default();
                out.push((get(b"Name"), get(b"M")));
            }
        }
    }
    out
}
