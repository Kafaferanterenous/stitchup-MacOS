//! AcroForm filling (PDF 32000 §12.7): list the fields, set a value, and
//! regenerate the widget appearance so the filled form looks right in every
//! viewer and when printed (no reliance on /NeedAppearances).
//!
//! Field dictionaries are modified in place, so callers snapshot
//! [`touched`] ids for undo (see `oplog::with_extra`).

use std::collections::HashMap;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Dictionary, Document as PdfDoc, Object, ObjectId, Stream, StringFormat};
use serde::{Deserialize, Serialize};

use crate::error::{EngineError, ErrorCode};

#[derive(Debug, Clone, Serialize)]
pub struct Widget {
    pub id: (u32, u16),
    pub page: u32,
    pub rect: [f64; 4],
    /// Checkbox / radio "on" appearance name for this widget.
    pub on_state: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Field {
    /// Fully qualified name (parent.child).
    pub name: String,
    /// "text" | "checkbox" | "radio" | "choice" | "button" | "signature"
    pub kind: String,
    pub value: String,
    pub options: Vec<String>,
    pub multiline: bool,
    pub readonly: bool,
    pub widgets: Vec<Widget>,
    #[serde(skip)]
    pub field_id: Option<ObjectId>,
    #[serde(skip)]
    da: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum FieldValue {
    Bool(bool),
    Text(String),
}

fn deref<'a>(doc: &'a PdfDoc, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok(),
        o => Some(o),
    }
}

fn text_of(o: Option<&Object>) -> String {
    match o {
        Some(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        Some(o @ Object::String(..)) => lopdf::decode_text_string(o).unwrap_or_default(),
        _ => String::new(),
    }
}

fn rect_of(d: &Dictionary) -> [f64; 4] {
    let v: Vec<f64> = d.get(b"Rect").ok().and_then(|o| o.as_array().ok()).map(|a| a.iter().filter_map(|x| x.as_float().ok().map(|f| f as f64)).collect()).unwrap_or_default();
    if v.len() == 4 {
        [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])]
    } else {
        [0.0; 4]
    }
}

fn on_state(doc: &PdfDoc, w: &Dictionary) -> Option<String> {
    let ap = w.get(b"AP").ok().and_then(|o| deref(doc, o))?.as_dict().ok()?;
    let n = ap.get(b"N").ok().and_then(|o| deref(doc, o))?.as_dict().ok()?;
    n.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()).find(|k| k != "Off")
}

/// Widget id → page number.
fn widget_pages(doc: &PdfDoc) -> HashMap<ObjectId, u32> {
    let mut m = HashMap::new();
    for (n, pid) in doc.get_pages() {
        let annots = doc.get_object(pid).ok().and_then(|p| p.as_dict().ok()).and_then(|d| d.get(b"Annots").ok()).and_then(|o| deref(doc, o)).and_then(|o| o.as_array().ok());
        for a in annots.into_iter().flatten() {
            if let Ok(id) = a.as_reference() {
                m.insert(id, n);
            }
        }
    }
    m
}

struct Inherit {
    name: String,
    ft: Vec<u8>,
    ff: i64,
    da: String,
    opt: Vec<String>,
}

