//! The serializable project document and its conversions to/from a runtime graph.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vb_core::{Edge, Graph, GraphNode, NodeId};

use crate::error::ProjectError;

/// The current project format version. Bumped when the schema changes.
pub const FORMAT_VERSION: u32 = 1;

/// The oldest format version this build can migrate to [`FORMAT_VERSION`].
pub const MIN_SUPPORTED_VERSION: u32 = 1;

/// One node in a serialized project: its stable id plus its structural definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeEntry {
    /// Stable identifier, unique within the project.
    pub id: NodeId,
    /// The node's kind label and typed ports.
    pub node: GraphNode,
}

/// The serializable form of a patch: nodes and the edges between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            .map(|id| NodeEntry {
                id,
                node: graph.node(id).expect("id from node_ids is valid").clone(),
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
        Ok(graph)
    }
}
