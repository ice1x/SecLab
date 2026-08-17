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

/// An ordered collection of [`Preset`]s with a current selection — the unit a host's
/// program-change or a UI dropdown steps through.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PresetBank {
    /// Format version (see [`PRESET_VERSION`]).
    pub version: u32,
    /// The presets, in bank order.
    pub presets: Vec<Preset>,
    /// Index of the current preset, if any.
    selected: Option<usize>,
}

impl PresetBank {
    /// Creates an empty bank.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: PRESET_VERSION,
            presets: Vec::new(),
            selected: None,
        }
    }

    /// Appends `preset`, selects it, and returns its index.
    pub fn add(&mut self, preset: Preset) -> usize {
        self.presets.push(preset);
        let index = self.presets.len() - 1;
        self.selected = Some(index);
        index
    }

    /// Removes the preset at `index`, keeping the selection pointing at a sensible
    /// neighbour (or `None` when the bank empties). Returns the removed preset.
    pub fn remove(&mut self, index: usize) -> Option<Preset> {
        if index >= self.presets.len() {
            return None;
        }
        let removed = self.presets.remove(index);
        self.selected = match self.selected {
            _ if self.presets.is_empty() => None,
            Some(sel) if sel > index => Some(sel - 1),
            Some(sel) if sel == index => Some(index.min(self.presets.len() - 1)),
            other => other,
        };
        Some(removed)
    }

    /// Number of presets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.presets.len()
    }

    /// Whether the bank has no presets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }

    /// The preset names, in bank order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.presets.iter().map(|p| p.name.as_str()).collect()
    }

    /// The current selection index, if any.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The currently selected preset, if any.
    #[must_use]
    pub fn current(&self) -> Option<&Preset> {
        self.selected.and_then(|i| self.presets.get(i))
    }

    /// Selects `index` and returns the now-current preset. An out-of-range index leaves
    /// the selection unchanged and returns `None`.
    pub fn select(&mut self, index: usize) -> Option<&Preset> {
        if index >= self.presets.len() {
            return None;
        }
        self.selected = Some(index);
        self.current()
    }

    /// Advances to the next preset (wrapping), returning it — how a host program-change
    /// steps forward. `None` only when the bank is empty.
    pub fn next_preset(&mut self) -> Option<&Preset> {
        self.step(1)
    }

    /// Steps to the previous preset (wrapping), returning it.
    pub fn prev_preset(&mut self) -> Option<&Preset> {
        self.step(-1)
    }

    /// Moves the selection by `delta` with wraparound.
    fn step(&mut self, delta: isize) -> Option<&Preset> {
        let n = self.presets.len();
        if n == 0 {
            return None;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let next = current.rem_euclid(n as isize);
        let next = (next + delta).rem_euclid(n as isize) as usize;
        self.selected = Some(next);
        self.current()
    }

    /// Serializes to a pretty RON string.
    ///
    /// # Errors
    /// [`PresetError::Serialize`] if RON serialization fails.
    pub fn to_ron(&self) -> Result<String, PresetError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| PresetError::Serialize(e.to_string()))
    }

    /// Parses a bank from RON, rejecting a future version.
    ///
    /// # Errors
    /// [`PresetError::Deserialize`] on malformed RON, or [`PresetError::UnsupportedVersion`].
    pub fn from_ron(text: &str) -> Result<Self, PresetError> {
        let bank: Self =
            ron::from_str(text).map_err(|e| PresetError::Deserialize(e.to_string()))?;
        if bank.version > PRESET_VERSION {
            return Err(PresetError::UnsupportedVersion(bank.version));
        }
        Ok(bank)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(name: &str) -> Preset {
        Preset {
            version: PRESET_VERSION,
            name: name.to_owned(),
            values: Vec::new(),
        }
    }

    #[test]
    fn add_appends_selects_and_reports_names() {
        let mut bank = PresetBank::new();
        assert!(bank.is_empty());
        assert_eq!(bank.add(preset("init")), 0);
        assert_eq!(bank.add(preset("lead")), 1);
        assert_eq!(bank.len(), 2);
        assert_eq!(bank.names(), vec!["init", "lead"]);
        assert_eq!(bank.selected(), Some(1), "the newest is selected");
        assert_eq!(bank.current().unwrap().name, "lead");
    }

    #[test]
    fn select_validates_the_index() {
        let mut bank = PresetBank::new();
        bank.add(preset("a"));
        bank.add(preset("b"));
        assert_eq!(bank.select(0).unwrap().name, "a");
        assert_eq!(bank.select(9), None, "out of range");
        assert_eq!(
            bank.selected(),
            Some(0),
            "selection unchanged on a bad index"
        );
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut bank = PresetBank::new();
        bank.add(preset("a"));
        bank.add(preset("b"));
        bank.add(preset("c"));
        bank.select(2); // "c"
        assert_eq!(bank.next_preset().unwrap().name, "a", "wraps forward");
        assert_eq!(bank.prev_preset().unwrap().name, "c", "wraps back");
        assert_eq!(bank.next_preset().unwrap().name, "a");
        assert_eq!(bank.next_preset().unwrap().name, "b");

        // An empty bank has nothing to step to.
        let mut empty = PresetBank::new();
        assert_eq!(empty.next_preset(), None);
    }

    #[test]
    fn remove_keeps_the_selection_sensible() {
        let mut bank = PresetBank::new();
        for n in ["a", "b", "c", "d"] {
            bank.add(preset(n));
        }
        bank.select(2); // "c"

        // Removing before the selection shifts it down to stay on "c".
        bank.remove(0);
        assert_eq!(bank.current().unwrap().name, "c");
        assert_eq!(bank.selected(), Some(1));

        // Removing the selected clamps onto the neighbour.
        bank.remove(1); // drops "c"; remaining ["b","d"]
        assert_eq!(bank.selected(), Some(1));
        assert_eq!(bank.current().unwrap().name, "d");

        // Emptying clears the selection.
        bank.remove(0);
        bank.remove(0);
        assert!(bank.is_empty());
        assert_eq!(bank.selected(), None);
        assert_eq!(bank.remove(0), None, "nothing to remove");
    }

    #[test]
    fn bank_round_trips_through_ron() {
        let mut bank = PresetBank::new();
        bank.add(preset("a"));
        bank.add(preset("b"));
        let ron = bank.to_ron().unwrap();
        assert_eq!(PresetBank::from_ron(&ron).unwrap(), bank);

        let mut future = bank.clone();
        future.version = PRESET_VERSION + 1;
        assert_eq!(
            PresetBank::from_ron(&future.to_ron().unwrap()),
            Err(PresetError::UnsupportedVersion(PRESET_VERSION + 1))
        );
    }
}