pub fn list(doc: &PdfDoc) -> Vec<Field> {
    let pages = widget_pages(doc);
    let mut out = vec![];
    let fields = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .and_then(|f| f.get(b"Fields").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
        .cloned()
        .unwrap_or_default();
    let da_default = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
        .and_then(|f| f.get(b"DA").ok())
        .map(|o| text_of(Some(o)))
        .unwrap_or_else(|| "/Helv 0 Tf 0 g".into());
    for f in &fields {
        walk(doc, f, &Inherit { name: String::new(), ft: vec![], ff: 0, da: da_default.clone(), opt: vec![] }, &pages, &mut out, 0);
    }
    out
}

fn walk(doc: &PdfDoc, node: &Object, parent: &Inherit, pages: &HashMap<ObjectId, u32>, out: &mut Vec<Field>, depth: u32) {
    if depth > 16 {
        return;
    }
    let id = node.as_reference().ok();
    let Some(d) = deref(doc, node).and_then(|o| o.as_dict().ok()) else { return };
    let partial = text_of(d.get(b"T").ok());
    let name = match (parent.name.is_empty(), partial.is_empty()) {
        (true, _) => partial.clone(),
        (false, true) => parent.name.clone(),
        (false, false) => format!("{}.{}", parent.name, partial),
    };
    let inh = Inherit {
        name,
        ft: d.get(b"FT").ok().and_then(|o| o.as_name().ok()).map(|n| n.to_vec()).unwrap_or_else(|| parent.ft.clone()),
        ff: d.get(b"Ff").ok().and_then(|o| o.as_i64().ok()).unwrap_or(parent.ff),
        da: d.get(b"DA").ok().map(|o| text_of(Some(o))).unwrap_or_else(|| parent.da.clone()),
        opt: d
            .get(b"Opt")
            .ok()
            .and_then(|o| deref(doc, o))
            .and_then(|o| o.as_array().ok())
            .map(|a| {
                a.iter()
                    .map(|x| match deref(doc, x) {
                        Some(Object::Array(pair)) => text_of(pair.first()),
                        other => text_of(other),
                    })
                    .collect()
            })
            .unwrap_or_else(|| parent.opt.clone()),
    };
    let kids: Vec<Object> = d.get(b"Kids").ok().and_then(|o| deref(doc, o)).and_then(|o| o.as_array().ok()).cloned().unwrap_or_default();
    // Kids with their own /T are child fields; otherwise they are widgets.
    let child_fields = kids.iter().any(|k| deref(doc, k).and_then(|o| o.as_dict().ok()).map(|kd| kd.has(b"T")).unwrap_or(false));
    if child_fields {
        for k in &kids {
            walk(doc, k, &inh, pages, out, depth + 1);
        }
        return;
    }
    let widget_objs: Vec<Object> = if kids.is_empty() { id.map(Object::Reference).into_iter().collect() } else { kids };
    let widgets: Vec<Widget> = widget_objs
        .iter()
        .filter_map(|w| {
            let wid = w.as_reference().ok()?;
            let wd = deref(doc, w)?.as_dict().ok()?;
            let page = pages.get(&wid).copied().or_else(|| {
                let p = wd.get(b"P").ok()?.as_reference().ok()?;
                doc.get_pages().into_iter().find(|(_, pid)| *pid == p).map(|(n, _)| n)
            })?;
            Some(Widget { id: wid, page, rect: rect_of(wd), on_state: on_state(doc, wd) })
        })
        .collect();
    let ff = inh.ff;
    let kind = match inh.ft.as_slice() {
        b"Tx" => "text",
        b"Ch" => "choice",
        b"Sig" => "signature",
        b"Btn" if ff & (1 << 16) != 0 => "button",
        b"Btn" if ff & (1 << 15) != 0 => "radio",
        b"Btn" => "checkbox",
        _ => return,
    };
    out.push(Field {
        name: inh.name,
        kind: kind.into(),
        value: text_of(d.get(b"V").ok().and_then(|o| deref(doc, o))),
        options: inh.opt,
        multiline: kind == "text" && ff & (1 << 12) != 0,
        readonly: ff & 1 != 0,
        widgets,
        field_id: id,
        da: inh.da,
    });
}

/// Objects `set_value` will modify for `name` (for undo snapshots).
pub fn touched(doc: &PdfDoc, name: &str) -> Vec<ObjectId> {
    let mut ids = vec![];
    if let Some(f) = list(doc).into_iter().find(|f| f.name == name) {
        ids.extend(f.field_id);
        ids.extend(f.widgets.iter().map(|w| w.id));
    }
    ids.sort();
    ids.dedup();
    ids
}

/// Font size and colour from a /DA string ("/Helv 12 Tf 0 0 1 rg").
fn parse_da(da: &str) -> (f64, Vec<Operation>) {
    let t: Vec<&str> = da.split_whitespace().collect();
    let mut size = 0.0;
    let mut color = vec![Operation::new("g", vec![0.into()])];
    for (i, w) in t.iter().enumerate() {
        match *w {
            "Tf" if i >= 1 => size = t[i - 1].parse().unwrap_or(0.0),
            "g" if i >= 1 => color = vec![Operation::new("g", vec![Object::Real(t[i - 1].parse().unwrap_or(0.0))])],
            "rg" if i >= 3 => {
                color = vec![Operation::new("rg", (1..=3).rev().map(|k| Object::Real(t[i - k].parse().unwrap_or(0.0))).collect())]
            }
            _ => {}
        }
    }
    (size, color)
}

/// Text appearance for one widget: the value in Helvetica (Polish letters
/// supported), auto-sized when /DA says 0, wrapped when multi-line.
fn text_appearance(doc: &mut PdfDoc, rect: [f64; 4], value: &str, da: &str, multiline: bool) -> Result<ObjectId, EngineError> {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let (da_size, color) = parse_da(da);
    // Like a real viewer: honour /DA, but auto-fit (0) and never overflow the box.
    let wrap = |size: f64| -> Vec<String> {
        let mut out = vec![];
        for para in value.split('\n') {
            let mut cur = String::new();
            for word in para.split(' ') {
                let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                if !cur.is_empty() && crate::docops::helv_bold_width(&trial, size) * 0.93 > w - 4.0 {
                    out.push(std::mem::take(&mut cur));
                    cur = word.to_string();
                } else {
                    cur = trial;
                }
            }
            out.push(cur);
        }
        out
    };
    let mut size = if da_size > 0.0 { da_size } else if multiline { 12.0 } else { 12.0_f64.min((h - 2.0) * 0.75) };
    let lines: Vec<String> = if multiline {
        loop {
            let l = wrap(size);
            if size <= 6.0 || (l.len() as f64) * size * 1.15 <= h - 3.0 {
                break l;
            }
            size -= 0.5;
        }
    } else {
        let one = value.replace('\n', " ");
        size = size.min((h - 2.0) * 0.75);
        while size > 6.0 && crate::docops::helv_bold_width(&one, size) * 0.93 > w - 4.0 {
            size -= 0.5;
        }
        vec![one]
    };
    let joined = lines.join("\n");
    let enc = crate::textedit::std14_encode(&joined);
    let r = |v: f64| Object::Real(v as f32);
    let mut ops = vec![
        Operation::new("BMC", vec![Object::Name(b"Tx".to_vec())]),
        Operation::new("q", vec![]),
        Operation::new("re", vec![r(1.0), r(1.0), r(w - 2.0), r(h - 2.0)]),
        Operation::new("W", vec![]),
        Operation::new("n", vec![]),
        Operation::new("BT", vec![]),
    ];
    ops.extend(color);
    ops.push(Operation::new("Tf", vec![Object::Name(b"StHv".to_vec()), r(size)]));
    let mut y = if multiline { h - 2.0 - size } else { (h - size * 0.72) / 2.0 };
    let mut pos = 0;
    for l in &lines {
        let n = l.chars().count();
        let bytes = enc.bytes[pos..pos + n].to_vec();
        pos += n + 1;
        ops.push(Operation::new("Tm", vec![r(1.0), r(0.0), r(0.0), r(1.0), r(2.0), r(y)]));
        ops.push(Operation::new("Tj", vec![Object::String(bytes, StringFormat::Literal)]));
        y -= size * 1.15;
    }
    ops.extend([Operation::new("ET", vec![]), Operation::new("Q", vec![]), Operation::new("EMC", vec![])]);
    let data = Content { operations: ops }.encode()?;
    Ok(doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), r(w), r(h)],
            "Resources" => dictionary! { "Font" => dictionary! { "StHv" => crate::textedit::std14_font("Helvetica", &enc.diffs) } },
        },
        data,
    )))
}

