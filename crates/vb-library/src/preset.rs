//! Presets and preset banks in the library (backlog 000098).
//!
//! A **preset** is a named set of parameter values for a patch, stored as a
//! [`crate::kind::PRESET`] node and linked to its patch with a
//! [`crate::rel::DERIVED_FROM`] edge (the "preset → patch" link). A **preset bank** is a
//! [`crate::kind::PRESET_BANK`] node that groups presets with
//! [`crate::rel::CONTAINS`] edges. The parameter values live in the preset node's
//! properties; the RON project/preset files remain the source of truth.

use std::collections::HashMap;

use drevo::error::Result;
use drevo::model::{Direction, Edge, Node, Properties};

use crate::library::{kind, new_edge, new_node, rel, Library};

impl Library {
    /// Adds a preset named `name` holding `values` (parameter name → value) and links it
    /// to its patch (`preset --derived-from--> patch`). Returns the preset node.
    ///
    /// # Errors
    /// Propagates any drevo error (including a duplicate title).
    pub fn add_preset_for(
        &self,
        patch_id: u64,
        name: &str,
        values: &[(&str, f32)],
    ) -> Result<Node> {
        let map: HashMap<String, serde_json::Value> = values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), serde_json::json!(v)))
            .collect();
        let mut node = new_node(kind::PRESET, name);
        node.properties = Properties(HashMap::from([(
            "values".to_owned(),
            serde_json::Value::Object(map.into_iter().collect()),
        )]));

        let preset = self.db().create_node(node)?;
        self.link_derived_from(preset.id, patch_id)?;
        Ok(preset)
    }

    /// Creates an empty preset bank named `name`.
    ///
    /// # Errors
    /// Propagates any drevo error (including a duplicate title).
    pub fn create_bank(&self, name: &str) -> Result<Node> {
        self.db().create_node(new_node(kind::PRESET_BANK, name))
    }

    /// Adds a preset to a bank (`bank --contains--> preset`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn add_to_bank(&self, bank_id: u64, preset_id: u64) -> Result<Edge> {
        self.db()
            .create_edge(new_edge(bank_id, preset_id, rel::CONTAINS))
    }

    /// The presets made for a patch (`preset --derived-from--> patch`, incoming), filtered
    /// to preset nodes so a forked patch does not count.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn presets_for_patch(&self, patch_id: u64) -> Result<Vec<Node>> {
        let mut presets =
            self.db()
                .neighbors(patch_id, Direction::Incoming, Some(rel::DERIVED_FROM))?;
        presets.retain(|node| node.kind == kind::PRESET);
        Ok(presets)
    }

    /// The patch a preset belongs to (`preset --derived-from--> patch`), if any.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn patch_of_preset(&self, preset_id: u64) -> Result<Option<Node>> {
        let mut patches =
            self.db()
                .neighbors(preset_id, Direction::Outgoing, Some(rel::DERIVED_FROM))?;
        patches.retain(|node| node.kind == kind::PATCH);
        Ok(patches.into_iter().next())
    }

    /// The presets grouped in a bank (`bank --contains--> preset`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn presets_in_bank(&self, bank_id: u64) -> Result<Vec<Node>> {
        self.db()
            .neighbors(bank_id, Direction::Outgoing, Some(rel::CONTAINS))
    }
}
