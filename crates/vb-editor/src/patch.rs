//! The patch as the editor sees it: the [`vb_core::Graph`] plus canvas positions,
//! draw order, and selection.
//!
//! `vb-core` stays the single source of truth for structure (nodes, ports, edges);
//! this view only adds what the canvas needs — where each node sits, which one is
//! selected, and who is on top.

use std::collections::HashMap;

use egui::{Pos2, Vec2};
use vb_core::{Edge, Graph, GraphNode, NodeId, Parameter, PortDirection, PortId};

use crate::layout::{hit_test, layout_node, NodeHit, NodeStyle};
use crate::widgets::{default_widget, WidgetKind};

/// Node kind of a macro (a nested subgraph shown as one node).
pub const MACRO_KIND: &str = "macro";
/// Kind of the inner node standing for the macro's inputs.
pub const MACRO_IN_KIND: &str = "macro-in";
/// Kind of the inner node collecting the macro's outputs.
pub const MACRO_OUT_KIND: &str = "macro-out";

fn audio_in(name: &str) -> vb_core::Port {
    vb_core::Port::input(name, vb_core::SignalType::Audio)
}

fn audio_out(name: &str) -> vb_core::Port {
    vb_core::Port::output(name, vb_core::SignalType::Audio)
}

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

/// Orients a drag endpoint pair as (output side, input side), or `None` when both
/// ends are the same direction.
fn normalize_wire(
    drag: WireDrag,
    target: (NodeId, NodeHit),
) -> Option<((NodeId, PortId), (NodeId, PortId))> {
    match (drag.direction, target) {
        (PortDirection::Output, (node, NodeHit::Input(port))) => {
            Some(((drag.node, drag.port), (node, port)))
        }
        (PortDirection::Input, (node, NodeHit::Output(port))) => {
            Some(((node, port), (drag.node, drag.port)))
        }
        _ => None,
    }
}

/// What kind of edit the last recorded history entry belongs to.
///
/// Consecutive edits of the same gesture (dragging one node, sliding one parameter)
/// coalesce into a single undo step; anything else records one step per action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gesture {
    /// Continuous node move; coalesces per node.
    Move(NodeId),
    /// Continuous parameter edit; coalesces per (node, param index).
    Param(NodeId, usize),
    /// A one-shot action (add, remove, duplicate, wire); never coalesces.
    Discrete,
}

/// Everything [`PatchView::undo`] needs to restore.
#[derive(Clone)]
struct Snapshot {
    graph: Graph,
    positions: HashMap<NodeId, Pos2>,
    order: Vec<NodeId>,
    selected: Option<NodeId>,
    params: HashMap<NodeId, Vec<ParamSlot>>,
}

/// Bounded undo/redo stacks plus the coalescing tag.
#[derive(Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The gesture the most recent undo entry was recorded for.
    last: Option<Gesture>,
}

/// Cap on stored undo steps; the oldest entries fall off beyond this.
const HISTORY_LIMIT: usize = 200;

