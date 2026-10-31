//! Format versioning: bring an older document up to [`FORMAT_VERSION`].
//!
//! Every project file carries the schema version it was written with. Loading runs the
//! document through a chain of one-step upgrades until it reaches the current version, so
//! old patches keep opening as the format evolves. Documents written by a *newer* build
//! are rejected rather than guessed at.

use crate::doc::{ProjectDoc, FORMAT_VERSION, MIN_SUPPORTED_VERSION};
use crate::error::ProjectError;

/// Applies a single `version` → `version + 1` upgrade step.
///
/// New format revisions add an arm here that rewrites the document and bumps
/// `version`. Version 1 is the first released format, so there is nothing to upgrade
/// from yet and the document is returned unchanged.
fn upgrade_once(doc: ProjectDoc) -> ProjectDoc {
    doc
}

/// Migrates `doc` to [`FORMAT_VERSION`].
///
/// # Errors
///
/// Returns [`ProjectError::UnsupportedVersion`] if the document is newer than this build
/// understands, older than [`MIN_SUPPORTED_VERSION`], or if an upgrade step fails to make
/// progress.
pub fn migrate_to_current(mut doc: ProjectDoc) -> Result<ProjectDoc, ProjectError> {
    if doc.version > FORMAT_VERSION || doc.version < MIN_SUPPORTED_VERSION {
        return Err(ProjectError::UnsupportedVersion {
            found: doc.version,
            min_supported: MIN_SUPPORTED_VERSION,
            current: FORMAT_VERSION,
        });
    }

    while doc.version < FORMAT_VERSION {
        let before = doc.version;
        doc = upgrade_once(doc);
        if doc.version <= before {
            // A step that does not advance the version would loop forever.
            return Err(ProjectError::UnsupportedVersion {
                found: before,
                min_supported: MIN_SUPPORTED_VERSION,
                current: FORMAT_VERSION,
            });
        }
    }

    Ok(doc)
}
