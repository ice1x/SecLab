//! `vb-project` — project serialization for VST_bomb.
//!
//! A [`ProjectDoc`] is the on-disk representation of a patch: the structural graph
//! (nodes with their ports, plus edges) in a versioned, `serde`-friendly form. **RON**
//! is the source-of-truth format. This crate converts to/from the runtime
//! [`vb_core::Graph`] and (de)serializes to RON text and files.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use vb_core::{Edge, Graph, GraphNode, NodeId};

/// The current project format version. Bumped when the schema changes.
pub const FORMAT_VERSION: u32 = 1;

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

/// Errors from serializing, deserializing or rebuilding a project.
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
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(m) => write!(f, "serialize error: {m}"),
            Self::Deserialize(m) => write!(f, "deserialize error: {m}"),
            Self::Io(m) => write!(f, "io error: {m}"),
            Self::UnknownEdgeEndpoint => write!(f, "edge references an unknown node id"),
            Self::Connect(m) => write!(f, "stored edge is invalid: {m}"),
        }
    }
}

impl std::error::Error for ProjectError {}

/// Serializes a project to pretty RON text.
///
/// # Errors
///
/// Returns [`ProjectError::Serialize`] if RON serialization fails.
pub fn to_ron(doc: &ProjectDoc) -> Result<String, ProjectError> {
    ron::ser::to_string_pretty(doc, ron::ser::PrettyConfig::default())
        .map_err(|e| ProjectError::Serialize(e.to_string()))
}

/// Parses a project from RON text.
///
/// # Errors
///
/// Returns [`ProjectError::Deserialize`] if the text is not valid project RON.
pub fn from_ron(text: &str) -> Result<ProjectDoc, ProjectError> {
    ron::from_str(text).map_err(|e| ProjectError::Deserialize(e.to_string()))
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

/// Reads a project from a RON file at `path`.
///
/// # Errors
///
/// Returns [`ProjectError::Io`] or [`ProjectError::Deserialize`] on failure.
pub fn load_from_path(path: &Path) -> Result<ProjectDoc, ProjectError> {
    let text = std::fs::read_to_string(path).map_err(|e| ProjectError::Io(e.to_string()))?;
    from_ron(&text)
}

#[cfg(test)]
mod tests {
    use super::{from_ron, load_from_path, save_to_path, to_ron, ProjectDoc, FORMAT_VERSION};
    use vb_core::{Graph, GraphNode, Port, PortId, SignalType};

    fn sample_graph() -> Graph {
        let mut g = Graph::new();
        let osc = g.add_node(GraphNode::new(
            "oscillator",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let flt = g.add_node(GraphNode::new(
            "biquad",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(osc, PortId(0), flt, PortId(0)).unwrap();
        g
    }

    #[test]
    fn ron_round_trip_preserves_document() {
        let doc = ProjectDoc::from_graph(&sample_graph());
        assert_eq!(doc.version, FORMAT_VERSION);
        let text = to_ron(&doc).unwrap();
        let parsed = from_ron(&text).unwrap();
        assert_eq!(doc, parsed);
    }

    #[test]
    fn graph_to_doc_to_graph_preserves_topology() {
        let original = sample_graph();
        let doc = ProjectDoc::from_graph(&original);
        let rebuilt = doc.to_graph().unwrap();
        assert_eq!(rebuilt.node_count(), original.node_count());
        assert_eq!(rebuilt.edges().len(), original.edges().len());

        // Kinds survive the round trip.
        let doc2 = ProjectDoc::from_graph(&rebuilt);
        let kinds: Vec<&str> = doc2.nodes.iter().map(|n| n.node.kind.as_str()).collect();
        assert!(kinds.contains(&"oscillator") && kinds.contains(&"biquad"));
    }

    #[test]
    fn rebuilt_graph_has_no_cycle() {
        let doc = ProjectDoc::from_graph(&sample_graph());
        assert!(!doc.to_graph().unwrap().has_cycle());
    }

    #[test]
    fn file_save_and_load_round_trip() {
        let doc = ProjectDoc::from_graph(&sample_graph());
        let path = std::env::temp_dir().join("vb_project_file_round_trip.ron");
        save_to_path(&doc, &path).unwrap();
        let loaded = load_from_path(&path).unwrap();
        assert_eq!(doc, loaded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bad_ron_is_an_error() {
        assert!(from_ron("this is not ron").is_err());
    }
}
