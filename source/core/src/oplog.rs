//! Operation log — exact structural undo/redo (A-c8).
//!
//! Every mutating command records a before + after snapshot of everything the
//! op touches: the root Pages node `/Kids` + `/Count`, every page dictionary
//! (cloned), and the trailer `/Info` dictionary. Applying a record is a
//! full-state replacement (`before` for undo, `after` for redo), so a mixed
//! sequence (reorder + rotate + delete + insert + edit_text + set_metadata)
//! reverts EXACTLY at every step regardless of op ordering. Page object ids are
//! stable across ops and are never garbage-collected, so re-inserting a
//! snapshot's kid refs always resolves.

use lopdf::{Document as PdfDoc, Object};

use crate::error::{EngineError, ErrorCode};

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Root Pages node `/Kids` (id, gen) in current order.
    pub kids: Vec<(u32, u16)>,
    /// Cloned page dictionaries for the pages currently in `/Kids`.
    pub pages: Vec<(u32, Object)>,
    /// Cloned `/Info` dictionary (or its inline object), if the trailer has one.
    pub info: Option<Object>,
}

/// One logged mutation with enough state to replay or fully revert it.
#[derive(Debug, Clone)]
pub struct OpRecord {
    pub label: String,
    pub before: Snapshot,
    pub after: Snapshot,
}

/// Undo/redo stacks. `undo` is a true LIFO history of applied mutations;
/// popping one applies its `before` side and moves it to `redo`. Any NEW
/// mutation clears `redo` (standard branching-editor semantics).
#[derive(Debug, Clone, Default)]
pub struct OpLog {
    pub undo: Vec<OpRecord>,
    pub redo: Vec<OpRecord>,
}

impl OpLog {
    pub fn push(&mut self, rec: OpRecord) {
        self.undo.push(rec);
        self.redo.clear();
    }
    pub fn pop_undo(&mut self) -> Option<OpRecord> {
        self.undo.pop()
    }
    pub fn pop_redo(&mut self) -> Option<OpRecord> {
        self.redo.pop()
    }
}

fn catalog_pages_id(doc: &PdfDoc) -> Result<(u32, u16), EngineError> {
    let root_ref = doc.trailer.get(b"Root")?;
    let root_id = root_ref.as_reference()?;
    let catalog = doc.get_object(root_id)?;
    let pages_ref = catalog.as_dict()?.get(b"Pages")?;
    pages_ref.as_reference().map_err(Into::into)
}

/// Capture the structural state of *everything a logged op could touch*.
pub fn snapshot(doc: &PdfDoc) -> Result<Snapshot, EngineError> {
    let pages_id = catalog_pages_id(doc)?;
    let node = doc.get_object(pages_id)?;
    let kids_arr = node.as_dict()?.get(b"Kids")?.as_array()?;
    let kids = kids_arr
        .iter()
        .map(|o| o.as_reference().map_err(|e| crate::err!(ErrorCode::Malformed, "kid is not a reference: {e}")))
        .collect::<Result<Vec<_>, _>>()?;

    let mut pages = Vec::with_capacity(kids.len());
    for (id, _gen) in kids.iter() {
        if let Some(obj) = doc.objects.get(&(*id, 0)) {
            pages.push((*id, obj.clone()));
        }
    }

    let info = match doc.trailer.get(b"Info") {
        Ok(id) => match id.as_reference() {
            Ok(r) => doc.objects.get(&r).cloned(),
            Err(_) => Some(id.clone()),
        },
        Err(_) => None,
    };

    Ok(Snapshot { kids, pages, info })
}

/// Apply one side of a record to the document (full replacement semantics).
/// `use_before == true` reverts (undo); `false` replays (redo).
pub fn apply(doc: &mut PdfDoc, rec: &OpRecord, use_before: bool) -> Result<(), EngineError> {
    let snap: &Snapshot = if use_before { &rec.before } else { &rec.after };

    let pages_id = catalog_pages_id(doc)?;
    let node_id = (pages_id.0, 0);
    let node = doc
        .objects
        .get_mut(&node_id)
        .ok_or_else(|| crate::err!(ErrorCode::Malformed, "pages node missing"))?;
    let dict = node.as_dict_mut()?;
    dict.set(
        "Kids",
        Object::Array(snap.kids.iter().map(|&r| Object::Reference(r)).collect()),
    );
    dict.set("Count", Object::Integer(snap.kids.len() as i64));

    for (id, obj) in snap.pages.iter() {
        doc.objects.insert((*id, 0), obj.clone());
    }

    let info_ref = doc.trailer.get(b"Info").ok().and_then(|o| o.as_reference().ok());
    let info_target = snap.info.clone();
    match info_target {
        // Restore the Info dictionary into the object the trailer already points at.
        Some(obj) => match info_ref {
            Some(r) => {
                doc.objects.insert(r, obj);
            }
            // No usable reference (absent or inline): (re)create one.
            None => {
                let id = doc.add_object(obj);
                doc.trailer.set("Info", Object::Reference(id));
            }
        },
        // The target state has no /Info at all. A metadata edit creates one, so
        // undo must remove it (and drop the object the trailer pointed at) to
        // revert exactly; without this the created Info is left behind.
        None => {
            if doc.trailer.get(b"Info").is_ok() {
                if let Some(r) = info_ref {
                    doc.objects.remove(&r);
                }
                doc.trailer.remove(b"Info");
            }
        }
    }

    Ok(())
}