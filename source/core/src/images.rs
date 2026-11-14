//! Compress ▸ image downsampling. Each image's effective resolution is worked
//! out from how large it is drawn on its pages; images well above the target
//! are resampled (area average) and re-encoded — JPEG stays JPEG (at the
//! chosen quality), lossless images stay lossless (Flate). An image is only
//! replaced when the new one is smaller. Images we can't safely handle
//! (masks, CMYK, 16-bit, Decode arrays, unusual filters) are left untouched.

use std::collections::BTreeMap;

use lopdf::{Document as PdfDoc, Object, ObjectId, Stream};
use serde::Serialize;

use crate::textedit::{font_info, interpret_page, FontInfo};

#[derive(Debug, Clone, Default, Serialize)]
pub struct DownsampleReport {
    /// Images resampled and replaced.
    pub images: usize,
    /// Bytes of those images before / after.
    pub before: usize,
    pub after: usize,
    /// Images left alone because their format isn't handled.
    pub skipped: usize,
}

/// Highest effective resolution (dpi) of every image XObject drawn directly on a page.
fn image_dpi(doc: &PdfDoc) -> BTreeMap<ObjectId, f64> {
    let mut out: BTreeMap<ObjectId, f64> = BTreeMap::new();
    for (_, page) in doc.get_pages() {
        let Ok(content) = doc.get_and_decode_page_content(page) else { continue };
        let fonts: BTreeMap<Vec<u8>, FontInfo> = doc.get_page_fonts(page).unwrap_or_default().into_iter().map(|(k, d)| (k, font_info(doc, d))).collect();
        let parsed = interpret_page(doc, &content.operations, &fonts);
        let xobjects = match crate::model::inherited(doc, page, b"Resources") {
            Some(Object::Dictionary(res)) => match res.get(b"XObject") {
                Ok(Object::Dictionary(d)) => d.clone(),
                Ok(Object::Reference(id)) => doc.get_object(*id).and_then(Object::as_dict).cloned().unwrap_or_default(),
                _ => continue,
            },
            _ => continue,
        };
        for d in &parsed.draws {
            let Ok(Object::Reference(id)) = xobjects.get(&d.name) else { continue };
            let Ok(Object::Stream(s)) = doc.get_object(*id) else { continue };
            if s.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image".as_slice()) {
                continue;
            }
            let (w, h) = (int(&s.dict, b"Width") as f64, int(&s.dict, b"Height") as f64);
            let (wpt, hpt) = ((d.ctm[0].powi(2) + d.ctm[1].powi(2)).sqrt(), (d.ctm[2].powi(2) + d.ctm[3].powi(2)).sqrt());
            if wpt < 0.5 || hpt < 0.5 {
                continue;
            }
            let dpi = (w / (wpt / 72.0)).max(h / (hpt / 72.0));
            let e = out.entry(*id).or_insert(0.0);
            *e = e.max(dpi);
        }
    }
    out
}

fn int(d: &lopdf::Dictionary, k: &[u8]) -> i64 {
    match d.get(k) {
        Ok(Object::Integer(i)) => *i,
        Ok(Object::Real(r)) => *r as i64,
        _ => 0,
    }
}

fn filters(d: &lopdf::Dictionary) -> Vec<Vec<u8>> {
    match d.get(b"Filter") {
        Ok(Object::Name(n)) => vec![n.clone()],
        Ok(Object::Array(a)) => a.iter().filter_map(|o| o.as_name().ok().map(|n| n.to_vec())).collect(),
        _ => vec![],
    }
}