fn dict_mut(doc: &mut PdfDoc, id: ObjectId) -> Result<&mut Dictionary, EngineError> {
    Ok(doc.get_object_mut(id)?.as_dict_mut()?)
}

/// Set a field's value (text/choice: string; checkbox: bool; radio: the
/// chosen button's on-state name) and redraw its widgets.
pub fn set_value(doc: &mut PdfDoc, name: &str, value: &FieldValue) -> Result<(), EngineError> {
    let f = list(doc).into_iter().find(|f| f.name == name).ok_or_else(|| crate::err!(ErrorCode::ValidationFailed, "no field {name}"))?;
    if f.readonly {
        return Err(crate::err!(ErrorCode::PermissionDenied, "field {name} is read-only"));
    }
    let field_id = f.field_id.or_else(|| f.widgets.first().map(|w| w.id)).ok_or_else(|| crate::err!(ErrorCode::Malformed, "field {name} has no object"))?;
    match (f.kind.as_str(), value) {
        ("text" | "choice", FieldValue::Text(t)) => {
            dict_mut(doc, field_id)?.set("V", lopdf::text_string(t));
            for w in &f.widgets {
                let ap = text_appearance(doc, w.rect, t, &f.da, f.multiline)?;
                dict_mut(doc, w.id)?.set("AP", dictionary! { "N" => ap });
            }
        }
        ("checkbox", FieldValue::Bool(on)) => {
            // Widgets without an appearance get a drawn check mark (on) / empty box (off).
            let mut f = f.clone();
            for w in f.widgets.iter_mut().filter(|w| w.on_state.is_none()) {
                check_appearance(doc, w.id, w.rect, "Yes")?;
                w.on_state = Some("Yes".into());
            }
            let on_name = f.widgets.iter().find_map(|w| w.on_state.clone()).unwrap_or_else(|| "Yes".into());
            let v = if *on { on_name.clone() } else { "Off".into() };
            dict_mut(doc, field_id)?.set("V", Object::Name(v.clone().into_bytes()));
            for w in &f.widgets {
                let st = if *on && w.on_state.is_some() { w.on_state.clone().unwrap() } else { "Off".into() };
                dict_mut(doc, w.id)?.set("AS", Object::Name(st.into_bytes()));
            }
        }
        ("radio", FieldValue::Text(choice)) => {
            if !f.widgets.iter().any(|w| w.on_state.as_deref() == Some(choice.as_str())) && choice != "Off" {
                return Err(crate::err!(ErrorCode::ValidationFailed, "{name} has no option {choice}"));
            }
            dict_mut(doc, field_id)?.set("V", Object::Name(choice.clone().into_bytes()));
            for w in &f.widgets {
                let st = if w.on_state.as_deref() == Some(choice.as_str()) { choice.clone() } else { "Off".into() };
                dict_mut(doc, w.id)?.set("AS", Object::Name(st.into_bytes()));
            }
        }
        (kind, _) => return Err(crate::err!(ErrorCode::Unsupported, "can't set a {kind} field to that value")),
    }
    Ok(())
}

