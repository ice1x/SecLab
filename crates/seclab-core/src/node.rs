//! Structural model of a node in the graph.

use crate::port::Port;

/// Stable identifier for a node within a [`crate::graph::Graph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeId(pub u64);

/// A node instance: a `kind` label plus its typed input and output ports.
///
/// This is the structural model only — DSP processing behaviour arrives with the
/// `Node` processing trait in a later task (000013). Keeping data and behaviour
/// separate keeps the graph model easy to serialize, diff and reason about.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GraphNode {
    /// Registry kind label (e.g. `"oscillator"`).
    pub kind: String,
    /// Input ports, addressed by index via [`crate::port::PortId`].
    pub inputs: Vec<Port>,
    /// Output ports, addressed by index via [`crate::port::PortId`].
    pub outputs: Vec<Port>,
    /// A feedback boundary: this node's inputs read the *previous* block, so cycles
    /// running through them are legal (they carry one block of latency instead of an
    /// unsolvable instantaneous loop). Delay-line nodes set this.
    #[serde(default)]
    pub feedback: bool,
}

impl GraphNode {
    /// Creates a node of the given `kind` with the supplied ports.
    #[must_use]
    pub fn new(kind: impl Into<String>, inputs: Vec<Port>, outputs: Vec<Port>) -> Self {
        Self {
            kind: kind.into(),
            inputs,
            outputs,
            feedback: false,
        }
    }

    /// Creates a feedback-boundary node: cycles through its inputs are allowed and
    /// arrive one block late.
    #[must_use]
    pub fn feedback_node(kind: impl Into<String>, inputs: Vec<Port>, outputs: Vec<Port>) -> Self {
        Self {
            feedback: true,
            ..Self::new(kind, inputs, outputs)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GraphNode;
    use crate::port::Port;
    use crate::signal::SignalType;

    #[test]
    fn stores_kind_and_ports() {
        let n = GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        );
        assert_eq!(n.kind, "gain");
        assert_eq!(n.inputs.len(), 1);
        assert_eq!(n.outputs.len(), 1);
    }
}
