//! The parameter mapping that ties the three views of one control together: a **UI
//! widget** in the editor, a **graph parameter** on a node, and a **host parameter** the
//! DAW automates.
//!
//! All three key off one stable identifier, [`ParamId`] (`n<node>_<index>`), so a knob in
//! the editor, the `(node, param-index)` it edits, and the automatable parameter the
//! plugin exposes are the same thing under three names. This module is the single
//! definition of that scheme — the plugin builds its host parameters from it and the
//! editor binds its widgets by it, instead of each re-deriving the id/name format.
//!
//! It lives in `vb-project` (which both the plugin and the editor depend on) and stays
//! free of the node registry: the caller passes each node kind's parameters in, so the
//! mapping needs only [`ProjectDoc`] and `vb-core`.

use serde::{Deserialize, Serialize};
use vb_core::{NodeId, Parameter};

use crate::doc::ProjectDoc;

/// A stable, host-facing parameter identifier: `n<node-id>_<param-index>`.
///
/// Stable across sessions, which is what lets DAW automation and preset state re-bind to
/// the right control after a reload.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParamId(pub String);

impl ParamId {
    /// The id for parameter `index` on `node`.
    #[must_use]
    pub fn of(node: NodeId, index: usize) -> Self {
        Self(format!("n{}_{index}", node.0))
    }

    /// The `(node, index)` this id addresses, or `None` if it is not a well-formed id.
    #[must_use]
    pub fn target(&self) -> Option<(NodeId, usize)> {
        let rest = self.0.strip_prefix('n')?;
        let (node, index) = rest.split_once('_')?;
        Some((NodeId(node.parse().ok()?), index.parse().ok()?))
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ParamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One control mapped across the UI / graph / host: its stable id, the node and
/// parameter index it edits, a human-readable host name, and the parameter's range.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamMapping {
    /// Stable id shared by the widget, the graph target, and the host parameter.
    pub id: ParamId,
    /// The node the parameter is on.
    pub node: NodeId,
    /// The parameter's index in the node's UI order.
    pub index: usize,
    /// Host-facing display name (`<kind> <param> (node <id>)`).
    pub name: String,
    /// Name, range and default (from the registry).
    pub param: Parameter,
}

/// The host-facing display name for a parameter.
fn display_name(kind: &str, param_name: &str, node: NodeId) -> String {
    format!("{kind} {param_name} (node {})", node.0)
}

/// Builds a [`ParamMapping`] for every parameter of every node in `doc`, in document
/// order. `params_of` yields a node kind's parameters (the editor/plugin pass a registry
/// lookup); a kind that yields none contributes no mappings.
pub fn build_mappings(
    doc: &ProjectDoc,
    params_of: impl Fn(&str) -> Vec<Parameter>,
) -> Vec<ParamMapping> {
    let mut mappings = Vec::new();
    for entry in &doc.nodes {
        for (index, param) in params_of(&entry.node.kind).into_iter().enumerate() {
            mappings.push(ParamMapping {
                id: ParamId::of(entry.id, index),
                node: entry.id,
                index,
                name: display_name(&entry.node.kind, &param.name, entry.id),
                param,
            });
        }
    }
    mappings
}

/// An indexed set of [`ParamMapping`]s with lookup by id or by graph target.
#[derive(Debug, Clone, Default)]
pub struct ParamMap {
    mappings: Vec<ParamMapping>,
}

impl ParamMap {
    /// Builds the map for `doc`, resolving each kind's parameters through `params_of`.
    #[must_use]
    pub fn build(doc: &ProjectDoc, params_of: impl Fn(&str) -> Vec<Parameter>) -> Self {
        Self {
            mappings: build_mappings(doc, params_of),
        }
    }

    /// All mappings, in document order.
    #[must_use]
    pub fn all(&self) -> &[ParamMapping] {
        &self.mappings
    }

    /// Number of mapped parameters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    /// Whether there are no mapped parameters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }

    /// The mapping with the given host id, if any.
    #[must_use]
    pub fn by_id(&self, id: &ParamId) -> Option<&ParamMapping> {
        self.mappings.iter().find(|m| &m.id == id)
    }

    /// The mapping for a graph target `(node, index)`, if any.
    #[must_use]
    pub fn by_target(&self, node: NodeId, index: usize) -> Option<&ParamMapping> {
        self.mappings
            .iter()
            .find(|m| m.node == node && m.index == index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{NodeEntry, ProjectDoc, FORMAT_VERSION};
    use vb_core::{GraphNode, Port, SignalType};

    fn node(id: u64, kind: &str) -> NodeEntry {
        NodeEntry::new(
            NodeId(id),
            GraphNode::new(
                kind,
                vec![Port::input("in", SignalType::Audio)],
                vec![Port::output("out", SignalType::Audio)],
            ),
        )
    }

    /// A doc with an oscillator (id 3) and a gain (id 5).
    fn doc() -> ProjectDoc {
        ProjectDoc {
            version: FORMAT_VERSION,
            nodes: vec![node(3, "oscillator"), node(5, "gain")],
            edges: vec![],
        }
    }

    /// Registry-like parameters per kind.
    fn params_of(kind: &str) -> Vec<Parameter> {
        match kind {
            "oscillator" => vec![
                Parameter::new("waveform", 0.0, 3.0, 0.0),
                Parameter::new("freq_hz", 20.0, 20_000.0, 440.0),
            ],
            "gain" => vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            _ => vec![],
        }
    }

    #[test]
    fn param_id_round_trips_through_its_target() {
        let id = ParamId::of(NodeId(7), 2);
        assert_eq!(id.as_str(), "n7_2");
        assert_eq!(id.target(), Some((NodeId(7), 2)));
        // Malformed ids do not resolve.
        assert_eq!(ParamId("nonsense".to_owned()).target(), None);
        assert_eq!(ParamId("n7".to_owned()).target(), None);
        assert_eq!(ParamId("nx_1".to_owned()).target(), None);
    }

    #[test]
    fn mappings_cover_every_parameter_with_stable_unique_ids() {
        let map = ParamMap::build(&doc(), params_of);
        assert_eq!(map.len(), 3, "osc has 2 params, gain 1");

        let ids: Vec<&str> = map.all().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["n3_0", "n3_1", "n5_0"]);
        // Unique.
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn names_identify_the_node_and_parameter() {
        let map = ParamMap::build(&doc(), params_of);
        let freq = map.by_id(&ParamId::of(NodeId(3), 1)).unwrap();
        assert_eq!(freq.name, "oscillator freq_hz (node 3)");
        assert_eq!(freq.param.max, 20_000.0);
    }

    #[test]
    fn lookup_by_id_and_by_target_agree() {
        let map = ParamMap::build(&doc(), params_of);
        let by_target = map.by_target(NodeId(5), 0).unwrap();
        assert_eq!(by_target.id, ParamId::of(NodeId(5), 0));
        let by_id = map.by_id(&ParamId("n5_0".to_owned())).unwrap();
        assert_eq!(by_id.node, NodeId(5));
        assert_eq!(by_id, by_target);

        // Misses.
        assert!(map.by_target(NodeId(99), 0).is_none());
        assert!(map.by_id(&ParamId("n9_9".to_owned())).is_none());
    }

    #[test]
    fn a_kind_with_no_parameters_contributes_nothing() {
        let map = ParamMap::build(&doc(), |_| Vec::new());
        assert!(map.is_empty());
    }
}
