//! Cycle detection and processing order for the graph (Kahn's algorithm).

use std::collections::HashMap;

use crate::graph::{Graph, GraphError};
use crate::node::NodeId;

impl Graph {
    /// Returns node ids in a valid processing order: every node appears after all of
    /// the nodes that feed its inputs. The order is deterministic (ties broken by id).
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::Cycle`] if the graph contains a cycle, in which case no
    /// processing order exists.
    pub fn topological_order(&self) -> Result<Vec<NodeId>, GraphError> {
        // In-degree = number of incoming edges per node. Edges into a feedback
        // node are not dependencies: that node reads the previous block, so its
        // sources do not need to run first.
        let depends = |to: NodeId| !self.node(to).is_some_and(|n| n.feedback);
        let mut in_degree: HashMap<NodeId, usize> = self.node_ids().map(|id| (id, 0)).collect();
        for edge in self.edges() {
            if !depends(edge.to_node) {
                continue;
            }
            if let Some(deg) = in_degree.get_mut(&edge.to_node) {
                *deg += 1;
            }
        }

        // Seed the ready set with all sources (in-degree 0), lowest id first.
        let mut ready: Vec<NodeId> = in_degree
            .iter()
            .filter(|&(_, &d)| d == 0)
            .map(|(&id, _)| id)
            .collect();
        ready.sort_unstable_by_key(|id| id.0);

        let mut order = Vec::with_capacity(in_degree.len());
        while let Some(id) = ready.pop() {
            order.push(id);
            let mut freed: Vec<NodeId> = Vec::new();
            for edge in self.edges().iter().filter(|e| e.from_node == id) {
                if !depends(edge.to_node) {
                    continue;
                }
                if let Some(deg) = in_degree.get_mut(&edge.to_node) {
                    *deg -= 1;
                    if *deg == 0 {
                        freed.push(edge.to_node);
                    }
                }
            }
            // Keep the highest id at the end so `pop` yields the lowest next.
            freed.sort_unstable_by_key(|id| std::cmp::Reverse(id.0));
            ready.extend(freed);
        }

        if order.len() == self.node_count() {
            Ok(order)
        } else {
            Err(GraphError::Cycle)
        }
    }

    /// Returns `true` if the graph contains a cycle.
    #[must_use]
    pub fn has_cycle(&self) -> bool {
        self.topological_order().is_err()
    }
}

#[cfg(test)]
mod tests {
    use crate::graph::{Graph, GraphError};
    use crate::node::{GraphNode, NodeId};
    use crate::port::{Port, PortId};
    use crate::signal::SignalType;

    /// A pass-through node with one audio input and one audio output.
    fn thru(kind: &str) -> GraphNode {
        GraphNode::new(
            kind,
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        )
    }

    fn source() -> GraphNode {
        GraphNode::new("src", vec![], vec![Port::output("out", SignalType::Audio)])
    }

    fn index_of(order: &[NodeId], id: NodeId) -> usize {
        order.iter().position(|&n| n == id).expect("node in order")
    }

    #[test]
    fn empty_graph_orders_to_nothing() {
        let g = Graph::new();
        assert_eq!(g.topological_order().unwrap(), vec![]);
        assert!(!g.has_cycle());
    }

    #[test]
    fn chain_orders_sources_before_sinks() {
        let mut g = Graph::new();
        let a = g.add_node(source());
        let b = g.add_node(thru("b"));
        let c = g.add_node(thru("c"));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        g.connect(b, PortId(0), c, PortId(0)).unwrap();

        let order = g.topological_order().unwrap();
        assert_eq!(order.len(), 3);
        assert!(index_of(&order, a) < index_of(&order, b));
        assert!(index_of(&order, b) < index_of(&order, c));
        assert!(!g.has_cycle());
    }

    #[test]
    fn diamond_respects_all_dependencies() {
        // a -> b, a -> c, b -> d, c -> d
        let mut g = Graph::new();
        let a = g.add_node(source());
        let b = g.add_node(thru("b"));
        let c = g.add_node(thru("c"));
        let d = g.add_node(GraphNode::new(
            "d",
            vec![
                Port::input("l", SignalType::Audio),
                Port::input("r", SignalType::Audio),
            ],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        g.connect(a, PortId(0), c, PortId(0)).unwrap();
        g.connect(b, PortId(0), d, PortId(0)).unwrap();
        g.connect(c, PortId(0), d, PortId(1)).unwrap();

        let order = g.topological_order().unwrap();
        assert!(index_of(&order, a) < index_of(&order, b));
        assert!(index_of(&order, a) < index_of(&order, c));
        assert!(index_of(&order, b) < index_of(&order, d));
        assert!(index_of(&order, c) < index_of(&order, d));
    }

    #[test]
    fn cycle_is_detected() {
        let mut g = Graph::new();
        let a = g.add_node(thru("a"));
        let b = g.add_node(thru("b"));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        g.connect(b, PortId(0), a, PortId(0)).unwrap();

        assert!(g.has_cycle());
        assert!(g.topological_order().is_err());
    }

    #[test]
    fn self_loop_is_a_cycle() {
        let mut g = Graph::new();
        let a = g.add_node(thru("a"));
        g.connect(a, PortId(0), a, PortId(0)).unwrap();
        assert!(g.has_cycle());
    }

    #[test]
    fn cycles_through_a_feedback_node_are_ordered_not_rejected() {
        let mut g = Graph::new();
        let fb = g.add_node(GraphNode::feedback_node(
            "fb",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let amp = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(fb, PortId(0), amp, PortId(0)).unwrap();
        g.connect(amp, PortId(0), fb, PortId(0)).unwrap(); // closes the loop

        assert!(
            !g.has_cycle(),
            "the loop is broken by the feedback boundary"
        );
        let order = g.topological_order().unwrap();
        // The feedback node has no live dependencies, so it runs first and reads
        // the gain's previous block.
        assert_eq!(order, vec![fb, amp]);
    }

    #[test]
    fn delayless_cycles_are_still_rejected() {
        let mut g = Graph::new();
        let a = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let b = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        g.connect(b, PortId(0), a, PortId(0)).unwrap();
        assert!(g.has_cycle());
        assert_eq!(g.topological_order(), Err(GraphError::Cycle));
    }
}
