//! Advance widths (1/1000 em) of the standard-14 Latin fonts for ASCII
//! 32..=126 (Adobe Core14 AFM metrics). Needed because PDFs may use these
//! fonts without a /Widths array — readers have the metrics built in, and so
//! must we to place glyphs (redaction, moving text, matching edits).

#[rustfmt::skip]
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556,
    556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278,
    500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469,
    556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
    278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

#[rustfmt::skip]
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556,
    556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278,
    556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584,
    556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556,
    333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

#[rustfmt::skip]
const TIMES_ROMAN: [u16; 95] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500, 500, 500, 500,
    500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667, 722, 611, 556, 722, 722, 333,
    389, 722, 611, 889, 722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469,
    500, 333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500, 500, 500, 333, 389,
    278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
];

#[rustfmt::skip]
const TIMES_BOLD: [u16; 95] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500, 500, 500,
    500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 930, 722, 667, 722, 722, 667, 611, 778, 778, 389,
    500, 778, 667, 944, 722, 778, 611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581,
    500, 333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500, 556, 556, 444, 389,
    333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520,
];

/// Widths table and default (for codes outside 32..=126) for a base font
/// name, recognising subset prefixes and common aliases (Arial, Times New Roman).
pub fn table(base_font: &str) -> Option<(&'static [u16; 95], u16)> {
    let b = base_font.split_once('+').map(|(_, r)| r).unwrap_or(base_font).to_lowercase().replace([' ', '-', ','], "");
    let bold = b.contains("bold") || b.contains("black");
    if b.starts_with("courier") {
        return Some((&[600; 95], 600));
    }
    if b.starts_with("helvetica") || b.starts_with("arial") {
        return Some(if bold { (&HELVETICA_BOLD, 556) } else { (&HELVETICA, 556) });
    }
    if b.starts_with("times") {
        return Some(if bold { (&TIMES_BOLD, 500) } else { (&TIMES_ROMAN, 500) });
    }
    None
}

/// Width of a single code in a standard font.
pub fn width(base_font: &str, code: u32) -> Option<f64> {
    let (t, def) = table(base_font)?;
    Some(if (32..=126).contains(&code) { t[(code - 32) as usize] } else { def } as f64)
}
