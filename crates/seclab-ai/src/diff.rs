//! The difference between two patches, for previewing an assistant edit before it is
//! applied.
//!
//! An edit asks the model for a full, validated [`ProjectDoc`]; comparing it to the
//! current one yields a [`PatchDiff`] — which nodes were added, removed or retyped, and
//! which edges appeared or disappeared. The UI shows this so the human approves a change
//! before it touches the canvas (applying it as an undoable command is 000105).
//!
//! Nodes are matched by id: the assistant is asked to keep ids stable for nodes it does
//! not change, so a small edit reads as a small diff.

use std::collections::{HashMap, HashSet};

use seclab_core::{Edge, NodeId};
use seclab_project::ProjectDoc;

/// A node whose kind changed while keeping its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retype {
    /// The node's id.
    pub id: NodeId,
    /// Its kind before the edit.
    pub from: String,
    /// Its kind after the edit.
    pub to: String,
}

/// The structural difference from one patch to another.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PatchDiff {
    /// Nodes present after but not before, as `(id, kind)`, sorted by id.
    pub added_nodes: Vec<(NodeId, String)>,
    /// Nodes present before but not after, as `(id, kind)`, sorted by id.
    pub removed_nodes: Vec<(NodeId, String)>,
    /// Nodes kept (same id) whose kind changed, sorted by id.
    pub retyped_nodes: Vec<Retype>,
    /// Edges present after but not before.
    pub added_edges: Vec<Edge>,
    /// Edges present before but not after.
    pub removed_edges: Vec<Edge>,
}

impl PatchDiff {
    /// Computes the diff turning `old` into `new`.
    #[must_use]
    pub fn between(old: &ProjectDoc, new: &ProjectDoc) -> Self {
        let old_kinds: HashMap<NodeId, &str> = old
            .nodes
            .iter()
            .map(|e| (e.id, e.node.kind.as_str()))
            .collect();
        let new_kinds: HashMap<NodeId, &str> = new
            .nodes
            .iter()
            .map(|e| (e.id, e.node.kind.as_str()))
            .collect();

        let mut added_nodes = Vec::new();
        let mut retyped_nodes = Vec::new();
        for entry in &new.nodes {
            match old_kinds.get(&entry.id) {
                None => added_nodes.push((entry.id, entry.node.kind.clone())),
                Some(&old_kind) if old_kind != entry.node.kind => retyped_nodes.push(Retype {
                    id: entry.id,
                    from: old_kind.to_owned(),
                    to: entry.node.kind.clone(),
                }),
                Some(_) => {}
            }
        }
        let mut removed_nodes: Vec<(NodeId, String)> = old
            .nodes
            .iter()
            .filter(|e| !new_kinds.contains_key(&e.id))
            .map(|e| (e.id, e.node.kind.clone()))
            .collect();

        added_nodes.sort_by_key(|(id, _)| id.0);
        removed_nodes.sort_by_key(|(id, _)| id.0);
        retyped_nodes.sort_by_key(|r| r.id.0);

        let old_edges: HashSet<Edge> = old.edges.iter().copied().collect();
        let new_edges: HashSet<Edge> = new.edges.iter().copied().collect();
        let added_edges = new
            .edges
            .iter()
            .filter(|e| !old_edges.contains(e))
            .copied()
            .collect();
        let removed_edges = old
            .edges
            .iter()
            .filter(|e| !new_edges.contains(e))
            .copied()
            .collect();

        Self {
            added_nodes,
            removed_nodes,
            retyped_nodes,
            added_edges,
            removed_edges,
        }
    }

