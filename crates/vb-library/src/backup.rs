//! Library export / backup and schema migrations (backlog 000100).
//!
//! The whole library graph round-trips through drevo's JSON dump format: [`Library::export_json`]
//! writes a portable snapshot, and [`Library::import_json`] merges one back (idempotent —
//! byte-equal nodes/edges are skipped). drevo owns the on-disk *schema* version and
//! migrates a store forward automatically when it is opened; [`Library::migrate`] exposes an
//! explicit up/down migration for a persistent store when you need to force it.

use std::path::Path;

use drevo::db::{Drevo, MigrationDirection};
use drevo::dump::ImportReport;
use drevo::error::Result;

use crate::library::Library;

impl Library {
    /// Exports the whole library as a portable JSON snapshot (a backup).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn export_json(&self) -> Result<String> {
        self.db().export_json()
    }

    /// Writes a JSON backup of the library to `path`.
    ///
    /// # Errors
    /// Propagates any drevo error (including I/O).
    pub fn export_json_to_path(&self, path: &Path) -> Result<()> {
        self.db().export_json_to_path(path)
    }

    /// Merges a JSON snapshot into this library. Idempotent: nodes/edges that already
    /// exist byte-for-byte are skipped, so restoring the same backup twice is safe.
    ///
    /// # Errors
    /// Propagates any drevo error (including a malformed dump).
    pub fn import_json(&self, raw: &str) -> Result<ImportReport> {
        self.db().import_json(raw)
    }

    /// Merges a JSON backup file at `path` into this library.
    ///
    /// # Errors
    /// Propagates any drevo error (including I/O).
    pub fn import_json_from_path(&self, path: &Path) -> Result<ImportReport> {
        self.db().import_json_from_path(path)
    }

    /// Migrates the on-disk schema of a persistent library at `path` up or down, returning
    /// the number of records rewritten. drevo also migrates forward automatically on open;
    /// this is the explicit control (e.g. a `Down` migration so an older build can read the
    /// store). Not applicable to an in-memory library.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn migrate(path: &Path, direction: MigrationDirection) -> Result<u64> {
        Drevo::migrate(path, direction)
    }
}
