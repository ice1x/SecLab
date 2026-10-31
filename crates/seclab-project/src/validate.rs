//! Structural validation of a parsed project document.
//!
//! Parsing only proves the RON was well-formed. A file can still be corrupt, hand-edited
//! or produced by another tool, so before a document is trusted it is checked against the
//! same rules the runtime graph enforces — ids are unique, edges point at real ports,
//! signal types line up, inputs are single-source, and the patch is acyclic.

use std::collections::HashSet;
use std::fmt;

use seclab_core::{NodeId, PortDirection, PortId};

use crate::doc::ProjectDoc;

/// A single problem found in a project document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// Two node entries share the same id.
    DuplicateNodeId(NodeId),
    /// A node has an empty kind label, so it cannot be resolved to a node type.
    EmptyKind(NodeId),
    /// An edge references a node id that is not in the document.
    UnknownEdgeEndpoint {
        /// Index of the offending edge.
        edge: usize,
        /// The id that could not be resolved.
        node: NodeId,
    },
    /// An edge references a port index the node does not have.
    PortOutOfRange {
        /// Index of the offending edge.
        edge: usize,
        /// The node the port was looked up on.
        node: NodeId,
        /// The out-of-range port index.
        port: PortId,
        /// Which side (input/output) was addressed.
        direction: PortDirection,
    },
    /// An edge connects ports whose signal types are incompatible.
    SignalMismatch {
        /// Index of the offending edge.
        edge: usize,
    },
    /// More than one edge feeds the same input port.
    InputAlreadyConnected {
        /// The target node.
        node: NodeId,
        /// The target input port.
        port: PortId,
    },
    /// The patch contains a cycle, so it cannot be scheduled.
    Cycle,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNodeId(id) => write!(f, "duplicate node id {id:?}"),
            Self::EmptyKind(id) => write!(f, "node {id:?} has an empty kind"),
            Self::UnknownEdgeEndpoint { edge, node } => {
                write!(f, "edge {edge} references unknown node {node:?}")
            }
            Self::PortOutOfRange {
                edge,
                node,
                port,
                direction,
            } => write!(
                f,
                "edge {edge} references {direction:?} port {port:?} out of range on node {node:?}"
            ),
            Self::SignalMismatch { edge } => {
                write!(f, "edge {edge} connects incompatible signal types")
            }
            Self::InputAlreadyConnected { node, port } => {
                write!(
                    f,
                    "input port {port:?} on node {node:?} has more than one source"
                )
            }
            Self::Cycle => write!(f, "patch contains a cycle"),
        }
    }
}

impl ProjectDoc {
    /// Checks the document against the graph's structural rules.
    ///
    /// Returns every problem found (empty when the document is sound) rather than
    /// stopping at the first, so a corrupt file can be reported in one pass.
    #[must_use]
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Vec::new();

        let mut seen_ids: HashSet<NodeId> = HashSet::new();
        for entry in &self.nodes {
            if !seen_ids.insert(entry.id) {
                issues.push(Issue::DuplicateNodeId(entry.id));
            }
            if entry.node.kind.trim().is_empty() {
                issues.push(Issue::EmptyKind(entry.id));
            }
        }

        let mut occupied_inputs: HashSet<(NodeId, PortId)> = HashSet::new();
        for (index, edge) in self.edges.iter().enumerate() {
            let source = self.node(edge.from_node);
            let target = self.node(edge.to_node);

            if source.is_none() {
                issues.push(Issue::UnknownEdgeEndpoint {
                    edge: index,
                    node: edge.from_node,
                });
            }
            if target.is_none() {
                issues.push(Issue::UnknownEdgeEndpoint {
                    edge: index,
                    node: edge.to_node,
                });
            }
            let (Some(source), Some(target)) = (source, target) else {
                continue;
            };

            let out_port = source.outputs.get(edge.from_port.0);
            let in_port = target.inputs.get(edge.to_port.0);
            if out_port.is_none() {
                issues.push(Issue::PortOutOfRange {
                    edge: index,
                    node: edge.from_node,
                    port: edge.from_port,
                    direction: PortDirection::Output,
                });
            }
            if in_port.is_none() {
                issues.push(Issue::PortOutOfRange {
                    edge: index,
                    node: edge.to_node,
                    port: edge.to_port,
                    direction: PortDirection::Input,
                });
            }
            let (Some(out_port), Some(in_port)) = (out_port, in_port) else {
                continue;
            };

            if !out_port.signal.compatible_with(in_port.signal) {
                issues.push(Issue::SignalMismatch { edge: index });
            }
            if !occupied_inputs.insert((edge.to_node, edge.to_port)) {
                issues.push(Issue::InputAlreadyConnected {
                    node: edge.to_node,
                    port: edge.to_port,
                });
            }
        }

        // Only meaningful once the wiring itself is sound. Every way `to_graph` can fail
        // (unknown node/port, signal mismatch, double-connected input) is already covered
        // above, so a failure here cannot add information.
        if issues.is_empty() && self.to_graph().is_ok_and(|g| g.has_cycle()) {
            issues.push(Issue::Cycle);
        }

        issues
    }

    /// Returns `true` when [`ProjectDoc::validate`] finds no problems.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_empty()
    }
}
