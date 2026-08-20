//! `vb-library` — the patch/preset library and AI knowledge base for VST_bomb (Epic 11).
//!
//! The library stores metadata and links for patches, macro-nodes and presets in
//! [drevo](https://github.com/) — the user's embedded graph database — while the RON
//! project files stay the source of truth for the audio structure. This crate is the
//! seam between VST_bomb and drevo.
//!
//! **drevo is optional.** Everything here lives behind the `library` Cargo feature, which
//! pulls drevo in via its *direct Rust API* (embedded `redb-backend`, no Bolt server). The
//! default build — and the required CI gate — does not compile drevo at all, so a checkout
//! without the sibling `drevo` crate still builds the rest of the workspace.
//!
//! This is the integration-assessment step (backlog 000093): it opens an embedded store
//! and proves basic CRUD works. The storage schema (patch / macro-node / preset nodes and
//! their links) and the import path land in 000094 / 000095.

#![cfg_attr(not(feature = "library"), allow(unused))]

/// Re-export of the embedded drevo API, so downstream code has a single, feature-gated
/// path to the graph store rather than depending on drevo directly.
#[cfg(feature = "library")]
pub use drevo;

#[cfg(feature = "library")]
pub mod backup;

#[cfg(feature = "library")]
pub mod import;

#[cfg(feature = "library")]
pub mod knowledge;

#[cfg(feature = "library")]
pub mod library;

#[cfg(feature = "library")]
pub mod preset;

#[cfg(feature = "library")]
pub mod rag;

#[cfg(feature = "library")]
pub use import::{import_patch, patches_with_node_kind};

#[cfg(feature = "library")]
pub use knowledge::OntologyReport;

#[cfg(feature = "library")]
pub use library::{kind, rel, Library};

/// A relevance-scored node from a full-text search (re-exported from drevo).
#[cfg(feature = "library")]
pub use drevo::model::ScoredNode;

/// Backup/migration types (re-exported from drevo).
#[cfg(feature = "library")]
pub use drevo::{db::MigrationDirection, dump::ImportReport};

/// Opens a fresh in-memory library store (no file, no server) — the embedded direct API.
///
/// Used by the smoke test and handy for transient/testing use; persistent stores open a
/// path with [`drevo::db::Drevo::open`].
///
/// # Errors
/// Propagates any [`drevo`] error from opening the store.
#[cfg(feature = "library")]
pub fn open_in_memory() -> drevo::error::Result<drevo::db::Drevo> {
    drevo::db::Drevo::open_in_memory()
}
