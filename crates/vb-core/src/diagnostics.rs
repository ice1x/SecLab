//! Static graph diagnostics the editor can surface before running a patch.

use crate::graph::Graph;
use crate::node::NodeId;
use crate::port::PortId;

/// A problem detected in the graph.
///
/// These are advisory: an unconnected input is legal (it reads as silence) but usually
/// worth flagging; a cycle, on the other hand, prevents scheduling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    /// An input port has no incoming edge, so it will read as silence.
    UnconnectedInput {
        /// The node owning the input.
        node: NodeId,
        /// The unconnected input port.
        port: PortId,
    },
    /// The graph contains a cycle and cannot be scheduled as-is.
    Cycle,
}

impl Graph {
    /// Collects advisory diagnostics: every unconnected input (in a stable order by node
    /// id then port), followed by a [`Diagnostic::Cycle`] marker if the graph has a cycle.
    #[must_use]
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut ids: Vec<NodeId> = self.node_ids().collect();
        ids.sort_unstable_by_key(|id| id.0);

        let mut out = Vec::new();
        for id in ids {
            let node = self.node(id).expect("id from node_ids is valid");
            for port in 0..node.inputs.len() {
                let connected = self
                    .edges()
                    .iter()
                    .any(|e| e.to_node == id && e.to_port.0 == port);
                if !connected {
                    out.push(Diagnostic::UnconnectedInput {
                        node: id,
                        port: PortId(port),
                    });
                }
            }
        }
        if self.has_cycle() {
            out.push(Diagnostic::Cycle);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Diagnostic;
    use crate::graph::Graph;
    use crate::node::GraphNode;
    use crate::port::{Port, PortId};
    use crate::signal::SignalType;

    fn source() -> GraphNode {
        GraphNode::new("src", vec![], vec![Port::output("out", SignalType::Audio)])
    }

    fn mono(kind: &str) -> GraphNode {
        GraphNode::new(
            kind,
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        )
    }

    #[test]
    fn fully_wired_chain_has_no_diagnostics() {
        let mut g = Graph::new();
        let a = g.add_node(source());
        let b = g.add_node(mono("b"));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        assert!(g.diagnostics().is_empty());
    }

    #[test]
    fn unconnected_input_is_reported() {
        let mut g = Graph::new();
        let b = g.add_node(mono("b"));
        assert_eq!(
            g.diagnostics(),
            vec![Diagnostic::UnconnectedInput {
                node: b,
                port: PortId(0)
            }]
        );
    }

    #[test]
    fn reports_every_dangling_input() {
        let mut g = Graph::new();
        let a = g.add_node(source());
        let mixer = g.add_node(GraphNode::new(
            "mix",
            vec![
                Port::input("l", SignalType::Audio),
                Port::input("r", SignalType::Audio),
            ],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(a, PortId(0), mixer, PortId(0)).unwrap();
        // Right input left dangling.
        assert_eq!(
            g.diagnostics(),
            vec![Diagnostic::UnconnectedInput {
                node: mixer,
                port: PortId(1)
            }]
        );
    }

    #[test]
    fn cycle_is_flagged() {
        let mut g = Graph::new();
        let a = g.add_node(mono("a"));
        let b = g.add_node(mono("b"));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        g.connect(b, PortId(0), a, PortId(0)).unwrap();
        assert!(g.diagnostics().contains(&Diagnostic::Cycle));
    }
}
