//! Deterministic assistance that needs no model call: what node could come next, and
//! one-click repair of incompatible or dangling connections.
//!
//! These are grounded in the registry and the graph's own rules, so they are exact and
//! instant — a good complement to the LLM assistant (which can also call them). Suggesting
//! a next node is "given this free output, which node kinds accept its signal"; auto-fix
//! removes exactly the edges the validator would reject (unknown endpoint, out-of-range
//! port, signal mismatch, a doubly-driven input), leaving the wiring sound.

use seclab_core::{Edge, NodeId, PortId, SignalType};
use seclab_nodes::Registry;
use seclab_project::ProjectDoc;

/// Node kinds whose inputs can accept a `signal` — i.e. what a source of that signal can
/// feed into. Sorted by kind name.
#[must_use]
pub fn compatible_consumers(registry: &Registry, signal: SignalType) -> Vec<&'static str> {
    registry
        .kinds()
        .filter(|kind| {
            registry
                .graph_node(kind)
                .is_some_and(|node| node.inputs.iter().any(|p| signal.compatible_with(p.signal)))
        })
        .collect()
}

/// An output port with no outgoing edge — a place to extend the patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeOutput {
    /// The node the port is on.
    pub node: NodeId,
    /// The output port index.
    pub port: PortId,
    /// The signal it carries.
    pub signal: SignalType,
}

/// Every output port in `doc` that nothing consumes yet.
#[must_use]
pub fn free_outputs(doc: &ProjectDoc) -> Vec<FreeOutput> {
    let mut free = Vec::new();
    for entry in &doc.nodes {
        for (i, port) in entry.node.outputs.iter().enumerate() {
            let taken = doc
                .edges
                .iter()
                .any(|e| e.from_node == entry.id && e.from_port.0 == i);
            if !taken {
                free.push(FreeOutput {
                    node: entry.id,
                    port: PortId(i),
                    signal: port.signal,
                });
            }
        }
    }
    free
}

/// A suggestion to extend the patch from a free output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextNode {
    /// The free output to attach to.
    pub from: FreeOutput,
    /// Node kinds whose input accepts that output's signal.
    pub kinds: Vec<&'static str>,
}

/// For each free output in `doc`, the node kinds that could consume it.
#[must_use]
pub fn suggest_next(doc: &ProjectDoc, registry: &Registry) -> Vec<NextNode> {
    free_outputs(doc)
        .into_iter()
        .map(|from| NextNode {
            kinds: compatible_consumers(registry, from.signal),
            from,
        })
        .collect()
}

/// Why an edge was removed by [`autofix`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDefect {
    /// An endpoint node id is not in the document.
    UnknownEndpoint,
    /// A port index is out of range on its node.
    PortOutOfRange,
    /// The two ports carry incompatible signal types.
    SignalMismatch,
    /// The destination input already has a source.
    InputOccupied,
}

impl std::fmt::Display for EdgeDefect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            EdgeDefect::UnknownEndpoint => "endpoint node missing",
            EdgeDefect::PortOutOfRange => "port index out of range",
            EdgeDefect::SignalMismatch => "incompatible signal types",
            EdgeDefect::InputOccupied => "input already connected",
        };
        f.write_str(s)
    }
}

/// One repair: the offending edge and why it was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fix {
    /// Index of the removed edge in the original document.
    pub edge_index: usize,
    /// The reason it was removed.
    pub defect: EdgeDefect,
}

/// The defect of `edge` given the inputs already claimed by kept edges, or `None` if it
/// is sound.
fn edge_defect(
    doc: &ProjectDoc,
    edge: &Edge,
    occupied: &std::collections::HashSet<(NodeId, PortId)>,
) -> Option<EdgeDefect> {
    let (Some(src), Some(dst)) = (doc.node(edge.from_node), doc.node(edge.to_node)) else {
        return Some(EdgeDefect::UnknownEndpoint);
    };
    let (Some(out), Some(inp)) = (
        src.outputs.get(edge.from_port.0),
        dst.inputs.get(edge.to_port.0),
    ) else {
        return Some(EdgeDefect::PortOutOfRange);
    };
    if !out.signal.compatible_with(inp.signal) {
        return Some(EdgeDefect::SignalMismatch);
    }
    if occupied.contains(&(edge.to_node, edge.to_port)) {
        return Some(EdgeDefect::InputOccupied);
    }
    None
}

