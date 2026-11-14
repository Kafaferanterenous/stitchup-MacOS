//! Stitchup structural core (D-05 lopdf backend).
//!
//! Mirrors `docs/STAGE0_DOCUMENT_MODEL.md` and `docs/STAGE0_ENGINE_INTERFACES.md`
//! (OpenDocumentAdapter + PageTreeAdapter + MetadataAdapter + CommitAdapter on
//! the structural side). Rendering is a frontend concern (PDF.js/pdf-lib); this
//! crate owns open/parse integrity, page-tree mutation, metadata, and ATOMIC
//! save-as.

pub mod error;
pub mod hash;
pub mod model;
pub mod oplog;
pub mod ops;
pub mod textedit;
pub mod docops;
pub mod sign;
pub mod annots;
pub mod objects;
pub mod std14;
pub mod forms;
pub mod ocr;
pub mod fonts;
pub mod images;

pub use error::{EngineError, ErrorCode};
pub use model::{
    build_open_info, collect_pages, page_rotation, read_metadata, Metadata, OpenInfo, PageMeta,
};
pub use ops::{
    commit_export, delete_pages, extract, flatten_page_tree, insert_blank, split, merge_doc, open_bytes, open_path, reorder,
    rotate_pages, set_metadata, unlock_document, CommitResult,
};