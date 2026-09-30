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

pub use error::{EngineError, ErrorCode};
pub use model::{
    build_open_info, collect_pages, page_rotation, read_metadata, Metadata, OpenInfo, PageMeta,
};
pub use ops::{
    commit_export, delete_pages, extract, insert_blank, merge_doc, open_bytes, open_path, reorder,
    rotate_pages, set_metadata, unlock_document, CommitResult,
};