//! Node geometry: where a node's frame, title bar, and port anchors sit on the canvas.
//!
//! Pure math over [`seclab_core::GraphNode`] — no egui runtime involved — so every rect and
//! anchor the painter uses is unit-testable. All coordinates are *canvas* space; the
//! painter maps them to the screen through the camera.

use egui::{Pos2, Rect, Vec2};
use seclab_core::{GraphNode, PortId};

/// Visual constants for node layout, in canvas units.
#[derive(Debug, Clone, Copy)]
pub struct NodeStyle {
    /// Node frame width.
    pub width: f32,
    /// Title bar height.
    pub title_height: f32,
    /// Height of one port row.
    pub row_height: f32,
    /// Port circle radius.
    pub port_radius: f32,
}

impl Default for NodeStyle {
    fn default() -> Self {
        Self {
            width: 120.0,
            title_height: 22.0,
            row_height: 18.0,
            port_radius: 5.0,
        }
    }
}

/// Computed geometry for one node, in canvas coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeLayout {
    /// Whole node frame (title + body).
    pub frame: Rect,
    /// Title bar strip at the top of the frame.
    pub title: Rect,
    /// Input port anchors on the left edge, index-aligned with `GraphNode::inputs`.
    pub inputs: Vec<Pos2>,
    /// Output port anchors on the right edge, index-aligned with `GraphNode::outputs`.
    pub outputs: Vec<Pos2>,
}

/// What a canvas point lands on inside a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeHit {
    /// The title bar (drag handle / rename target).
    Title,
    /// The body below the title.
    Body,
    /// An input port anchor.
    Input(PortId),
    /// An output port anchor.
    Output(PortId),
}

/// Lays out `node` with its frame's top-left corner at `origin`.
#[must_use]
pub fn layout_node(node: &GraphNode, origin: Pos2, style: &NodeStyle) -> NodeLayout {
    let rows = node.inputs.len().max(node.outputs.len());
    #[allow(clippy::cast_precision_loss)]
    let body_height = rows as f32 * style.row_height;
    let frame = Rect::from_min_size(
        origin,
        Vec2::new(style.width, style.title_height + body_height),
    );
    let title = Rect::from_min_size(origin, Vec2::new(style.width, style.title_height));

    let row_center = |i: usize| {
        #[allow(clippy::cast_precision_loss)]
        let row = i as f32;
        frame.min.y + style.title_height + (row + 0.5) * style.row_height
    };
    let inputs = (0..node.inputs.len())
        .map(|i| Pos2::new(frame.min.x, row_center(i)))
        .collect();
    let outputs = (0..node.outputs.len())
        .map(|i| Pos2::new(frame.max.x, row_center(i)))
        .collect();

    NodeLayout {
        frame,
        title,
        inputs,
        outputs,
    }
}

/// Returns what `p` (canvas coords) lands on, or `None` if it misses the node.
///
/// Port anchors win over the frame so a click near the edge grabs the port, not the
/// body; the grab zone is slightly larger than the drawn circle.
#[must_use]
pub fn hit_test(layout: &NodeLayout, p: Pos2, style: &NodeStyle) -> Option<NodeHit> {
    let grab = style.port_radius * 1.6;
    for (i, anchor) in layout.inputs.iter().enumerate() {
        if (*anchor - p).length() <= grab {
            return Some(NodeHit::Input(PortId(i)));
        }
    }
    for (i, anchor) in layout.outputs.iter().enumerate() {
        if (*anchor - p).length() <= grab {
            return Some(NodeHit::Output(PortId(i)));
        }
    }
    if layout.title.contains(p) {
        return Some(NodeHit::Title);
    }
    if layout.frame.contains(p) {
        return Some(NodeHit::Body);
    }
    None
}

#[cfg(test)]
mod tests {
    use seclab_core::{Port, SignalType};

    use super::*;

    fn style() -> NodeStyle {
        NodeStyle::default()
    }

    fn biquad() -> GraphNode {
        GraphNode::new(
            "biquad",
            vec![
                Port::input("in", SignalType::Audio),
                Port::input("cutoff", SignalType::Control),
            ],
            vec![Port::output("out", SignalType::Audio)],
        )
    }

    #[test]
    fn title_spans_the_top_of_the_frame() {
        let l = layout_node(&biquad(), Pos2::new(10.0, 20.0), &style());
        assert_eq!(l.title.min, l.frame.min);
        assert_eq!(l.title.width(), l.frame.width());
        assert!((l.title.height() - style().title_height).abs() < f32::EPSILON);
    }

    #[test]
    fn height_grows_with_the_longer_port_column() {
        let s = style();
        let one_row = layout_node(
            &GraphNode::new("gain", vec![], vec![Port::output("out", SignalType::Audio)]),
            Pos2::ZERO,
            &s,
        );
        let two_rows = layout_node(&biquad(), Pos2::ZERO, &s);
        assert!((one_row.frame.height() - (s.title_height + s.row_height)).abs() < f32::EPSILON);
        assert!(
            (two_rows.frame.height() - (s.title_height + 2.0 * s.row_height)).abs() < f32::EPSILON
        );
    }

    #[test]
    fn inputs_sit_on_the_left_edge_and_outputs_on_the_right() {
        let l = layout_node(&biquad(), Pos2::new(50.0, 0.0), &style());
        assert_eq!(l.inputs.len(), 2);
        assert_eq!(l.outputs.len(), 1);
        for p in &l.inputs {
            assert!((p.x - l.frame.min.x).abs() < f32::EPSILON);
        }
        for p in &l.outputs {
            assert!((p.x - l.frame.max.x).abs() < f32::EPSILON);
        }
        // Anchors are ordered top-to-bottom and stay inside the body strip.
        assert!(l.inputs[0].y < l.inputs[1].y);
        for p in l.inputs.iter().chain(&l.outputs) {
            assert!(p.y > l.title.max.y && p.y < l.frame.max.y);
        }
    }

    #[test]
    fn source_nodes_without_inputs_still_lay_out() {
        let osc = GraphNode::new(
            "oscillator",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        );
        let l = layout_node(&osc, Pos2::ZERO, &style());
        assert!(l.inputs.is_empty());
        assert_eq!(l.outputs.len(), 1);
    }

    #[test]
    fn hit_test_distinguishes_title_body_ports_and_miss() {
        let s = style();
        let l = layout_node(&biquad(), Pos2::new(100.0, 100.0), &s);

        assert_eq!(hit_test(&l, l.title.center(), &s), Some(NodeHit::Title));
        assert_eq!(
            hit_test(&l, l.frame.center(), &s),
            Some(NodeHit::Body),
            "frame centre is body (ports hug the edges)"
        );
        assert_eq!(
            hit_test(&l, l.inputs[1], &s),
            Some(NodeHit::Input(PortId(1)))
        );
        assert_eq!(
            hit_test(&l, l.outputs[0], &s),
            Some(NodeHit::Output(PortId(0)))
        );
        assert_eq!(hit_test(&l, Pos2::new(0.0, 0.0), &s), None);
    }

    #[test]
    fn port_grab_zone_beats_the_body_near_the_edge() {
        let s = style();
        let l = layout_node(&biquad(), Pos2::ZERO, &s);
        // Just inside the frame, next to the first input anchor.
        let p = l.inputs[0] + Vec2::new(s.port_radius, 0.0);
        assert_eq!(hit_test(&l, p, &s), Some(NodeHit::Input(PortId(0))));
    }
}
