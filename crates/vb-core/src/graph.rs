//! The graph container: nodes, edges, and the connection rules between ports.

use std::collections::HashMap;
use std::fmt;

use crate::node::{GraphNode, NodeId};
use crate::port::{PortDirection, PortId};

/// A directed connection from an output port to an input port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Edge {
    /// Source node (the output side).
    pub from_node: NodeId,
    /// Index of the output port on `from_node`.
    pub from_port: PortId,
    /// Destination node (the input side).
    pub to_node: NodeId,
    /// Index of the input port on `to_node`.
    pub to_port: PortId,
}

/// Reasons a graph mutation can be rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// A referenced node does not exist in the graph.
    UnknownNode(NodeId),
    /// A referenced port index is out of range on the given node/side.
    UnknownPort {
        /// The node the port was looked up on.
        node: NodeId,
        /// The out-of-range port index.
        port: PortId,
        /// Which side (input/output) was addressed.
        direction: PortDirection,
    },
    /// The output and input signal types are not compatible.
    SignalMismatch,
    /// The target input port already has an incoming edge (inputs are single-source).
    InputAlreadyConnected {
        /// The already-connected target node.
        node: NodeId,
        /// The already-connected target input port.
        port: PortId,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(f, "unknown node {id:?}"),
            Self::UnknownPort {
                node,
                port,
                direction,
            } => {
                write!(f, "unknown {direction:?} port {port:?} on node {node:?}")
            }
            Self::SignalMismatch => write!(f, "incompatible signal types between ports"),
            Self::InputAlreadyConnected { node, port } => {
                write!(
                    f,
                    "input port {port:?} on node {node:?} is already connected"
                )
            }
        }
    }
}

impl std::error::Error for GraphError {}

/// A directed graph of typed nodes and the edges that wire their ports together.
///
/// The graph enforces the structural rules at connect time: referenced nodes/ports
/// must exist, an output may only feed an input, the signal types must be compatible,
/// and each input accepts a single source. Cycle detection and processing order are
/// handled by later tasks (000015, 000016).
#[derive(Debug, Default)]
pub struct Graph {
    next_id: u64,
    nodes: HashMap<NodeId, GraphNode>,
    edges: Vec<Edge>,
}

