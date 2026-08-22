//! The serializable project document and its conversions to/from a runtime graph.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vb_core::{Edge, Graph, GraphNode, NodeId};

use crate::error::ProjectError;

/// The current project format version. Bumped when the schema changes.
pub const FORMAT_VERSION: u32 = 1;

/// The oldest format version this build can migrate to [`FORMAT_VERSION`].
pub const MIN_SUPPORTED_VERSION: u32 = 1;

/// One node in a serialized project: its stable id, its structural definition, and
/// its saved parameter values.
///
/// A preset is structure **and** knob settings: `values` holds this node's parameter
/// values in the registry's `params()` order for its kind. It is optional and defaults
/// to empty — an empty list means "use the registry defaults", so older documents and
/// structure-only graphs keep working unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeEntry {
    /// Stable identifier, unique within the project.
    pub id: NodeId,
    /// The node's kind label and typed ports.
    pub node: GraphNode,
    /// Saved parameter values in registry `params()` order; empty means defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<f32>,
}

impl NodeEntry {
    /// A structural entry with no saved parameter values (registry defaults apply).
    #[must_use]
    pub fn new(id: NodeId, node: GraphNode) -> Self {
        Self {
            id,
            node,
            values: Vec::new(),
        }
    }

    /// An entry carrying saved parameter values, in registry `params()` order.
    #[must_use]
    pub fn with_values(id: NodeId, node: GraphNode, values: Vec<f32>) -> Self {
        Self { id, node, values }
    }
}

/// The serializable form of a patch: nodes and the edges between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectDoc {
    /// Format version (see [`FORMAT_VERSION`]).
    pub version: u32,
    /// Nodes, in a stable order.
    pub nodes: Vec<NodeEntry>,
    /// Edges wiring node ports together.
    pub edges: Vec<Edge>,
}

impl ProjectDoc {
    /// Captures a [`Graph`] as a serializable document, nodes ordered by id.
    #[must_use]
    pub fn from_graph(graph: &Graph) -> Self {
        let mut ids: Vec<NodeId> = graph.node_ids().collect();
        ids.sort_unstable_by_key(|id| id.0);
        let nodes = ids
            .into_iter()
            .map(|id| {
                NodeEntry::new(
                    id,
                    graph.node(id).expect("id from node_ids is valid").clone(),
                )
            })
            .collect();
        Self {
            version: FORMAT_VERSION,
            nodes,
            edges: graph.edges().to_vec(),
        }
    }

    /// Looks up a node entry by its stored id.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&GraphNode> {
        self.nodes.iter().find(|e| e.id == id).map(|e| &e.node)
    }

    /// This node's saved parameter values (registry `params()` order), or an empty
    /// slice if the id is unknown or the node carries no saved values.
    #[must_use]
    pub fn node_values(&self, id: NodeId) -> &[f32] {
        self.nodes
            .iter()
            .find(|e| e.id == id)
            .map_or(&[], |e| e.values.as_slice())
    }

    /// Records saved parameter values for a node, replacing any existing ones.
    /// Returns `false` if the id is not present.
    pub fn set_node_values(&mut self, id: NodeId, values: Vec<f32>) -> bool {
        match self.nodes.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.values = values;
                true
            }
            None => false,
        }
    }

    /// Rebuilds a runtime [`Graph`] from this document.
    ///
    /// Node ids are reallocated by the graph; edges are remapped accordingly, so the
    /// topology is preserved even though the concrete ids may differ.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectError::UnknownEdgeEndpoint`] if an edge references a node id that
    /// is not present, or [`ProjectError::Connect`] if a stored edge fails validation.
    pub fn to_graph(&self) -> Result<Graph, ProjectError> {
        self.to_graph_with_remap().map(|(graph, _)| graph)
    }

    /// Like [`to_graph`](Self::to_graph), but also returns the map from each stored
    /// (document) node id to the freshly allocated runtime id.
    ///
    /// Callers that need to attach per-node data — parameter values, sink selection —
    /// use the remap to address runtime nodes by their document id instead of assuming
    /// the ids coincide.
    ///
    /// # Errors
    ///
    /// Same as [`to_graph`](Self::to_graph).
    pub fn to_graph_with_remap(&self) -> Result<(Graph, HashMap<NodeId, NodeId>), ProjectError> {
        let mut graph = Graph::new();
        let mut remap: HashMap<NodeId, NodeId> = HashMap::new();
        for entry in &self.nodes {
            let new_id = graph.add_node(entry.node.clone());
            remap.insert(entry.id, new_id);
        }
        for edge in &self.edges {
            let from = *remap
                .get(&edge.from_node)
                .ok_or(ProjectError::UnknownEdgeEndpoint)?;
            let to = *remap
                .get(&edge.to_node)
                .ok_or(ProjectError::UnknownEdgeEndpoint)?;
            graph
                .connect(from, edge.from_port, to, edge.to_port)
                .map_err(|e| ProjectError::Connect(e.to_string()))?;
        }
        Ok((graph, remap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_core::{Port, SignalType};

    fn one_node_graph() -> Graph {
        let mut g = Graph::new();
        g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g
    }

    #[test]
    fn from_graph_leaves_values_empty() {
        let doc = ProjectDoc::from_graph(&one_node_graph());
        assert!(doc.nodes.iter().all(|e| e.values.is_empty()));
    }

    #[test]
    fn set_and_read_node_values() {
        let mut doc = ProjectDoc::from_graph(&one_node_graph());
        let id = doc.nodes[0].id;
        assert!(doc.set_node_values(id, vec![0.25]));
        assert_eq!(doc.node_values(id), &[0.25]);
        assert!(!doc.set_node_values(NodeId(9999), vec![1.0]));
        assert_eq!(doc.node_values(NodeId(9999)), &[] as &[f32]);
    }

    #[test]
    fn values_survive_ron_round_trip() {
        let mut doc = ProjectDoc::from_graph(&one_node_graph());
        let id = doc.nodes[0].id;
        doc.set_node_values(id, vec![0.5]);
        let ron = crate::io::to_ron(&doc).expect("serializes");
        let back = crate::io::from_ron(&ron).expect("parses");
        assert_eq!(back.node_values(back.nodes[0].id), &[0.5]);
        assert_eq!(doc, back);
    }

    #[test]
    fn empty_values_are_omitted_from_ron() {
        let doc = ProjectDoc::from_graph(&one_node_graph());
        let ron = crate::io::to_ron(&doc).expect("serializes");
        assert!(
            !ron.contains("values"),
            "structure-only docs must not emit a values field: {ron}"
        );
    }

    #[test]
    fn documents_without_values_still_parse() {
        // A structure-only document written before values existed.
        let legacy = r#"(version: 1, nodes: [(id: (0), node: (kind: "gain", inputs: [], outputs: []))], edges: [])"#;
        let doc = crate::io::from_ron(legacy).expect("legacy doc parses");
        assert!(doc.nodes[0].values.is_empty());
    }

    #[test]
    fn remap_maps_document_ids_to_runtime_ids() {
        let mut doc = ProjectDoc::from_graph(&one_node_graph());
        // Force a non-trivial stored id so identity mapping would be wrong.
        doc.nodes[0].id = NodeId(77);
        let (graph, remap) = doc.to_graph_with_remap().expect("builds");
        let runtime = remap[&NodeId(77)];
        assert!(graph.node(runtime).is_some());
    }
}
