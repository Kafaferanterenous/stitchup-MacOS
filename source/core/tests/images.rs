use stitchup_core::docops::{compressed_bytes_with, new_blank};
use stitchup_core::objects::{insert_image, list, ImageData};

fn photo(w: usize, h: usize) -> Vec<u8> {
    // Smooth gradients + texture, like a photo.
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            px.push(((x * 255) / w) as u8);
            px.push(((y * 255) / h) as u8);
            px.push((((x ^ y) & 31) * 4) as u8);
        }
    }
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 95).encode(&px, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    out
}

#[test]
fn compress_downsamples_oversized_images_and_keeps_them_in_place() {
    let mut doc = new_blank(612.0, 792.0).unwrap();
    // 2400 px drawn 200 pt wide = 864 dpi.
    insert_image(&mut doc, 1, [72.0, 500.0, 272.0, 700.0], &ImageData::Jpeg { bytes: photo(2400, 2400) }).unwrap();
    // RGBA (Flate + soft mask) 1200 px drawn 150 pt wide = 576 dpi.
    let mut rgba = Vec::with_capacity(1200 * 1200 * 4);
    for y in 0..1200usize {
        for x in 0..1200usize {
            rgba.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128, if (x / 100 + y / 100) % 2 == 0 { 255 } else { 90 }]);
        }
    }
    insert_image(&mut doc, 1, [300.0, 500.0, 450.0, 650.0], &ImageData::Rgba { width: 1200, height: 1200, pixels: rgba }).unwrap();
    // Small image already at low resolution: untouched.
    insert_image(&mut doc, 1, [72.0, 100.0, 372.0, 400.0], &ImageData::Jpeg { bytes: photo(300, 300) }).unwrap();
    let before_rects: Vec<_> = list(&doc, 1).unwrap().into_iter().filter(|o| o.kind == "image").map(|o| o.rect).collect();

    let (plain, _) = compressed_bytes_with(&doc, None, 75).unwrap();
    let (small, rep) = compressed_bytes_with(&doc, Some(150.0), 75).unwrap();
    assert_eq!(rep.images, 2, "{rep:?}");
    assert!(small.len() * 3 < plain.len(), "{} → {}", plain.len(), small.len());

    let back = lopdf::Document::load_mem(&small).unwrap();
    let mut widths: Vec<i64> = back
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Subtype").and_then(|n| n.as_name()).ok() == Some(b"Image".as_slice()))
        .map(|s| s.dict.get(b"Width").unwrap().as_i64().unwrap())
        .collect();
    widths.sort();
    // 2400 → 417 (150 dpi over 200 pt), 1200 → 313 (+ its mask), 300 untouched.
    assert_eq!(widths, vec![300, 313, 313, 417], "{widths:?}");
    let after_rects: Vec<_> = list(&back, 1).unwrap().into_iter().filter(|o| o.kind == "image").map(|o| o.rect).collect();
    assert_eq!(before_rects, after_rects, "images stay where they were");
}
