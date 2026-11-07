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

/// The one RON pretty-printing configuration every on-disk SecLab document uses.
///
/// It pins the line terminator to `"\n"`: `PrettyConfig::default()` uses the platform's
/// native newline, so the same patch saved on Windows came out with CRLF — a different
/// file byte-for-byte, which broke the committed-example byte-equality guards on the first
/// Windows CI run and would make a patch's text differ by the OS it was saved on. The
/// patch format is one format on every platform.
pub(crate) fn ron_pretty() -> ron::ser::PrettyConfig {
    ron::ser::PrettyConfig::default().new_line("\n")
}

/// Serializes a project to pretty RON text (LF line endings on every platform).
///
/// # Errors
///
/// Returns [`ProjectError::Serialize`] if RON serialization fails.
pub fn to_ron(doc: &ProjectDoc) -> Result<String, ProjectError> {
    ron::ser::to_string_pretty(doc, ron_pretty())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_project_uses_lf_line_endings_on_every_platform() {
        // ron's `PrettyConfig::default()` picks the platform newline (CRLF on Windows). The
        // patch format must be byte-identical whichever OS saved it — the committed
        // `examples/*.ron` are compared byte-for-byte against their builders in CI.
        let doc = ProjectDoc::from_graph(&seclab_core::Graph::default());
        let text = to_ron(&doc).expect("serialises");
        assert!(text.contains('\n'), "pretty RON spans lines: {text:?}");
        assert!(!text.contains('\r'), "CRLF leaked into RON: {text:?}");
        // And it still round-trips.
        assert_eq!(from_ron(&text).expect("parses"), doc);
    }
}