/// On/Off appearances for a checkbox widget that has none.
fn check_appearance(doc: &mut PdfDoc, widget: ObjectId, rect: [f64; 4], on: &str) -> Result<(), EngineError> {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let r = |v: f64| Object::Real(v as f32);
    let bbox = vec![0.into(), 0.into(), r(w), r(h)];
    let s = w.min(h);
    let (ox, oy) = ((w - s) / 2.0, (h - s) / 2.0);
    let check = vec![
        Operation::new("q", vec![]),
        Operation::new("w", vec![r((s * 0.12).max(0.8))]),
        Operation::new("J", vec![1.into()]),
        Operation::new("j", vec![1.into()]),
        Operation::new("RG", vec![0.into(), 0.into(), 0.into()]),
        Operation::new("m", vec![r(ox + s * 0.2), r(oy + s * 0.52)]),
        Operation::new("l", vec![r(ox + s * 0.42), r(oy + s * 0.28)]),
        Operation::new("l", vec![r(ox + s * 0.82), r(oy + s * 0.76)]),
        Operation::new("S", vec![]),
        Operation::new("Q", vec![]),
    ];
    let on_id = doc.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox.clone() },
        Content { operations: check }.encode()?,
    ));
    let off_id = doc.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => bbox }, Vec::new()));
    let mut n = Dictionary::new();
    n.set(on.as_bytes().to_vec(), on_id);
    n.set("Off", off_id);
    dict_mut(doc, widget)?.set("AP", dictionary! { "N" => n });
    Ok(())
}
