//! Errors produced while serializing, loading or checking a project.

use std::fmt;

use crate::validate::Issue;

/// Errors from serializing, deserializing, migrating or rebuilding a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
    /// RON serialization failed.
    Serialize(String),
    /// RON parsing failed.
    Deserialize(String),
    /// Reading or writing a project file failed.
    Io(String),
    /// An edge referenced a node id not present in the document.
    UnknownEdgeEndpoint,
    /// A stored edge failed the graph's connection validation.
    Connect(String),
    /// The document's format version cannot be handled by this build.
    UnsupportedVersion {
        /// The version found in the document.
        found: u32,
        /// The oldest version this build can migrate from.
        min_supported: u32,
        /// The newest version this build understands.
        current: u32,
    },
    /// The document parsed but is not a structurally valid patch.
    Invalid(Vec<Issue>),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(m) => write!(f, "serialize error: {m}"),
            Self::Deserialize(m) => write!(f, "deserialize error: {m}"),
            Self::Io(m) => write!(f, "io error: {m}"),
            Self::UnknownEdgeEndpoint => write!(f, "edge references an unknown node id"),
            Self::Connect(m) => write!(f, "stored edge is invalid: {m}"),
            Self::UnsupportedVersion { found, min_supported, current } => write!(
                f,
                "unsupported project version {found} (this build handles {min_supported}..={current})"
            ),
            Self::Invalid(issues) => {
                write!(f, "invalid project ({} issue(s)):", issues.len())?;
                for issue in issues {
                    write!(f, "\n  - {issue}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ProjectError {}
