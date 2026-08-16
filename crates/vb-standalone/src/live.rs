//! Building a runnable engine out of whatever is on the editor canvas.
//!
//! This is the editor→runtime half of task 000062: every audible edit in the
//! [`vb_editor::PatchView`] is compiled into a fresh allocation-free
//! [`RtEngine`] (processors configured from the patch's parameter values via the
//! registry) and handed to the audio thread through the lock-free engine swap.

use std::collections::HashMap;

use vb_core::{MacroProcessor, MacroSpec, NodeId, PortId, ProcessContext, Processor, SignalType};
use vb_editor::patch::{MACRO_IN_KIND, MACRO_KIND, MACRO_OUT_KIND};
use vb_editor::PatchView;
use vb_nodes::{BuildContext, Registry};
use vb_runtime::RtEngine;

/// Emits silence — stands in for a macro's `macro-in` boundary when the subpatch is
/// auditioned on its own (no outer signal arrives).
struct Silence;
impl Processor for Silence {
    fn process(&mut self, ctx: &ProcessContext, _i: &[&[f32]], out: &mut [&mut [f32]]) {
        for o in out.iter_mut() {
            o[..ctx.frames].fill(0.0);
        }
    }
}

/// Copies input to output — the `macro-out` boundary.
struct Passthrough;
impl Processor for Passthrough {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], out: &mut [&mut [f32]]) {
        match inputs.first() {
            Some(input) => out[0][..ctx.frames].copy_from_slice(&input[..ctx.frames]),
            None => out[0][..ctx.frames].fill(0.0),
        }
    }
}

/// Builds the processor for one node, recursing into macro subpatches.
fn processor_for(
    patch: &PatchView,
    id: NodeId,
    kind: &str,
    registry: &Registry,
    ctx: &BuildContext,
    frames: usize,
) -> Option<Box<dyn Processor>> {
    match kind {
        MACRO_KIND => {
            let sub = patch.subpatch(id)?;
            let graph = sub.graph().clone();
            let input_node = sub
                .draw_order()
                .find(|(_, n, _)| n.kind == MACRO_IN_KIND)
                .map(|(id, _, _)| id)?;
            let output_node = sub
                .draw_order()
                .find(|(_, n, _)| n.kind == MACRO_OUT_KIND)
                .map(|(id, _, _)| id)?;
            let mut procs = std::collections::HashMap::new();
            for (sub_id, node, _) in sub.draw_order() {
                if sub_id == input_node {
                    continue; // MacroProcessor::compile feeds this one itself
                }
                procs.insert(
                    sub_id,
                    processor_for(sub, sub_id, &node.kind, registry, ctx, frames)?,
                );
            }
            let spec = MacroSpec {
                graph,
                input_node,
                outputs: vec![(output_node, PortId(0))],
            };
            MacroProcessor::compile(spec, procs, frames)
                .ok()
                .map(|p| Box::new(p) as Box<dyn Processor>)
        }
        MACRO_IN_KIND => Some(Box::new(Silence)),
        MACRO_OUT_KIND => Some(Box::new(Passthrough)),
        _ => {
            let descriptor = registry.get(kind)?;
            let values: Vec<f32> = patch.params(id).iter().map(|s| s.value).collect();
            Some(descriptor.processor_with(ctx, &values))
        }
    }
}

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
    ctx: &BuildContext,
    frames: usize,
) -> Option<(RtEngine, Option<NodeId>)> {
    let graph = patch.graph();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    for id in graph.node_ids() {
        let node = graph.node(id)?;
        procs.insert(
            id,
            processor_for(patch, id, &node.kind, registry, ctx, frames)?,
        );
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
        let (mut engine, sink) =
            build_live_engine(&patch, &registry, &BuildContext::default(), 256).expect("compiles");
        assert_eq!(sink, Some(amp), "terminal audio node is the sink");
        assert!(peak(&mut engine, amp) > 0.1, "the patch makes sound");
    }

    #[test]
    fn parameter_values_reach_the_processors() {
        let registry = Registry::with_builtins();
        let (loud_patch, amp) = patch_with_gain(1.0);
        let (quiet_patch, _) = patch_with_gain(0.25);

        let (mut loud, _) =
            build_live_engine(&loud_patch, &registry, &BuildContext::default(), 256).unwrap();
        let (mut quiet, _) =
            build_live_engine(&quiet_patch, &registry, &BuildContext::default(), 256).unwrap();

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
        assert!(build_live_engine(&patch, &registry, &BuildContext::default(), 256).is_none());
    }

    #[test]
    fn a_macro_node_compiles_and_passes_audio() {
        let registry = Registry::with_builtins();
        let mut patch = PatchView::new();
        let osc = patch.add_node_with_params(
            registry.graph_node("oscillator").unwrap(),
            registry.get("oscillator").unwrap().params(),
            Pos2::new(0.0, 0.0),
        );
        let mac = patch.add_macro(Pos2::new(200.0, 0.0));
        patch.connect(osc, PortId(0), mac, PortId(0)).unwrap();

        // Inside the macro: macro-in → gain → macro-out.
        {
            let mut sub = patch.take_subpatch(mac).unwrap();
            let (min, mout) = {
                let mut ids = (None, None);
                for (id, n, _) in sub.draw_order() {
                    match n.kind.as_str() {
                        vb_editor::patch::MACRO_IN_KIND => ids.0 = Some(id),
                        vb_editor::patch::MACRO_OUT_KIND => ids.1 = Some(id),
                        _ => {}
                    }
                }
                (ids.0.unwrap(), ids.1.unwrap())
            };
            let amp = sub.add_node_with_params(
                registry.graph_node("gain").unwrap(),
                registry.get("gain").unwrap().params(),
                Pos2::new(180.0, 0.0),
            );
            sub.connect(min, PortId(0), amp, PortId(0)).unwrap();
            sub.connect(amp, PortId(0), mout, PortId(0)).unwrap();
            patch.restore_subpatch(mac, sub);
        }

        let (mut engine, sink) =
            build_live_engine(&patch, &registry, &BuildContext::default(), 256).expect("compiles");
        assert_eq!(sink, Some(mac), "the macro is the terminal audio node");
        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let out = engine.node_output(mac).unwrap().channel(0).unwrap();
        assert!(
            out.iter().any(|&s| s.abs() > 0.01),
            "audio flows through the macro boundary"
        );
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
        let (_engine, sink) = build_live_engine(&patch, &registry, &BuildContext::default(), 256)
            .expect("still compiles");
        assert_eq!(sink, None);
    }
}
