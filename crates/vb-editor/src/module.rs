//! Reusable macro modules: a macro's inner patch captured as a portable, versioned
//! RON document, so a subgraph built once can be saved and dropped into other patches.
//!
//! A module stores everything the editor needs to rebuild the subpatch faithfully —
//! each node's kind and ports, its canvas position, and its parameter values *and*
//! chosen control widgets — plus the edges between them. [`crate::PatchView`] owns the
//! conversions (it alone can read its private positions/params); this module is the
//! serializable shape and its file I/O.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vb_core::{Edge, GraphNode, NodeId, Parameter};

use crate::widgets::WidgetKind;

/// Current module format version. Bumped when the schema changes.
pub const MODULE_VERSION: u32 = 1;

/// File extension modules are written with (`<name>.vbmod.ron`).
pub const MODULE_EXTENSION: &str = "vbmod.ron";

/// One parameter of a module node: its descriptor, current value, and control widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleParam {
    /// Name, range and default.
    pub param: Parameter,
    /// Stored value (inside the descriptor's range).
    pub value: f32,
    /// Which inspector control this parameter uses.
    pub widget: WidgetKind,
}

/// One node of a module: its structural definition, canvas position, and parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleNode {
    /// The node's id inside the module (used to rewire edges on import).
    pub id: NodeId,
    /// Kind label and typed ports.
    pub node: GraphNode,
    /// Canvas position `(x, y)`.
    pub pos: (f32, f32),
    /// Editable parameters with their values and widgets.
    pub params: Vec<ModuleParam>,
}

/// A macro's inner patch as a portable, versioned document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroModule {
    /// Format version (see [`MODULE_VERSION`]).
    pub version: u32,
    /// Human-readable module name (also the default file stem).
    pub name: String,
    /// Nodes, in draw order.
    pub nodes: Vec<ModuleNode>,
    /// Edges wiring the nodes' ports.
    pub edges: Vec<Edge>,
}

/// What can go wrong exporting or importing a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleError {
    /// RON serialization failed.
    Serialize(String),
    /// RON parsing failed.
    Deserialize(String),
    /// The file's version is newer than this build supports.
    UnsupportedVersion(u32),
    /// Rebuilding the patch failed (e.g. a dangling edge endpoint or a rejected wire).
    Build(String),
    /// A filesystem operation failed.
    Io(String),
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleError::Serialize(e) => write!(f, "serialize module: {e}"),
            ModuleError::Deserialize(e) => write!(f, "parse module: {e}"),
            ModuleError::UnsupportedVersion(v) => {
                write!(
                    f,
                    "module version {v} is newer than supported {MODULE_VERSION}"
                )
            }
            ModuleError::Build(e) => write!(f, "rebuild module patch: {e}"),
            ModuleError::Io(e) => write!(f, "module file: {e}"),
        }
    }
}

impl std::error::Error for ModuleError {}

impl MacroModule {
    /// Serializes to a pretty RON string.
    ///
    /// # Errors
    /// [`ModuleError::Serialize`] if RON serialization fails.
    pub fn to_ron(&self) -> Result<String, ModuleError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| ModuleError::Serialize(e.to_string()))
    }

    /// Parses a module from RON, rejecting a future format version.
    ///
    /// # Errors
    /// [`ModuleError::Deserialize`] on malformed RON, or
    /// [`ModuleError::UnsupportedVersion`] if the file is newer than this build.
    pub fn from_ron(s: &str) -> Result<Self, ModuleError> {
        let module: Self = ron::from_str(s).map_err(|e| ModuleError::Deserialize(e.to_string()))?;
        if module.version > MODULE_VERSION {
            return Err(ModuleError::UnsupportedVersion(module.version));
        }
        Ok(module)
    }

    /// Writes the module to `dir/<name>.vbmod.ron`, creating `dir` if needed, and
    /// returns the path written.
    ///
    /// # Errors
    /// [`ModuleError::Serialize`] or [`ModuleError::Io`].
    pub fn write_to_dir(&self, dir: &Path) -> Result<PathBuf, ModuleError> {
        std::fs::create_dir_all(dir).map_err(|e| ModuleError::Io(e.to_string()))?;
        let path = dir.join(format!("{}.{MODULE_EXTENSION}", sanitize(&self.name)));
        std::fs::write(&path, self.to_ron()?).map_err(|e| ModuleError::Io(e.to_string()))?;
        Ok(path)
    }

    /// Reads and parses a module file.
    ///
    /// # Errors
    /// [`ModuleError::Io`], [`ModuleError::Deserialize`], or
    /// [`ModuleError::UnsupportedVersion`].
    pub fn read_from_file(path: &Path) -> Result<Self, ModuleError> {
        let text = std::fs::read_to_string(path).map_err(|e| ModuleError::Io(e.to_string()))?;
        Self::from_ron(&text)
    }
}