/// Removes every incompatible or dangling edge from `doc`, returning the repaired
/// document and the list of fixes applied.
///
/// The first edge into any input wins; later ones are dropped as [`EdgeDefect::InputOccupied`].
/// Node topology (including cycles) is left untouched — this only prunes bad wires, so the
/// result is free of edge defects (a separate concern, cycles, still surface in
/// [`ProjectDoc::validate`]).
#[must_use]
pub fn autofix(doc: &ProjectDoc) -> (ProjectDoc, Vec<Fix>) {
    let mut kept = Vec::new();
    let mut fixes = Vec::new();
    let mut occupied: std::collections::HashSet<(NodeId, PortId)> =
        std::collections::HashSet::new();

    for (index, edge) in doc.edges.iter().enumerate() {
        match edge_defect(doc, edge, &occupied) {
            Some(defect) => fixes.push(Fix {
                edge_index: index,
                defect,
            }),
            None => {
                occupied.insert((edge.to_node, edge.to_port));
                kept.push(*edge);
            }
        }
    }

    let fixed = ProjectDoc {
        version: doc.version,
        nodes: doc.nodes.clone(),
        edges: kept,
    };
    (fixed, fixes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use seclab_core::{Graph, GraphNode, Port, PortId, SignalType};
    use seclab_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

    fn ao(name: &str) -> Port {
        Port::output(name, SignalType::Audio)
    }
    fn ai(name: &str) -> Port {
        Port::input(name, SignalType::Audio)
    }

    fn osc_gain() -> ProjectDoc {
        let mut g = Graph::new();
        let osc = g.add_node(GraphNode::new("oscillator", vec![], vec![ao("out")]));
        let amp = g.add_node(GraphNode::new("gain", vec![ai("in")], vec![ao("out")]));
        g.connect(osc, PortId(0), amp, PortId(0)).unwrap();
        ProjectDoc::from_graph(&g)
    }

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
    fn consumers_accept_by_signal_type() {
        let r = Registry::with_builtins();
        let audio = compatible_consumers(&r, SignalType::Audio);
        assert!(
            audio.contains(&"gain") && audio.contains(&"biquad") && audio.contains(&"audio-out")
        );
        // A pure source has no inputs, so it never consumes.
        assert!(!audio.contains(&"oscillator"));
        // Control consumers include the envelope's gate input.
        let control = compatible_consumers(&r, SignalType::Control);
        assert!(control.contains(&"adsr"));
        assert!(!control.contains(&"gain"), "gain takes audio, not control");
    }

    #[test]
    fn free_outputs_are_the_unconnected_ports() {
        let d = osc_gain();
        let free = free_outputs(&d);
        // osc.out feeds gain, so only gain.out is free.
        assert_eq!(free.len(), 1);
        assert_eq!(free[0].signal, SignalType::Audio);
        let gain_id = d.nodes.iter().find(|e| e.node.kind == "gain").unwrap().id;
        assert_eq!(free[0].node, gain_id);
    }

    #[test]
    fn suggest_next_offers_compatible_consumers_for_each_free_output() {
        let r = Registry::with_builtins();
        let suggestions = suggest_next(&osc_gain(), &r);
        assert_eq!(suggestions.len(), 1, "one free output");
        assert!(suggestions[0].kinds.contains(&"biquad"));
        assert!(!suggestions[0].kinds.contains(&"oscillator"));
    }

    #[test]
    fn autofix_leaves_a_sound_patch_untouched() {
        let d = osc_gain();
        let (fixed, fixes) = autofix(&d);
        assert!(fixes.is_empty());
        assert_eq!(fixed, d);
    }

    #[test]
    fn autofix_drops_a_signal_mismatch() {
        // audio out → control in.
        let d = doc(
            vec![
                node(0, "oscillator", vec![], vec![ao("out")]),
                node(
                    1,
                    "adsr",
                    vec![Port::input("gate", SignalType::Control)],
                    vec![],
                ),
            ],
            vec![edge(0, 0, 1, 0)],
        );
        assert!(!d.is_valid(), "the mismatch makes it invalid");
        let (fixed, fixes) = autofix(&d);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].defect, EdgeDefect::SignalMismatch);
        assert!(fixed.edges.is_empty());
        assert!(fixed.is_valid(), "repaired");
    }

    #[test]
    fn autofix_drops_dangling_and_out_of_range_edges() {
        // Edge to a missing node, and an out-of-range output port.
        let d = doc(
            vec![node(0, "gain", vec![ai("in")], vec![ao("out")])],
            vec![edge(0, 5, 0, 0), edge(0, 0, 9, 0)],
        );
        let (fixed, fixes) = autofix(&d);
        assert_eq!(fixes.len(), 2);
        let defects: Vec<_> = fixes.iter().map(|f| f.defect).collect();
        assert!(defects.contains(&EdgeDefect::PortOutOfRange));
        assert!(defects.contains(&EdgeDefect::UnknownEndpoint));
        assert!(fixed.edges.is_empty());
    }

    #[test]
    fn autofix_keeps_the_first_edge_into_an_input_and_drops_the_rest() {
        // Two sources into gain.in: keep the first, drop the second.
        let d = doc(
            vec![
                node(0, "oscillator", vec![], vec![ao("out")]),
                node(1, "noise", vec![], vec![ao("out")]),
                node(2, "gain", vec![ai("in")], vec![ao("out")]),
            ],
            vec![edge(0, 0, 2, 0), edge(1, 0, 2, 0)],
        );
        let (fixed, fixes) = autofix(&d);
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].defect, EdgeDefect::InputOccupied);
        assert_eq!(fixes[0].edge_index, 1, "the second edge is the duplicate");
        assert_eq!(fixed.edges, vec![edge(0, 0, 2, 0)]);
        assert!(fixed.is_valid());
    }
}
