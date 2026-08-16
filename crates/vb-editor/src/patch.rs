//! The patch as the editor sees it: the [`vb_core::Graph`] plus canvas positions,
//! draw order, and selection.
//!
//! `vb-core` stays the single source of truth for structure (nodes, ports, edges);
//! this view only adds what the canvas needs — where each node sits, which one is
//! selected, and who is on top.

use std::collections::HashMap;

use egui::Pos2;
use vb_core::{Edge, Graph, GraphNode, NodeId, PortDirection, PortId};

use crate::layout::{hit_test, layout_node, NodeHit, NodeStyle};

/// An in-progress wire drag: the port it started from.
///
/// Either end can start the drag; [`PatchView::end_wire`] normalizes the direction so
/// the edge always runs output → input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDrag {
    /// Node the drag started on.
    pub node: NodeId,
    /// Which side of the node the grabbed port is on.
    pub direction: PortDirection,
    /// The grabbed port.
    pub port: PortId,
}

/// Editor-side view of a patch: graph + positions + selection.
#[derive(Default)]
pub struct PatchView {
    graph: Graph,
    positions: HashMap<NodeId, Pos2>,
    /// Draw order, back to front. Hit-testing walks it front to back.
    order: Vec<NodeId>,
    selected: Option<NodeId>,
    /// Wire drag in progress, if any.
    drag: Option<WireDrag>,
}

impl PatchView {
    /// Creates an empty patch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The underlying structural graph.
    #[must_use]
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Adds `node` to the graph at canvas position `pos` (top of the draw order).
    pub fn add_node(&mut self, node: GraphNode, pos: Pos2) -> NodeId {
        let id = self.graph.add_node(node);
        self.positions.insert(id, pos);
        self.order.push(id);
        id
    }

    /// Connects an output to an input directly (programmatic wiring, e.g. seeding).
    ///
    /// # Errors
    /// Propagates [`vb_core::GraphError`] from [`Graph::connect`].
    pub fn connect(
        &mut self,
        from: NodeId,
        from_port: PortId,
        to: NodeId,
        to_port: PortId,
    ) -> Result<Edge, vb_core::GraphError> {
        self.graph.connect(from, from_port, to, to_port)
    }

    /// Canvas position of a node's top-left corner.
    #[must_use]
    pub fn position(&self, id: NodeId) -> Option<Pos2> {
        self.positions.get(&id).copied()
    }

    /// Nodes in draw order (back to front), with their positions.
    pub fn draw_order(&self) -> impl Iterator<Item = (NodeId, &GraphNode, Pos2)> + '_ {
        self.order.iter().filter_map(|id| {
            let node = self.graph.node(*id)?;
            let pos = self.positions.get(id)?;
            Some((*id, node, *pos))
        })
    }

    /// Currently selected node, if any.
    #[must_use]
    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    /// Topmost node under `p` (canvas coords) and what part of it was hit.
    #[must_use]
    pub fn node_at(&self, p: Pos2, style: &NodeStyle) -> Option<(NodeId, NodeHit)> {
        for id in self.order.iter().rev() {
            let (node, pos) = (self.graph.node(*id)?, *self.positions.get(id)?);
            if let Some(hit) = hit_test(&layout_node(node, pos, style), p, style) {
                return Some((*id, hit));
            }
        }
        None
    }

    /// Canvas position of a port anchor, for painting wires.
    #[must_use]
    pub fn port_anchor(
        &self,
        node: NodeId,
        direction: PortDirection,
        port: PortId,
        style: &NodeStyle,
    ) -> Option<Pos2> {
        let layout = layout_node(self.graph.node(node)?, *self.positions.get(&node)?, style);
        match direction {
            PortDirection::Input => layout.inputs.get(port.0).copied(),
            PortDirection::Output => layout.outputs.get(port.0).copied(),
        }
    }

    /// Starts a wire drag if `p` lands on a port; returns the drag state.
    ///
    /// Non-port hits (title, body, empty canvas) start nothing — the caller falls back
    /// to selection/panning.
    pub fn begin_wire(&mut self, p: Pos2, style: &NodeStyle) -> Option<WireDrag> {
        let drag = match self.node_at(p, style) {
            Some((node, NodeHit::Input(port))) => Some(WireDrag {
                node,
                direction: PortDirection::Input,
                port,
            }),
            Some((node, NodeHit::Output(port))) => Some(WireDrag {
                node,
                direction: PortDirection::Output,
                port,
            }),
            _ => None,
        };
        self.drag = drag;
        drag
    }

    /// The wire drag in progress, if any.
    #[must_use]
    pub fn wire_drag(&self) -> Option<WireDrag> {
        self.drag
    }

    /// Finishes the wire drag at `p`: connects if `p` lands on a port of the opposite
    /// direction and `vb-core` accepts the edge (signal types match, input free, no
    /// cycle). Returns the created edge; any miss or rejection just clears the drag.
    pub fn end_wire(&mut self, p: Pos2, style: &NodeStyle) -> Option<Edge> {
        let drag = self.drag.take()?;
        let (from, to) = match (drag.direction, self.node_at(p, style)?) {
            (PortDirection::Output, (node, NodeHit::Input(port))) => {
                ((drag.node, drag.port), (node, port))
            }
            (PortDirection::Input, (node, NodeHit::Output(port))) => {
                ((node, port), (drag.node, drag.port))
            }
            _ => return None,
        };
        self.graph.connect(from.0, from.1, to.0, to.1).ok()
    }

    /// Handles a click at `p`: selects the topmost node there (raising it to the top
    /// of the draw order) or clears the selection on empty canvas. Returns the hit.
    pub fn click(&mut self, p: Pos2, style: &NodeStyle) -> Option<(NodeId, NodeHit)> {
        let hit = self.node_at(p, style);
        match hit {
            Some((id, _)) => {
                self.selected = Some(id);
                if let Some(i) = self.order.iter().position(|n| *n == id) {
                    let id = self.order.remove(i);
                    self.order.push(id);
                }
            }
            None => self.selected = None,
        }
        hit
    }
}