impl Graph {
    /// Creates an empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a node and returns its freshly allocated identifier.
    pub fn add_node(&mut self, node: GraphNode) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        self.nodes.insert(id, node);
        id
    }

    /// Returns the node for `id`, if present.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&GraphNode> {
        self.nodes.get(&id)
    }

    /// Number of nodes currently in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// All edges currently in the graph.
    #[must_use]
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Connects an output port to an input port, validating the connection.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError`] if either node or port is unknown, the signal types are
    /// incompatible, or the target input is already connected.
    pub fn connect(
        &mut self,
        from_node: NodeId,
        from_port: PortId,
        to_node: NodeId,
        to_port: PortId,
    ) -> Result<Edge, GraphError> {
        let source = self
            .nodes
            .get(&from_node)
            .ok_or(GraphError::UnknownNode(from_node))?;
        let out_signal = source
            .outputs
            .get(from_port.0)
            .ok_or(GraphError::UnknownPort {
                node: from_node,
                port: from_port,
                direction: PortDirection::Output,
            })?
            .signal;

        let target = self
            .nodes
            .get(&to_node)
            .ok_or(GraphError::UnknownNode(to_node))?;
        let in_signal = target
            .inputs
            .get(to_port.0)
            .ok_or(GraphError::UnknownPort {
                node: to_node,
                port: to_port,
                direction: PortDirection::Input,
            })?
            .signal;

        if !out_signal.compatible_with(in_signal) {
            return Err(GraphError::SignalMismatch);
        }

        if self
            .edges
            .iter()
            .any(|e| e.to_node == to_node && e.to_port == to_port)
        {
            return Err(GraphError::InputAlreadyConnected {
                node: to_node,
                port: to_port,
            });
        }

        let edge = Edge {
            from_node,
            from_port,
            to_node,
            to_port,
        };
        self.edges.push(edge);
        Ok(edge)
    }

    /// Removes the edge feeding `to_node`'s input `to_port`, if any, and returns it.
    ///
    /// After this the input is free and may be reconnected.
    pub fn disconnect(&mut self, to_node: NodeId, to_port: PortId) -> Option<Edge> {
        let pos = self
            .edges
            .iter()
            .position(|e| e.to_node == to_node && e.to_port == to_port)?;
        Some(self.edges.remove(pos))
    }

    /// Removes a node and every edge incident to it, returning the node if it existed.
    pub fn remove_node(&mut self, id: NodeId) -> Option<GraphNode> {
        let removed = self.nodes.remove(&id)?;
        self.edges.retain(|e| e.from_node != id && e.to_node != id);
        Some(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::{Graph, GraphError};
    use crate::node::GraphNode;
    use crate::port::{Port, PortId};
    use crate::signal::SignalType;

    fn osc() -> GraphNode {
        GraphNode::new("osc", vec![], vec![Port::output("out", SignalType::Audio)])
    }

    fn gain() -> GraphNode {
        GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        )
    }

    #[test]
    fn add_node_assigns_unique_ids_and_counts() {
        let mut g = Graph::new();
        let a = g.add_node(osc());
        let b = g.add_node(gain());
        assert_ne!(a, b);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.node(a).unwrap().kind, "osc");
    }

    #[test]
    fn connect_compatible_ports_succeeds() {
        let mut g = Graph::new();
        let a = g.add_node(osc());
        let b = g.add_node(gain());
        let edge = g.connect(a, PortId(0), b, PortId(0)).unwrap();
        assert_eq!(edge.from_node, a);
        assert_eq!(edge.to_node, b);
        assert_eq!(g.edges().len(), 1);
    }

    #[test]
    fn connect_unknown_node_is_rejected() {
        let mut g = Graph::new();
        let a = g.add_node(osc());
        let missing = crate::node::NodeId(999);
        assert_eq!(
            g.connect(a, PortId(0), missing, PortId(0)),
            Err(GraphError::UnknownNode(missing))
        );
    }

    #[test]
    fn connect_unknown_port_is_rejected() {
        let mut g = Graph::new();
        let a = g.add_node(osc());
        let b = g.add_node(gain());
        // osc has a single output (index 0); index 1 is out of range.
        assert!(matches!(
            g.connect(a, PortId(1), b, PortId(0)),
            Err(GraphError::UnknownPort { .. })
        ));
    }

    #[test]
    fn connect_signal_mismatch_is_rejected() {
        let mut g = Graph::new();
        let a = g.add_node(osc()); // audio output
        let ctrl = g.add_node(GraphNode::new(
            "ctrl-sink",
            vec![Port::input("cv", SignalType::Control)],
            vec![],
        ));
        assert_eq!(
            g.connect(a, PortId(0), ctrl, PortId(0)),
            Err(GraphError::SignalMismatch)
        );
    }

    #[test]
    fn input_accepts_a_single_source() {
        let mut g = Graph::new();
        let a = g.add_node(osc());
        let b = g.add_node(osc());
        let sink = g.add_node(gain());
        g.connect(a, PortId(0), sink, PortId(0)).unwrap();
        assert_eq!(
            g.connect(b, PortId(0), sink, PortId(0)),
            Err(GraphError::InputAlreadyConnected {
                node: sink,
                port: PortId(0)
            })
        );
    }

    #[test]
    fn disconnect_frees_the_input_for_reconnection() {
        let mut g = Graph::new();
        let a = g.add_node(osc());
        let b = g.add_node(osc());
        let sink = g.add_node(gain());
        g.connect(a, PortId(0), sink, PortId(0)).unwrap();

        let removed = g.disconnect(sink, PortId(0)).expect("edge exists");
        assert_eq!(removed.from_node, a);
        assert_eq!(g.edges().len(), 0);

        // The input is free again, so a different source connects fine.
        g.connect(b, PortId(0), sink, PortId(0))
            .expect("reconnect after disconnect");
        assert_eq!(g.edges().len(), 1);
    }

    #[test]
    fn disconnect_unconnected_input_returns_none() {
        let mut g = Graph::new();
        let sink = g.add_node(gain());
        assert_eq!(g.disconnect(sink, PortId(0)), None);
    }

    #[test]
    fn remove_node_drops_incident_edges() {
        let mut g = Graph::new();
        let a = g.add_node(osc());
        let mid = g.add_node(gain());
        let sink = g.add_node(gain());
        g.connect(a, PortId(0), mid, PortId(0)).unwrap();
        g.connect(mid, PortId(0), sink, PortId(0)).unwrap();
        assert_eq!(g.edges().len(), 2);

        let removed = g.remove_node(mid).expect("node exists");
        assert_eq!(removed.kind, "gain");
        assert_eq!(g.node_count(), 2);
        // Both edges touched `mid`, so both are gone.
        assert_eq!(g.edges().len(), 0);
    }

    #[test]
    fn remove_unknown_node_returns_none() {
        let mut g = Graph::new();
        assert_eq!(g.remove_node(crate::node::NodeId(42)), None);
    }
}
