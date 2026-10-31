//! Reading and writing project documents as RON.
//!
//! Two levels are offered. [`from_ron`] / [`load_from_path`] parse only — useful for
//! tooling that wants to inspect or repair a file. [`open_ron`] / [`open_path`] are the
//! trustworthy path: parse, migrate to the current format, then validate, so what comes
//! back is a document the runtime can actually build.

use std::path::Path;

use crate::doc::ProjectDoc;
use crate::error::ProjectError;
use crate::migrate::migrate_to_current;

/// Serializes a project to pretty RON text.
///
/// # Errors
///
/// Returns [`ProjectError::Serialize`] if RON serialization fails.
pub fn to_ron(doc: &ProjectDoc) -> Result<String, ProjectError> {
    ron::ser::to_string_pretty(doc, ron::ser::PrettyConfig::default())
        .map_err(|e| ProjectError::Serialize(e.to_string()))
}

/// Parses a project from RON text without migrating or validating it.
///
/// # Errors
///
/// Returns [`ProjectError::Deserialize`] if the text is not valid project RON.
pub fn from_ron(text: &str) -> Result<ProjectDoc, ProjectError> {
    ron::from_str(text).map_err(|e| ProjectError::Deserialize(e.to_string()))
}

/// Parses, migrates and validates a project from RON text.
///
/// # Errors
///
/// Returns [`ProjectError::Deserialize`] for malformed RON,
/// [`ProjectError::UnsupportedVersion`] if the format cannot be migrated, or
/// [`ProjectError::Invalid`] listing every structural problem found.
pub fn open_ron(text: &str) -> Result<ProjectDoc, ProjectError> {
    let doc = migrate_to_current(from_ron(text)?)?;
    let issues = doc.validate();
    if issues.is_empty() {
        Ok(doc)
    } else {
        Err(ProjectError::Invalid(issues))
    }
}

/// Writes a project to `path` as RON.
///
/// # Errors
///
/// Returns [`ProjectError::Serialize`] or [`ProjectError::Io`] on failure.
pub fn save_to_path(doc: &ProjectDoc, path: &Path) -> Result<(), ProjectError> {
    let text = to_ron(doc)?;
    std::fs::write(path, text).map_err(|e| ProjectError::Io(e.to_string()))
}

/// Reads a project from a RON file without migrating or validating it.
///
/// # Errors
///
/// Returns [`ProjectError::Io`] or [`ProjectError::Deserialize`] on failure.
pub fn load_from_path(path: &Path) -> Result<ProjectDoc, ProjectError> {
    let text = std::fs::read_to_string(path).map_err(|e| ProjectError::Io(e.to_string()))?;
    from_ron(&text)
}

/// Reads, migrates and validates a project from a RON file.
///
/// # Errors
///
/// Returns [`ProjectError::Io`], [`ProjectError::Deserialize`],
/// [`ProjectError::UnsupportedVersion`] or [`ProjectError::Invalid`] on failure.
pub fn open_path(path: &Path) -> Result<ProjectDoc, ProjectError> {
    let text = std::fs::read_to_string(path).map_err(|e| ProjectError::Io(e.to_string()))?;
    open_ron(&text)
}
