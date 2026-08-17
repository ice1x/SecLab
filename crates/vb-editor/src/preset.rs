//! Presets: a named snapshot of a patch's parameter values, saved to RON and reapplied
//! as one undoable step.
//!
//! A preset captures *values only* — not the graph — so it belongs to the patch it was
//! taken from and is matched back by node id. That covers the everyday cases (A/B
//! snapshots, "save this sound", restoring a tweak) without the cross-patch mapping a
//! preset **bank** needs (000079). [`crate::PatchView`] owns the capture and apply; this
//! module is the serializable shape.

use serde::{Deserialize, Serialize};
use vb_core::NodeId;

/// Current preset format version.
pub const PRESET_VERSION: u32 = 1;

/// One captured value: which node, which parameter index, and the value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PresetValue {
    /// The node the parameter is on.
    pub node: NodeId,
    /// The parameter's index in the node's UI order.
    pub index: usize,
    /// The captured value.
    pub value: f32,
}

/// A named set of parameter values for a patch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// Format version (see [`PRESET_VERSION`]).
    pub version: u32,
    /// Human-readable name.
    pub name: String,
    /// Every captured parameter value, in a stable order.
    pub values: Vec<PresetValue>,
}

/// What can go wrong reading a preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    /// RON serialization failed.
    Serialize(String),
    /// RON parsing failed.
    Deserialize(String),
    /// The file's version is newer than this build supports.
    UnsupportedVersion(u32),
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresetError::Serialize(e) => write!(f, "serialize preset: {e}"),
            PresetError::Deserialize(e) => write!(f, "parse preset: {e}"),
            PresetError::UnsupportedVersion(v) => {
                write!(
                    f,
                    "preset version {v} is newer than supported {PRESET_VERSION}"
                )
            }
        }
    }
}

impl std::error::Error for PresetError {}

impl Preset {
    /// Serializes to a pretty RON string.
    ///
    /// # Errors
    /// [`PresetError::Serialize`] if RON serialization fails.
    pub fn to_ron(&self) -> Result<String, PresetError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| PresetError::Serialize(e.to_string()))
    }

    /// Parses a preset from RON, rejecting a future version.
    ///
    /// # Errors
    /// [`PresetError::Deserialize`] on malformed RON, or [`PresetError::UnsupportedVersion`]
    /// if the file is newer than this build.
    pub fn from_ron(text: &str) -> Result<Self, PresetError> {
        let preset: Self =
            ron::from_str(text).map_err(|e| PresetError::Deserialize(e.to_string()))?;
        if preset.version > PRESET_VERSION {
            return Err(PresetError::UnsupportedVersion(preset.version));
        }
        Ok(preset)
    }
}