/// One editable parameter on a node: its descriptor, current value, and the control
/// the inspector draws for it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSlot {
    /// Name, range and default (from the registry descriptor).
    pub param: Parameter,
    /// Current value, always inside the descriptor's range.
    pub value: f32,
    /// Which control the inspector renders (knob, slider, switch, XY-pad).
    pub widget: WidgetKind,
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
    /// Editable parameter values per node, in the descriptor's UI order.
    params: HashMap<NodeId, Vec<ParamSlot>>,
    /// Inner patches of macro nodes, keyed by the macro node's id.
    subpatches: HashMap<NodeId, PatchView>,
    /// Undo/redo state.
    history: History,
    /// Bumped on every audible mutation (structure, params, undo/redo) so embedders
    /// know when to rebuild the audio engine. Selection changes do not count.
    revision: u64,
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

    /// Captures the current state for the undo stack.
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            graph: self.graph.clone(),
            positions: self.positions.clone(),
            order: self.order.clone(),
            selected: self.selected,
            params: self.params.clone(),
        }
    }

    /// Restores a snapshot, returning the state it replaced.
    fn restore(&mut self, snap: Snapshot) -> Snapshot {
        let current = self.snapshot();
        self.graph = snap.graph;
        self.positions = snap.positions;
        self.order = snap.order;
        self.selected = snap.selected;
        self.params = snap.params;
        self.drag = None;
        current
    }

    /// Records an undo step for `gesture`, unless it coalesces with the previous one.
    /// Any new edit invalidates the redo stack.
    fn record(&mut self, gesture: Gesture) {
        self.history.redo.clear();
        if gesture != Gesture::Discrete && self.history.last == Some(gesture) {
            return;
        }
        let snap = self.snapshot();
        self.history.undo.push(snap);
        if self.history.undo.len() > HISTORY_LIMIT {
            self.history.undo.remove(0);
        }
        self.history.last = Some(gesture);
    }

    /// Marks the patch as audibly changed.
    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Monotonic counter of audible edits. Equal values mean "nothing to rebuild".
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Adds a macro node at `pos`: one audio in, one audio out, with a fresh inner
    /// patch pre-seeded with its `macro-in` / `macro-out` boundary nodes.
    ///
    /// Subpatch edits keep their own undo history; deleting the macro node drops the
    /// subpatch (not undoable — documented MVP limitation of nested histories).
    pub fn add_macro(&mut self, pos: Pos2) -> NodeId {
        let id = self.add_node(
            GraphNode::new(
                MACRO_KIND,
                vec![crate::patch::audio_in("in")],
                vec![crate::patch::audio_out("out")],
            ),
            pos,
        );
        let mut sub = PatchView::new();
        sub.add_node(
            GraphNode::new(MACRO_IN_KIND, vec![], vec![audio_out("in0")]),
            Pos2::new(0.0, 0.0),
        );
        sub.add_node(
            GraphNode::new(
                MACRO_OUT_KIND,
                vec![audio_in("out0")],
                vec![audio_out("out")],
            ),
            Pos2::new(360.0, 0.0),
        );
        sub.clear_history();
        self.subpatches.insert(id, sub);
        id
    }

    /// Whether `id` is a macro node with an inner patch.
    #[must_use]
    pub fn is_macro(&self, id: NodeId) -> bool {
        self.subpatches.contains_key(&id)
    }

    /// The inner patch of macro `id`.
    #[must_use]
    pub fn subpatch(&self, id: NodeId) -> Option<&PatchView> {
        self.subpatches.get(&id)
    }

    /// Removes the inner patch of macro `id` for editing; put it back with
    /// [`PatchView::restore_subpatch`]. The swap keeps every existing editing code
    /// path working on a plain `PatchView`.
    pub fn take_subpatch(&mut self, id: NodeId) -> Option<PatchView> {
        self.subpatches.remove(&id)
    }

    /// Returns an edited inner patch to macro `id` and marks the change audible.
    pub fn restore_subpatch(&mut self, id: NodeId, sub: PatchView) {
        self.subpatches.insert(id, sub);
        self.touch();
    }

    /// Ends the current continuous gesture (mouse released): the next move or
    /// parameter edit starts a fresh undo step.
    pub fn end_gesture(&mut self) {
        self.history.last = None;
    }

    /// Forgets all history — call after programmatic seeding so the user cannot undo
    /// into an empty patch.
    pub fn clear_history(&mut self) {
        self.history = History::default();
    }

    /// Whether there is anything to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    /// Whether there is anything to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    /// Reverts the most recent edit. Returns `false` with an empty undo stack.
    pub fn undo(&mut self) -> bool {
        match self.history.undo.pop() {
            Some(snap) => {
                let current = self.restore(snap);
                self.history.redo.push(current);
                self.history.last = None;
                self.touch();
                true
            }
            None => false,
        }
    }

    /// Re-applies the most recently undone edit. Returns `false` if there is none.
    pub fn redo(&mut self) -> bool {
        match self.history.redo.pop() {
            Some(snap) => {
                let current = self.restore(snap);
                self.history.undo.push(current);
                self.history.last = None;
                self.touch();
                true
            }
            None => false,
        }
    }

    /// Adds `node` to the graph at canvas position `pos` (top of the draw order),
    /// with no editable parameters.
    pub fn add_node(&mut self, node: GraphNode, pos: Pos2) -> NodeId {
        self.add_node_with_params(node, Vec::new(), pos)
    }

    /// Adds `node` with its editable parameters, each starting at its default.
    pub fn add_node_with_params(
        &mut self,
        node: GraphNode,
        params: Vec<Parameter>,
        pos: Pos2,
    ) -> NodeId {
        self.record(Gesture::Discrete);
        self.touch();
        let id = self.graph.add_node(node);
        self.positions.insert(id, pos);
        self.order.push(id);
        let slots = params
            .into_iter()
            .map(|param| ParamSlot {
                value: param.default,
                widget: default_widget(&param),
                param,
            })
            .collect();
        self.params.insert(id, slots);
        id
    }

    /// The node's editable parameters with their current values.
    #[must_use]
    pub fn params(&self, id: NodeId) -> &[ParamSlot] {
        self.params.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Sets parameter `index` on `id`, clamped into the parameter's range.
    ///
    /// Returns the stored (clamped) value, or `None` for an unknown node/index.
    pub fn set_param(&mut self, id: NodeId, index: usize, value: f32) -> Option<f32> {
        if !self.params.get(&id).is_some_and(|s| index < s.len()) {
            return None;
        }
        self.record(Gesture::Param(id, index));
        self.touch();
        let slot = self.params.get_mut(&id)?.get_mut(index)?;
        slot.value = slot.param.clamp(value);
        Some(slot.value)
    }

    /// Reassigns the control widget for parameter `index` on `id`.
    ///
    /// A widget is a view-only mapping — it changes nothing audible and is not an undo
    /// step — so this neither [`touch`](Self::touch)es the revision nor records history.
    /// Returns `false` for an unknown node/index.
    pub fn set_widget(&mut self, id: NodeId, index: usize, widget: WidgetKind) -> bool {
        match self.params.get_mut(&id).and_then(|s| s.get_mut(index)) {
            Some(slot) => {
                slot.widget = widget;
                true
            }
            None => false,
        }
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
        self.graph.can_connect(from, from_port, to, to_port)?;
        self.record(Gesture::Discrete);
        self.touch();
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
    /// direction and [`Graph::can_connect`] accepts the edge (signal types match,
    /// input free, no cycle). Returns the created edge; any miss or rejection just
    /// clears the drag.
    pub fn end_wire(&mut self, p: Pos2, style: &NodeStyle) -> Option<Edge> {
        let drag = self.drag.take()?;
        let target = self.node_at(p, style)?;
        let (from, to) = normalize_wire(drag, target)?;
        self.graph.can_connect(from.0, from.1, to.0, to.1).ok()?;
        self.record(Gesture::Discrete);
        self.touch();
        self.graph.connect(from.0, from.1, to.0, to.1).ok()
    }

    /// Whether dropping the active wire on the given port would connect.
    ///
    /// `None` when no wire drag is active (nothing to highlight); `Some(false)` for
    /// same-direction ports and edges [`Graph::can_connect`] rejects (signal mismatch,
    /// occupied input, cycle); `Some(true)` for a valid drop target.
    #[must_use]
    pub fn wire_target_ok(
        &self,
        node: NodeId,
        direction: PortDirection,
        port: PortId,
    ) -> Option<bool> {
        let drag = self.drag?;
        let hit = match direction {
            PortDirection::Input => NodeHit::Input(port),
            PortDirection::Output => NodeHit::Output(port),
        };
        let Some((from, to)) = normalize_wire(drag, (node, hit)) else {
            return Some(false);
        };
        Some(self.graph.can_connect(from.0, from.1, to.0, to.1).is_ok())
    }

    /// Snaps a node's position to the nearest multiple of `step` (canvas units).
    /// Not recorded as its own undo step: callers snap at the end of a move gesture,
    /// so the whole drag-plus-snap coalesces into one entry.
    pub fn snap_node(&mut self, id: NodeId, step: f32) -> bool {
        if step <= 0.0 {
            return false;
        }
        match self.positions.get_mut(&id) {
            Some(pos) => {
                pos.x = (pos.x / step).round() * step;
                pos.y = (pos.y / step).round() * step;
                true
            }
            None => false,
        }
    }

    /// Moves a node by a canvas-space delta. Returns `false` for unknown nodes.
    pub fn move_node(&mut self, id: NodeId, delta: Vec2) -> bool {
        if !self.positions.contains_key(&id) {
            return false;
        }
        self.record(Gesture::Move(id));
        if let Some(pos) = self.positions.get_mut(&id) {
            *pos += delta;
        }
        true
    }

    /// Removes a node and everything attached to it: incident edges (via
    /// [`Graph::remove_node`]), position, draw-order entry, params, and the selection
    /// if it pointed here. Returns `false` for unknown nodes.
    pub fn remove_node(&mut self, id: NodeId) -> bool {
        if self.graph.node(id).is_none() {
            return false;
        }
        self.record(Gesture::Discrete);
        self.touch();
        if self.graph.remove_node(id).is_none() {
            return false;
        }
        self.positions.remove(&id);
        self.order.retain(|n| *n != id);
        self.params.remove(&id);
        self.subpatches.remove(&id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        true
    }

    /// Removes the selected node, returning its id.
    pub fn remove_selected(&mut self) -> Option<NodeId> {
        let id = self.selected?;
        self.remove_node(id).then_some(id)
    }

    /// Duplicates a node: same kind and ports, current (not default) parameter
    /// values, offset position, no edges. The copy becomes the selection.
    pub fn duplicate(&mut self, id: NodeId) -> Option<NodeId> {
        let node = self.graph.node(id)?.clone();
        let pos = *self.positions.get(&id)? + Vec2::new(24.0, 24.0);
        self.record(Gesture::Discrete);
        self.touch();
        let slots = self.params.get(&id).cloned().unwrap_or_default();
        let copy = self.graph.add_node(node);
        self.positions.insert(copy, pos);
        self.order.push(copy);
        self.params.insert(copy, slots);
        self.selected = Some(copy);
        Some(copy)
    }

    /// Duplicates the selected node, selecting and returning the copy.
    pub fn duplicate_selected(&mut self) -> Option<NodeId> {
        self.duplicate(self.selected?)
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
    fn end_wire_refuses_edges_that_would_close_a_cycle() {
        let (mut patch, a, b) = two_gains();
        let s = style();
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();

        // b.out → a.in would close a → b → a.
        let out_b = patch
            .port_anchor(b, PortDirection::Output, PortId(0), &s)
            .unwrap();
        let in_a = patch
            .port_anchor(a, PortDirection::Input, PortId(0), &s)
            .unwrap();
        patch.begin_wire(out_b, &s);
        assert!(patch.end_wire(in_a, &s).is_none());
        assert_eq!(patch.graph().edges().len(), 1, "cycle edge not created");
    }

    #[test]
    fn wire_target_ok_highlights_only_valid_drops() {
        let (mut patch, a, b) = two_gains();
        let c = patch.add_node(gain(), Pos2::new(600.0, 0.0));
        let s = style();
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();

        // No drag: nothing to highlight.
        assert_eq!(
            patch.wire_target_ok(c, PortDirection::Input, PortId(0)),
            None
        );

        // Dragging from b.out: c.in is valid; a.in closes a cycle; b.in is our own
        // occupied input anyway; any output is same-direction.
        let out_b = patch
            .port_anchor(b, PortDirection::Output, PortId(0), &s)
            .unwrap();
        patch.begin_wire(out_b, &s);
        assert_eq!(
            patch.wire_target_ok(c, PortDirection::Input, PortId(0)),
            Some(true)
        );
        assert_eq!(
            patch.wire_target_ok(a, PortDirection::Input, PortId(0)),
            Some(false),
            "would close a cycle"
        );
        assert_eq!(
            patch.wire_target_ok(b, PortDirection::Input, PortId(0)),
            Some(false),
            "occupied input"
        );
        assert_eq!(
            patch.wire_target_ok(a, PortDirection::Output, PortId(0)),
            Some(false),
            "same direction"
        );

        // Dragging from an input highlights free outputs instead.
        patch.end_wire(Pos2::new(-999.0, -999.0), &s);
        let in_c = patch
            .port_anchor(c, PortDirection::Input, PortId(0), &s)
            .unwrap();
        patch.begin_wire(in_c, &s);
        assert_eq!(
            patch.wire_target_ok(b, PortDirection::Output, PortId(0)),
            Some(true)
        );
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
    fn params_seed_at_defaults_and_clamp_on_set() {
        let mut patch = PatchView::new();
        let id = patch.add_node_with_params(
            gain(),
            vec![
                Parameter::new("gain", 0.0, 2.0, 1.0),
                Parameter::new("pan", -1.0, 1.0, 0.0),
            ],
            Pos2::ZERO,
        );

        let slots = patch.params(id);
        assert_eq!(slots.len(), 2);
        assert!(
            (slots[0].value - 1.0).abs() < f32::EPSILON,
            "starts at default"
        );

        assert_eq!(patch.set_param(id, 0, 5.0), Some(2.0), "clamped to max");
        assert_eq!(patch.set_param(id, 1, -0.25), Some(-0.25));
        assert!((patch.params(id)[0].value - 2.0).abs() < f32::EPSILON);

        // Unknown index / node are refused without panicking.
        assert_eq!(patch.set_param(id, 9, 1.0), None);
        assert_eq!(patch.set_param(vb_core::NodeId(999), 0, 1.0), None);
    }

    #[test]
    fn params_default_to_a_knob_and_the_widget_can_be_retyped() {
        let mut patch = PatchView::new();
        let id = patch.add_node_with_params(
            gain(),
            vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            Pos2::ZERO,
        );
        assert_eq!(
            patch.params(id)[0].widget,
            WidgetKind::Knob,
            "default widget"
        );

        let r0 = patch.revision();
        assert!(patch.set_widget(id, 0, WidgetKind::Switch));
        assert_eq!(patch.params(id)[0].widget, WidgetKind::Switch);
        assert_eq!(
            patch.revision(),
            r0,
            "a widget is a view mapping — not an audible edit"
        );

        // Unknown node/index are refused.
        assert!(!patch.set_widget(id, 9, WidgetKind::Slider));
        assert!(!patch.set_widget(NodeId(999), 0, WidgetKind::Slider));
    }

    #[test]
    fn duplicate_carries_the_widget_assignment() {
        let mut patch = PatchView::new();
        let id = patch.add_node_with_params(
            gain(),
            vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            Pos2::ZERO,
        );
        patch.set_widget(id, 0, WidgetKind::Slider);
        patch.click(Pos2::new(10.0, 10.0), &style());
        let copy = patch.duplicate_selected().expect("copy");
        assert_eq!(patch.params(copy)[0].widget, WidgetKind::Slider);
    }

    #[test]
    fn plain_add_node_has_no_params() {
        let mut patch = PatchView::new();
        let id = patch.add_node(gain(), Pos2::ZERO);
        assert!(patch.params(id).is_empty());
    }

    #[test]
    fn snap_rounds_to_the_grid() {
        let (mut patch, a, _) = two_gains();
        patch.move_node(a, Vec2::new(37.0, -22.0));
        assert!(patch.snap_node(a, 16.0));
        assert_eq!(patch.position(a), Some(Pos2::new(32.0, -16.0)));
        // Zero/negative steps and unknown nodes are refused.
        assert!(!patch.snap_node(a, 0.0));
        assert!(!patch.snap_node(NodeId(999), 16.0));
    }

    #[test]
    fn move_node_shifts_its_position() {
        let (mut patch, a, _) = two_gains();
        assert!(patch.move_node(a, Vec2::new(15.0, -5.0)));
        assert_eq!(patch.position(a), Some(Pos2::new(15.0, -5.0)));
        assert!(!patch.move_node(NodeId(999), Vec2::ZERO));
    }

    #[test]
    fn remove_selected_cleans_node_edges_params_and_selection() {
        let mut patch = PatchView::new();
        let a = patch.add_node(gain(), Pos2::new(0.0, 0.0));
        let b = patch.add_node_with_params(
            gain(),
            vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            Pos2::new(300.0, 0.0),
        );
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();
        patch.click(Pos2::new(310.0, 5.0), &style()); // select b
        assert_eq!(patch.selected(), Some(b));

        assert_eq!(patch.remove_selected(), Some(b));
        assert_eq!(patch.selected(), None);
        assert!(patch.graph().node(b).is_none());
        assert!(patch.graph().edges().is_empty(), "incident edge removed");
        assert!(patch.params(b).is_empty());
        assert!(patch.position(b).is_none());
        assert!(patch.draw_order().all(|(id, _, _)| id != b));

        // Nothing selected now: another remove is a no-op.
        assert_eq!(patch.remove_selected(), None);
    }

    #[test]
    fn duplicate_clones_params_by_value_but_not_edges() {
        let mut patch = PatchView::new();
        let a = patch.add_node(gain(), Pos2::new(0.0, 0.0));
        let b = patch.add_node_with_params(
            gain(),
            vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            Pos2::new(300.0, 0.0),
        );
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();
        patch.set_param(b, 0, 1.7);
        patch.click(Pos2::new(310.0, 5.0), &style()); // select b

        let copy = patch.duplicate_selected().expect("copy created");
        assert_ne!(copy, b);
        assert_eq!(patch.selected(), Some(copy), "the copy takes the selection");
        assert_eq!(patch.graph().node(copy).unwrap().kind, "gain");
        assert!(
            (patch.params(copy)[0].value - 1.7).abs() < f32::EPSILON,
            "current value survives, not the default"
        );
        assert_eq!(
            patch.position(copy),
            Some(Pos2::new(324.0, 24.0)),
            "offset from the original"
        );
        assert_eq!(patch.graph().edges().len(), 1, "edges are not cloned");
    }

    #[test]
    fn undo_and_redo_walk_add_and_remove() {
        let mut patch = PatchView::new();
        assert!(!patch.can_undo() && !patch.can_redo());
        assert!(!patch.undo(), "empty stack refuses");

        let a = patch.add_node(gain(), Pos2::new(0.0, 0.0));
        let b = patch.add_node_with_params(
            gain(),
            vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            Pos2::new(300.0, 0.0),
        );
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();
        assert_eq!(patch.graph().node_count(), 2);

        assert!(patch.undo(), "undo the connect");
        assert!(patch.graph().edges().is_empty());
        assert!(patch.undo(), "undo adding b");
        assert_eq!(patch.graph().node_count(), 1);

        assert!(patch.redo(), "redo adding b");
        assert_eq!(patch.graph().node_count(), 2);
        assert!(patch.params(b).len() == 1, "params come back too");
        assert!(patch.redo(), "redo the connect");
        assert_eq!(patch.graph().edges().len(), 1);
        assert!(!patch.redo(), "nothing further");
    }

    #[test]
    fn undoing_a_remove_restores_edges_and_params() {
        let mut patch = PatchView::new();
        let a = patch.add_node(gain(), Pos2::new(0.0, 0.0));
        let b = patch.add_node_with_params(
            gain(),
            vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            Pos2::new(300.0, 0.0),
        );
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();
        patch.set_param(b, 0, 1.5);

        assert!(patch.remove_node(b));
        assert!(patch.graph().edges().is_empty());

        assert!(patch.undo());
        assert!(patch.graph().node(b).is_some());
        assert_eq!(patch.graph().edges().len(), 1);
        assert!((patch.params(b)[0].value - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn continuous_gestures_coalesce_until_ended() {
        let (mut patch, a, _) = two_gains();
        patch.clear_history();

        // One drag = many move_node calls = one undo step.
        patch.move_node(a, Vec2::new(5.0, 0.0));
        patch.move_node(a, Vec2::new(5.0, 0.0));
        patch.move_node(a, Vec2::new(5.0, 0.0));
        patch.end_gesture();
        // A second drag is its own step.
        patch.move_node(a, Vec2::new(0.0, 10.0));
        patch.end_gesture();

        assert_eq!(patch.position(a), Some(Pos2::new(15.0, 10.0)));
        assert!(patch.undo());
        assert_eq!(
            patch.position(a),
            Some(Pos2::new(15.0, 0.0)),
            "second drag undone"
        );
        assert!(patch.undo());
        assert_eq!(
            patch.position(a),
            Some(Pos2::new(0.0, 0.0)),
            "first drag undone"
        );
        assert!(!patch.can_undo(), "exactly two steps were recorded");
    }

    #[test]
    fn param_slides_coalesce_per_param() {
        let mut patch = PatchView::new();
        let id = patch.add_node_with_params(
            gain(),
            vec![
                Parameter::new("gain", 0.0, 2.0, 1.0),
                Parameter::new("pan", -1.0, 1.0, 0.0),
            ],
            Pos2::ZERO,
        );
        patch.clear_history();

        patch.set_param(id, 0, 1.2);
        patch.set_param(id, 0, 1.4); // same slider, same gesture
        patch.set_param(id, 1, 0.5); // different param → new step
        patch.end_gesture();

        assert!(patch.undo());
        assert!((patch.params(id)[1].value - 0.0).abs() < f32::EPSILON);
        assert!((patch.params(id)[0].value - 1.4).abs() < f32::EPSILON);
        assert!(patch.undo());
        assert!((patch.params(id)[0].value - 1.0).abs() < f32::EPSILON);
        assert!(!patch.can_undo());
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let mut patch = PatchView::new();
        patch.add_node(gain(), Pos2::ZERO);
        patch.add_node(gain(), Pos2::new(300.0, 0.0));
        patch.undo();
        assert!(patch.can_redo());

        patch.add_node(gain(), Pos2::new(600.0, 0.0));
        assert!(!patch.can_redo(), "diverging edit invalidates redo");
    }

    #[test]
    fn revision_tracks_audible_edits_only() {
        let mut patch = PatchView::new();
        let r0 = patch.revision();

        let a = patch.add_node(gain(), Pos2::ZERO);
        let b = patch.add_node_with_params(
            gain(),
            vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            Pos2::new(300.0, 0.0),
        );
        assert!(patch.revision() > r0, "adding nodes is audible");

        let r1 = patch.revision();
        patch.connect(a, PortId(0), b, PortId(0)).unwrap();
        assert!(patch.revision() > r1, "wiring is audible");

        let r2 = patch.revision();
        patch.click(Pos2::new(5.0, 5.0), &style());
        patch.move_node(a, Vec2::new(50.0, 0.0));
        assert_eq!(patch.revision(), r2, "selection and moves are not audible");

        let r3 = patch.revision();
        patch.set_param(b, 0, 0.5);
        assert!(patch.revision() > r3, "param edits are audible");

        let r4 = patch.revision();
        assert!(patch.undo());
        assert!(patch.revision() > r4, "undo changes what sounds");
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