#[cfg(test)]
mod tests {
    use vb_core::{Port, SignalType};

    use super::*;

    fn gain() -> GraphNode {
        GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        )
    }

    fn style() -> NodeStyle {
        NodeStyle::default()
    }

    #[test]
    fn click_selects_the_node_under_the_cursor() {
        let mut patch = PatchView::new();
        let id = patch.add_node(gain(), Pos2::new(100.0, 100.0));
        let hit = patch.click(Pos2::new(110.0, 110.0), &style());
        assert_eq!(hit.map(|(id, _)| id), Some(id));
        assert_eq!(patch.selected(), Some(id));
    }

    #[test]
    fn click_on_empty_canvas_clears_the_selection() {
        let mut patch = PatchView::new();
        patch.add_node(gain(), Pos2::new(100.0, 100.0));
        patch.click(Pos2::new(110.0, 110.0), &style());
        assert!(patch.selected().is_some());
        patch.click(Pos2::new(-500.0, -500.0), &style());
        assert_eq!(patch.selected(), None);
    }

    #[test]
    fn overlapping_nodes_hit_the_topmost_and_raise_it() {
        let mut patch = PatchView::new();
        let below = patch.add_node(gain(), Pos2::new(100.0, 100.0));
        let above = patch.add_node(gain(), Pos2::new(110.0, 110.0)); // overlaps, added later

        let p = Pos2::new(115.0, 115.0); // inside both bodies
        assert_eq!(patch.click(p, &style()).map(|(id, _)| id), Some(above));

        // Clicking raises: select the lower one where it peeks out, then the shared
        // point must now hit it first.
        let peek = Pos2::new(102.0, 104.0); // only inside `below`'s title area
        assert_eq!(patch.click(peek, &style()).map(|(id, _)| id), Some(below));
        assert_eq!(patch.node_at(p, &style()).map(|(id, _)| id), Some(below));
    }

    /// Two gains side by side, far enough apart not to overlap.
    fn two_gains() -> (PatchView, NodeId, NodeId) {
        let mut patch = PatchView::new();
        let a = patch.add_node(gain(), Pos2::new(0.0, 0.0));
        let b = patch.add_node(gain(), Pos2::new(300.0, 0.0));
        (patch, a, b)
    }

    #[test]
    fn wire_drag_starts_only_on_ports() {
        let (mut patch, a, _) = two_gains();
        let s = style();
        let out = patch
            .port_anchor(a, PortDirection::Output, PortId(0), &s)
            .unwrap();

        assert!(patch.begin_wire(out, &s).is_some());
        assert_eq!(
            patch.wire_drag(),
            Some(WireDrag {
                node: a,
                direction: PortDirection::Output,
                port: PortId(0)
            })
        );

        // Title, body, empty canvas: no drag.
        assert!(patch.begin_wire(Pos2::new(60.0, 5.0), &s).is_none());
        assert!(patch.begin_wire(Pos2::new(-999.0, -999.0), &s).is_none());
        assert_eq!(patch.wire_drag(), None);
    }

    #[test]
    fn dragging_output_to_input_creates_the_edge() {
        let (mut patch, a, b) = two_gains();
        let s = style();
        let out = patch
            .port_anchor(a, PortDirection::Output, PortId(0), &s)
            .unwrap();
        let inp = patch
            .port_anchor(b, PortDirection::Input, PortId(0), &s)
            .unwrap();

        patch.begin_wire(out, &s);
        let edge = patch.end_wire(inp, &s).expect("edge created");
        assert_eq!((edge.from_node, edge.to_node), (a, b));
        assert_eq!(patch.graph().edges().len(), 1);
        assert_eq!(patch.wire_drag(), None);
    }

    #[test]
    fn dragging_input_to_output_normalizes_the_direction() {
        let (mut patch, a, b) = two_gains();
        let s = style();
        let inp = patch
            .port_anchor(b, PortDirection::Input, PortId(0), &s)
            .unwrap();
        let out = patch
            .port_anchor(a, PortDirection::Output, PortId(0), &s)
            .unwrap();

        patch.begin_wire(inp, &s);
        let edge = patch.end_wire(out, &s).expect("edge created");
        // Still output → input even though the drag started at the input.
        assert_eq!((edge.from_node, edge.to_node), (a, b));
    }

    #[test]
    fn dropping_on_nothing_or_same_direction_connects_nothing() {
        let (mut patch, a, b) = two_gains();
        let s = style();
        let out_a = patch
            .port_anchor(a, PortDirection::Output, PortId(0), &s)
            .unwrap();
        let out_b = patch
            .port_anchor(b, PortDirection::Output, PortId(0), &s)
            .unwrap();

        patch.begin_wire(out_a, &s);
        assert!(patch.end_wire(Pos2::new(-999.0, -999.0), &s).is_none());
        assert_eq!(patch.wire_drag(), None, "miss still clears the drag");

        patch.begin_wire(out_a, &s);
        assert!(patch.end_wire(out_b, &s).is_none(), "output → output");
        assert!(patch.graph().edges().is_empty());
    }

    #[test]
    fn core_validation_rejects_bad_wires() {
        let s = style();

        // Signal mismatch: audio out → control in.
        let mut patch = PatchView::new();
        let src = patch.add_node(gain(), Pos2::new(0.0, 0.0));
        let ctl = patch.add_node(
            GraphNode::new(
                "adsr",
                vec![Port::input("gate", SignalType::Control)],
                vec![Port::output("env", SignalType::Control)],
            ),
            Pos2::new(300.0, 0.0),
        );
        let out = patch
            .port_anchor(src, PortDirection::Output, PortId(0), &s)
            .unwrap();
        let gate = patch
            .port_anchor(ctl, PortDirection::Input, PortId(0), &s)
            .unwrap();
        patch.begin_wire(out, &s);
        assert!(patch.end_wire(gate, &s).is_none());
        assert!(patch.graph().edges().is_empty());

        // Occupied input: the second wire into the same port is rejected.
        let (mut patch, a, b) = two_gains();
        let c = patch.add_node(gain(), Pos2::new(0.0, 200.0));
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();
        let out_c = patch
            .port_anchor(c, PortDirection::Output, PortId(0), &s)
            .unwrap();
        let inp_b = patch
            .port_anchor(b, PortDirection::Input, PortId(0), &s)
            .unwrap();
        patch.begin_wire(out_c, &s);
        assert!(patch.end_wire(inp_b, &s).is_none());
        assert_eq!(patch.graph().edges().len(), 1, "original edge untouched");
    }

    #[test]
    fn port_anchor_matches_the_layout() {
        let (patch, a, _) = two_gains();
        let s = style();
        let layout = layout_node(patch.graph().node(a).unwrap(), Pos2::ZERO, &s);
        assert_eq!(
            patch.port_anchor(a, PortDirection::Output, PortId(0), &s),
            Some(layout.outputs[0])
        );
        assert_eq!(
            patch.port_anchor(a, PortDirection::Input, PortId(9), &s),
            None,
            "out-of-range port"
        );
    }

    #[test]
    fn positions_and_graph_stay_in_sync() {
        let mut patch = PatchView::new();
        let id = patch.add_node(gain(), Pos2::new(7.0, 9.0));
        assert_eq!(patch.position(id), Some(Pos2::new(7.0, 9.0)));
        assert_eq!(patch.graph().node_count(), 1);
        let drawn: Vec<_> = patch.draw_order().map(|(id, _, _)| id).collect();
        assert_eq!(drawn, vec![id]);
    }
}
