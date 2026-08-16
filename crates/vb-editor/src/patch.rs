//! The patch as the editor sees it: the [`vb_core::Graph`] plus canvas positions,
//! draw order, and selection.
//!
//! `vb-core` stays the single source of truth for structure (nodes, ports, edges);
//! this view only adds what the canvas needs — where each node sits, which one is
//! selected, and who is on top.

use std::collections::HashMap;

use egui::Pos2;
use vb_core::{Graph, GraphNode, NodeId};

use crate::layout::{hit_test, layout_node, NodeHit, NodeStyle};

/// Editor-side view of a patch: graph + positions + selection.
#[derive(Default)]
pub struct PatchView {
    graph: Graph,
    positions: HashMap<NodeId, Pos2>,
    /// Draw order, back to front. Hit-testing walks it front to back.
    order: Vec<NodeId>,
    selected: Option<NodeId>,
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
