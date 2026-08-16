//! Building a runnable engine out of whatever is on the editor canvas.
//!
//! This is the editor→runtime half of task 000062: every audible edit in the
//! [`vb_editor::PatchView`] is compiled into a fresh allocation-free
//! [`RtEngine`] (processors configured from the patch's parameter values via the
//! registry) and handed to the audio thread through the lock-free engine swap.

use std::collections::HashMap;

use vb_core::{NodeId, Processor, SignalType};
use vb_editor::PatchView;
use vb_nodes::Registry;
use vb_runtime::RtEngine;

/// Compiles the patch into an engine plus the node to listen to.
///
/// Every node's processor is built from the patch's *current* parameter values
/// ([`vb_nodes::NodeDescriptor::processor_with`]). The sink is the topmost terminal
/// audio node — one with an audio output and no outgoing edge — or `None` (silence)
/// if the patch has no such node.
///
/// Returns `None` when the patch cannot be compiled: a node kind missing from the
/// registry, or an engine-side rejection (e.g. port fan-out limits). The caller then
/// keeps the previous engine running.
#[must_use]
pub fn build_live_engine(
    patch: &PatchView,
    registry: &Registry,
    frames: usize,
) -> Option<(RtEngine, Option<NodeId>)> {
    let graph = patch.graph();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    for id in graph.node_ids() {
        let node = graph.node(id)?;
        let descriptor = registry.get(&node.kind)?;
        let values: Vec<f32> = patch.params(id).iter().map(|s| s.value).collect();
        procs.insert(id, descriptor.processor_with(&values));
    }

    // Topmost terminal audio node wins: audio output, nothing downstream.
    let sink = patch
        .draw_order()
        .filter(|(id, node, _)| {
            node.outputs.iter().any(|p| p.signal == SignalType::Audio)
                && !graph.edges().iter().any(|e| e.from_node == *id)
        })
        .map(|(id, _, _)| id)
        .last();

    let engine = RtEngine::compile(graph, procs, frames).ok()?;
    Some((engine, sink))
}

#[cfg(test)]
mod tests {
    use egui::Pos2;
    use vb_core::{GraphNode, Port, PortId, ProcessContext};

    use super::*;

    /// osc → gain, both from the registry, with the given gain value.
    fn patch_with_gain(gain: f32) -> (PatchView, NodeId) {
        let registry = Registry::with_builtins();
        let mut patch = PatchView::new();
        let osc = patch.add_node_with_params(
            registry.graph_node("oscillator").unwrap(),
            registry.get("oscillator").unwrap().params(),
            Pos2::new(0.0, 0.0),
        );
        let amp = patch.add_node_with_params(
            registry.graph_node("gain").unwrap(),
            registry.get("gain").unwrap().params(),
            Pos2::new(300.0, 0.0),
        );
        patch.connect(osc, PortId(0), amp, PortId(0)).unwrap();
        patch.set_param(amp, 0, gain);
        (patch, amp)
    }

    fn peak(engine: &mut RtEngine, sink: NodeId) -> f32 {
        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        engine
            .node_output(sink)
            .unwrap()
            .channel(0)
            .unwrap()
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn the_canvas_patch_compiles_and_the_terminal_gain_is_the_sink() {
        let registry = Registry::with_builtins();
        let (patch, amp) = patch_with_gain(1.0);
        let (mut engine, sink) = build_live_engine(&patch, &registry, 256).expect("compiles");
        assert_eq!(sink, Some(amp), "terminal audio node is the sink");
        assert!(peak(&mut engine, amp) > 0.1, "the patch makes sound");
    }

    #[test]
    fn parameter_values_reach_the_processors() {
        let registry = Registry::with_builtins();
        let (loud_patch, amp) = patch_with_gain(1.0);
        let (quiet_patch, _) = patch_with_gain(0.25);

        let (mut loud, _) = build_live_engine(&loud_patch, &registry, 256).unwrap();
        let (mut quiet, _) = build_live_engine(&quiet_patch, &registry, 256).unwrap();

        let loud_peak = peak(&mut loud, amp);
        let quiet_peak = peak(&mut quiet, amp);
        assert!(
            (quiet_peak - loud_peak * 0.25).abs() < 1e-3,
            "gain param scales the output: {loud_peak} vs {quiet_peak}"
        );
    }

    #[test]
    fn unknown_kinds_refuse_to_build() {
        let registry = Registry::with_builtins();
        let mut patch = PatchView::new();
        patch.add_node(
            GraphNode::new(
                "mystery",
                vec![],
                vec![Port::output("out", SignalType::Audio)],
            ),
            Pos2::ZERO,
        );
        assert!(build_live_engine(&patch, &registry, 256).is_none());
    }

    #[test]
    fn a_patch_without_terminal_audio_builds_with_no_sink() {
        let registry = Registry::with_builtins();
        let mut patch = PatchView::new();
        // adsr's output is Control, not Audio: nothing to listen to.
        patch.add_node_with_params(
            registry.graph_node("adsr").unwrap(),
            registry.get("adsr").unwrap().params(),
            Pos2::ZERO,
        );
        let (_engine, sink) = build_live_engine(&patch, &registry, 256).expect("still compiles");
        assert_eq!(sink, None);
    }
}