/// Filenames of module files in `dir` (files ending in `.vbmod.ron`), sorted.
/// A missing or unreadable directory yields an empty list.
#[must_use]
pub fn list_modules(dir: &Path) -> Vec<PathBuf> {
    let suffix = format!(".{MODULE_EXTENSION}");
    let mut out: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(&suffix))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

/// Makes `name` safe as a file stem: keeps alphanumerics, `-` and `_`; the rest become
/// `-`. Empty input becomes `"module"`.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "module".to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_core::{Port, PortId, SignalType};

    fn sample_module() -> MacroModule {
        let gain = GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        );
        MacroModule {
            version: MODULE_VERSION,
            name: "my synth voice".to_owned(),
            nodes: vec![ModuleNode {
                id: NodeId(0),
                node: gain,
                pos: (12.0, -4.0),
                params: vec![ModuleParam {
                    param: Parameter::new("gain", 0.0, 2.0, 1.0),
                    value: 0.7,
                    widget: WidgetKind::Knob,
                }],
            }],
            edges: vec![Edge {
                from_node: NodeId(0),
                from_port: PortId(0),
                to_node: NodeId(0),
                to_port: PortId(0),
            }],
        }
    }

    #[test]
    fn ron_round_trip_preserves_everything() {
        let module = sample_module();
        let ron = module.to_ron().expect("serialize");
        let back = MacroModule::from_ron(&ron).expect("parse");
        assert_eq!(back, module, "positions, params, widgets and edges survive");
    }

    #[test]
    fn a_future_version_is_rejected() {
        let mut module = sample_module();
        module.version = MODULE_VERSION + 1;
        let ron = module.to_ron().unwrap();
        assert_eq!(
            MacroModule::from_ron(&ron),
            Err(ModuleError::UnsupportedVersion(MODULE_VERSION + 1))
        );
    }

    #[test]
    fn malformed_ron_is_a_clean_error() {
        assert!(matches!(
            MacroModule::from_ron("this is not ron"),
            Err(ModuleError::Deserialize(_))
        ));
    }

    #[test]
    fn write_read_and_list_round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("vbmod-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let module = sample_module();

        let path = module.write_to_dir(&dir).expect("write");
        // The unsafe characters in the name are sanitized in the file stem.
        assert!(path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("my-synth-voice"));

        let listed = list_modules(&dir);
        assert_eq!(listed, vec![path.clone()], "the module is discoverable");

        let read = MacroModule::read_from_file(&path).expect("read");
        assert_eq!(read, module);

        // A directory with no modules lists nothing.
        let empty = std::env::temp_dir().join("vbmod-test-does-not-exist-xyz");
        assert!(list_modules(&empty).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_keeps_safe_chars_and_falls_back() {
        assert_eq!(sanitize("Lead/Bass 01"), "Lead-Bass-01");
        assert_eq!(sanitize("ok_name-1"), "ok_name-1");
        assert_eq!(sanitize(""), "module");
        assert_eq!(sanitize("***"), "---");
    }
}