/// Number of colour components, or None for colour spaces we don't resample.
fn components(doc: &PdfDoc, d: &lopdf::Dictionary) -> Option<usize> {
    let cs = match d.get(b"ColorSpace").ok()? {
        Object::Reference(r) => doc.get_object(*r).ok()?,
        o => o,
    };
    match cs {
        Object::Name(n) if n == b"DeviceRGB" => Some(3),
        Object::Name(n) if n == b"DeviceGray" => Some(1),
        Object::Array(a) if a.first().and_then(|o| o.as_name().ok()) == Some(b"ICCBased".as_slice()) => {
            let s = match a.get(1)? {
                Object::Reference(r) => doc.get_object(*r).ok()?.as_stream().ok()?,
                _ => return None,
            };
            match int(&s.dict, b"N") {
                1 => Some(1),
                3 => Some(3),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Area-average resample of 8-bit pixels with `n` channels.
fn resample(px: &[u8], w: usize, h: usize, n: usize, nw: usize, nh: usize) -> Vec<u8> {
    let mut out = vec![0u8; nw * nh * n];
    for y in 0..nh {
        let y0 = y * h / nh;
        let y1 = ((y + 1) * h / nh).max(y0 + 1).min(h);
        for x in 0..nw {
            let x0 = x * w / nw;
            let x1 = ((x + 1) * w / nw).max(x0 + 1).min(w);
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            for c in 0..n {
                let mut sum = 0u32;
                for sy in y0..y1 {
                    let row = sy * w * n;
                    for sx in x0..x1 {
                        sum += px[row + sx * n + c] as u32;
                    }
                }
                out[(y * nw + x) * n + c] = ((sum + count / 2) / count) as u8;
            }
        }
    }
    out
}

fn flate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Decode an image stream to 8-bit pixels: (pixels, channels, was_jpeg).
fn decode(doc: &PdfDoc, s: &Stream) -> Option<(Vec<u8>, usize, bool)> {
    let d = &s.dict;
    if d.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false) || d.get(b"Decode").is_ok() || int(d, b"BitsPerComponent") != 8 {
        return None;
    }
    let n = components(doc, d)?;
    let (w, h) = (int(d, b"Width") as usize, int(d, b"Height") as usize);
    match filters(d).iter().map(|f| f.as_slice()).collect::<Vec<_>>().as_slice() {
        [b"DCTDecode"] => {
            let mut dec = jpeg_decoder::Decoder::new(std::io::Cursor::new(&s.content));
            let px = dec.decode().ok()?;
            let info = dec.info()?;
            let ch = match info.pixel_format {
                jpeg_decoder::PixelFormat::L8 => 1,
                jpeg_decoder::PixelFormat::RGB24 => 3,
                _ => return None,
            };
            (ch == n && info.width as usize == w && info.height as usize == h).then_some((px, n, true))
        }
        [b"FlateDecode"] | [] => {
            let px = if filters(d).is_empty() { s.content.clone() } else { s.decompressed_content().ok()? };
            (px.len() >= w * h * n).then(|| (px[..w * h * n].to_vec(), n, false))
        }
        _ => None,
    }
}

/// Downsample images above `target_dpi` (only those drawn at > 1.25× it).
/// `quality` (1–100) is used for images that were JPEG.
pub fn downsample_images(doc: &mut PdfDoc, target_dpi: f64, quality: u8) -> DownsampleReport {
    let mut rep = DownsampleReport::default();
    for (id, dpi) in image_dpi(doc) {
        if dpi <= target_dpi * 1.25 {
            continue;
        }
        let Ok(Object::Stream(s)) = doc.get_object(id).cloned() else { continue };
        let (w, h) = (int(&s.dict, b"Width") as usize, int(&s.dict, b"Height") as usize);
        let scale = target_dpi / dpi;
        let (nw, nh) = (((w as f64 * scale).round() as usize).max(1), ((h as f64 * scale).round() as usize).max(1));
        if w * h < 4096 || nw >= w || nh > 65_000 {
            continue;
        }
        let Some((px, n, jpeg)) = decode(doc, &s) else {
            rep.skipped += 1;
            continue;
        };
        // A soft mask (transparency) is resampled to the same size, or the image is left alone.
        let smask = match s.dict.get(b"SMask") {
            Ok(Object::Reference(m)) => match doc.get_object(*m).ok().and_then(|o| o.as_stream().ok()).cloned() {
                Some(ms) if int(&ms.dict, b"Width") as usize == w && int(&ms.dict, b"Height") as usize == h => match decode(doc, &ms) {
                    Some((mpx, 1, _)) => Some((*m, ms, mpx)),
                    _ => {
                        rep.skipped += 1;
                        continue;
                    }
                },
                _ => {
                    rep.skipped += 1;
                    continue;
                }
            },
            Ok(_) => {
                rep.skipped += 1;
                continue;
            }
            Err(_) => None,
        };
        let small = resample(&px, w, h, n, nw, nh);
        let (data, filter) = if jpeg {
            let mut out = Vec::new();
            let ct = if n == 1 { jpeg_encoder::ColorType::Luma } else { jpeg_encoder::ColorType::Rgb };
            if jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100)).encode(&small, nw as u16, nh as u16, ct).is_err() {
                rep.skipped += 1;
                continue;
            }
            (out, "DCTDecode")
        } else {
            (flate(&small), "FlateDecode")
        };
        if data.len() >= s.content.len() {
            continue; // not worth it
        }
        rep.before += s.content.len();
        rep.after += data.len();
        rep.images += 1;
        let mut dict = s.dict.clone();
        dict.set("Width", nw as i64);
        dict.set("Height", nh as i64);
        dict.set("Filter", Object::Name(filter.as_bytes().to_vec()));
        dict.remove(b"DecodeParms");
        dict.set("Length", data.len() as i64);
        let mut ns = Stream::new(dict, data);
        ns.allows_compression = false;
        doc.objects.insert(id, Object::Stream(ns));
        if let Some((mid, ms, mpx)) = smask {
            let msmall = resample(&mpx, w, h, 1, nw, nh);
            let data = flate(&msmall);
            let mut md = ms.dict.clone();
            md.set("Width", nw as i64);
            md.set("Height", nh as i64);
            md.set("Filter", "FlateDecode");
            md.remove(b"DecodeParms");
            md.set("Length", data.len() as i64);
            let mut m = Stream::new(md, data);
            m.allows_compression = false;
            doc.objects.insert(mid, Object::Stream(m));
        }
    }
    rep
}