    /// Whether the two patches are structurally identical.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added_nodes.is_empty()
            && self.removed_nodes.is_empty()
            && self.retyped_nodes.is_empty()
            && self.added_edges.is_empty()
            && self.removed_edges.is_empty()
    }

    /// A human-readable preview: `+`/`-`/`~` lines for nodes and edges.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "no changes".to_owned();
        }
        let mut lines = Vec::new();
        for (id, kind) in &self.added_nodes {
            lines.push(format!("+ node {} ({kind})", id.0));
        }
        for (id, kind) in &self.removed_nodes {
            lines.push(format!("- node {} ({kind})", id.0));
        }
        for r in &self.retyped_nodes {
            lines.push(format!("~ node {} ({} \u{2192} {})", r.id.0, r.from, r.to));
        }
        for e in &self.added_edges {
            lines.push(format!(
                "+ edge {}.{} -> {}.{}",
                e.from_node.0, e.from_port.0, e.to_node.0, e.to_port.0
            ));
        }
        for e in &self.removed_edges {
            lines.push(format!(
                "- edge {}.{} -> {}.{}",
                e.from_node.0, e.from_port.0, e.to_node.0, e.to_port.0
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seclab_core::{Graph, GraphNode, Port, PortId, SignalType};
    use seclab_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

    fn audio_out(name: &str) -> Port {
        Port::output(name, SignalType::Audio)
    }
    fn audio_in(name: &str) -> Port {
        Port::input(name, SignalType::Audio)
    }

    /// osc(0) -> gain(1).
    fn current() -> ProjectDoc {
        let mut g = Graph::new();
        let osc = g.add_node(GraphNode::new("oscillator", vec![], vec![audio_out("out")]));
        let amp = g.add_node(GraphNode::new(
            "gain",
            vec![audio_in("in")],
            vec![audio_out("out")],
        ));
        g.connect(osc, PortId(0), amp, PortId(0)).unwrap();
        ProjectDoc::from_graph(&g)
    }

    /// A doc with explicit ids so a diff is exact.
    fn doc(nodes: Vec<NodeEntry>, edges: Vec<Edge>) -> ProjectDoc {
        ProjectDoc {
            version: FORMAT_VERSION,
            nodes,
            edges,
        }
    }

    fn node(id: u64, kind: &str, ins: Vec<Port>, outs: Vec<Port>) -> NodeEntry {
        NodeEntry::new(NodeId(id), GraphNode::new(kind, ins, outs))
    }

    fn edge(fnode: u64, fport: usize, tnode: u64, tport: usize) -> Edge {
        Edge {
            from_node: NodeId(fnode),
            from_port: PortId(fport),
            to_node: NodeId(tnode),
            to_port: PortId(tport),
        }
    }

    #[test]
    fn identical_docs_diff_to_nothing() {
        let d = current();
        let diff = PatchDiff::between(&d, &d);
        assert!(diff.is_empty());
        assert_eq!(diff.summary(), "no changes");
    }

    #[test]
    fn inserting_a_filter_shows_the_added_node_and_rewired_edges() {
        let old = current();
        // osc(0) -> biquad(2) -> gain(1): keep osc/gain ids, insert biquad.
        let new = doc(
            vec![
                node(0, "oscillator", vec![], vec![audio_out("out")]),
                node(1, "gain", vec![audio_in("in")], vec![audio_out("out")]),
                node(
                    2,
                    "biquad",
                    vec![audio_in("in"), Port::input("cutoff", SignalType::Control)],
                    vec![audio_out("out")],
                ),
            ],
            vec![edge(0, 0, 2, 0), edge(2, 0, 1, 0)],
        );

        let diff = PatchDiff::between(&old, &new);
        assert_eq!(diff.added_nodes, vec![(NodeId(2), "biquad".to_owned())]);
        assert!(diff.removed_nodes.is_empty());
        assert!(diff.retyped_nodes.is_empty());
        assert_eq!(diff.added_edges, vec![edge(0, 0, 2, 0), edge(2, 0, 1, 0)]);
        assert_eq!(diff.removed_edges, vec![edge(0, 0, 1, 0)]);

        let summary = diff.summary();
        assert!(summary.contains("+ node 2 (biquad)"));
        assert!(summary.contains("- edge 0.0 -> 1.0"));
    }

    #[test]
    fn removing_a_node_is_reported() {
        let old = current();
        let new = doc(
            vec![node(0, "oscillator", vec![], vec![audio_out("out")])],
            vec![],
        );
        let diff = PatchDiff::between(&old, &new);
        assert_eq!(diff.removed_nodes, vec![(NodeId(1), "gain".to_owned())]);
        assert_eq!(diff.removed_edges, vec![edge(0, 0, 1, 0)]);
        assert!(diff.summary().contains("- node 1 (gain)"));
    }

    #[test]
    fn retyping_a_node_keeps_its_id() {
        let old = current();
        // Swap the gain for a waveshaper, same id.
        let new = doc(
            vec![
                node(0, "oscillator", vec![], vec![audio_out("out")]),
                node(
                    1,
                    "waveshaper",
                    vec![audio_in("in")],
                    vec![audio_out("out")],
                ),
            ],
            vec![edge(0, 0, 1, 0)],
        );
        let diff = PatchDiff::between(&old, &new);
        assert_eq!(
            diff.retyped_nodes,
            vec![Retype {
                id: NodeId(1),
                from: "gain".to_owned(),
                to: "waveshaper".to_owned(),
            }]
        );
        assert!(diff.added_nodes.is_empty() && diff.removed_nodes.is_empty());
        assert!(diff.added_edges.is_empty() && diff.removed_edges.is_empty());
        assert!(diff
            .summary()
            .contains("~ node 1 (gain \u{2192} waveshaper)"));
    }
}
