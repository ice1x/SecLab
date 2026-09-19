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
    /// **Panel-surface** position of this node's control block, `(x, y)`. `None` means the
    /// panel auto-flows the module into a grid slot. Purely presentational — it never affects
    /// the audio graph — and optional, so structure-only graphs and older documents load with
    /// no placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_pos: Option<(f32, f32)>,
    /// Per-parameter **control-widget kind** on the panel, in registry `params()` order, each as
    /// the kind's short label (`"knob"` / `"slider"` / `"switch"` / `"xy-pad"`). Empty means every
    /// parameter uses its default control. The format stores the label (a plain string) rather
    /// than the editor's `WidgetKind` enum so `vb-project` stays free of a `vb-editor` dependency.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widgets: Vec<String>,
    /// Optional **panel accent colour** for this node's control block, as a `#rrggbb` hex string.
    /// `None` means the module uses the theme's default framing. Presentational and optional, so
    /// an unstyled patch stores nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_color: Option<String>,
    /// Indices (registry `params()` order) of parameters **hidden from the panel** — the ones the
    /// user unchecked in the inspector's "show on panel". Stored sparsely as a hidden-set rather
    /// than a per-parameter flag so the common case (every control visible) stores nothing and old
    /// documents load with all controls visible. Purely presentational — it never affects audio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panel_hidden: Vec<usize>,
}

impl NodeEntry {
    /// A structural entry with no saved parameter values (registry defaults apply).
    #[must_use]
    pub fn new(id: NodeId, node: GraphNode) -> Self {
        Self {
            id,
            node,
            values: Vec::new(),
            panel_pos: None,
            widgets: Vec::new(),
            panel_color: None,
            panel_hidden: Vec::new(),
        }
    }

    /// An entry carrying saved parameter values, in registry `params()` order.
    #[must_use]
    pub fn with_values(id: NodeId, node: GraphNode, values: Vec<f32>) -> Self {
        Self {
            id,
            node,
            values,
            panel_pos: None,
            widgets: Vec::new(),
            panel_color: None,
            panel_hidden: Vec::new(),
        }
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

    /// This node's saved panel position, or `None` if the id is unknown or the module is
    /// unplaced (auto-flowed).
    #[must_use]
    pub fn node_panel_pos(&self, id: NodeId) -> Option<(f32, f32)> {
        self.nodes
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.panel_pos)
    }

    /// Records a node's panel position (`None` clears it back to auto-flow). Returns `false`
    /// if the id is not present.
    pub fn set_node_panel_pos(&mut self, id: NodeId, pos: Option<(f32, f32)>) -> bool {
        match self.nodes.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.panel_pos = pos;
                true
            }
            None => false,
        }
    }

    /// This node's saved per-parameter widget-kind labels (registry `params()` order), or an
    /// empty slice if the id is unknown or the node uses default controls.
    #[must_use]
    pub fn node_widgets(&self, id: NodeId) -> &[String] {
        self.nodes
            .iter()
            .find(|e| e.id == id)
            .map_or(&[], |e| e.widgets.as_slice())
    }

    /// Records a node's per-parameter widget-kind labels, replacing any existing ones. Returns
    /// `false` if the id is not present.
    pub fn set_node_widgets(&mut self, id: NodeId, widgets: Vec<String>) -> bool {
        match self.nodes.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.widgets = widgets;
                true
            }
            None => false,
        }
    }

    /// This node's saved panel accent colour (`#rrggbb`), or `None` if the id is unknown or the
    /// module uses the default framing.
    #[must_use]
    pub fn node_panel_color(&self, id: NodeId) -> Option<&str> {
        self.nodes
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.panel_color.as_deref())
    }

    /// Records a node's panel accent colour (`None` clears it back to the default). Returns
    /// `false` if the id is not present.
    pub fn set_node_panel_color(&mut self, id: NodeId, color: Option<String>) -> bool {
        match self.nodes.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.panel_color = color;
                true
            }
            None => false,
        }
    }

    /// The indices of this node's parameters hidden from the panel (registry `params()` order),
    /// or an empty slice if the id is unknown or every control is shown.
    #[must_use]
    pub fn node_panel_hidden(&self, id: NodeId) -> &[usize] {
        self.nodes
            .iter()
            .find(|e| e.id == id)
            .map_or(&[], |e| e.panel_hidden.as_slice())
    }

    /// Records which of a node's parameters are hidden from the panel, replacing any existing set.
    /// Returns `false` if the id is not present.
    pub fn set_node_panel_hidden(&mut self, id: NodeId, hidden: Vec<usize>) -> bool {
        match self.nodes.iter_mut().find(|e| e.id == id) {
            Some(entry) => {
                entry.panel_hidden = hidden;
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
    fn panel_layout_survives_ron_round_trip() {
        let mut doc = ProjectDoc::from_graph(&one_node_graph());
        let id = doc.nodes[0].id;
        assert!(doc.set_node_panel_pos(id, Some((120.5, 40.0))));
        assert!(doc.set_node_widgets(id, vec!["slider".to_owned()]));
        assert!(doc.set_node_panel_color(id, Some("#ff8800".to_owned())));
        assert!(doc.set_node_panel_hidden(id, vec![1, 3]));
        let ron = crate::io::to_ron(&doc).expect("serializes");
        let back = crate::io::from_ron(&ron).expect("parses");
        assert_eq!(back.node_panel_pos(id), Some((120.5, 40.0)));
        assert_eq!(back.node_widgets(id), &["slider".to_owned()]);
        assert_eq!(back.node_panel_color(id), Some("#ff8800"));
        assert_eq!(back.node_panel_hidden(id), &[1, 3]);
        assert_eq!(doc, back);
        // Unknown ids read as absent, and setting on a missing id fails.
        assert_eq!(doc.node_panel_pos(NodeId(9999)), None);
        assert_eq!(doc.node_panel_color(NodeId(9999)), None);
        assert_eq!(doc.node_panel_hidden(NodeId(9999)), &[] as &[usize]);
        assert!(!doc.set_node_widgets(NodeId(9999), vec!["knob".to_owned()]));
        assert!(!doc.set_node_panel_hidden(NodeId(9999), vec![0]));
    }

    #[test]
    fn an_unplaced_default_panel_is_omitted_from_ron() {
        // A patch nobody has arranged must not emit panel fields — no format bloat, and old
        // readers see exactly the document they saw before the panel layout existed.
        let doc = ProjectDoc::from_graph(&one_node_graph());
        let ron = crate::io::to_ron(&doc).expect("serializes");
        assert!(!ron.contains("panel_pos"), "no placement to store: {ron}");
        assert!(
            !ron.contains("widgets"),
            "no widget overrides to store: {ron}"
        );
        assert!(!ron.contains("panel_color"), "no accent to store: {ron}");
        assert!(
            !ron.contains("panel_hidden"),
            "every control visible stores nothing: {ron}"
        );
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
