//! The complete, compile-tested example for the "add your own node" developer guide
//! (`docs/adding-a-node.md`, backlog 000088).
//!
//! It builds a third-party node entirely against the public API — implement
//! [`vb_core::Processor`], describe it with a [`vb_nodes::NodeDescriptor`], register it
//! into a [`vb_nodes::Registry`] — then runs a patch that uses it through the real engine.
//! The guide walks through these same pieces; keeping the example here means the guide's
//! code cannot drift from an API that still compiles and runs.

use std::collections::HashMap;

use vb_core::{
    Engine, Graph, GraphNode, Parameter, Port, PortId, ProcessContext, Processor, SignalType,
};
use vb_nodes::{NodeDescriptor, Registry};

// ---------------------------------------------------------------------------
// 1. The DSP: implement `Processor`.
// ---------------------------------------------------------------------------

/// A soft-clip "drive": `out = tanh(amount * in)`. One audio input, one audio output,
/// one `amount` parameter. `tanh` is inherently bounded to (-1, 1), so it saturates
/// smoothly as the drive rises.
struct Drive {
    amount: f32,
}

impl Processor for Drive {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        for (out, &x) in outputs[0].iter_mut().zip(input).take(ctx.frames) {
            *out = (self.amount * x).tanh();
        }
    }

    fn reset(&mut self) {
        // Stateless: nothing to clear between runs.
    }
}

// ---------------------------------------------------------------------------
// 2. The descriptor: how the registry builds the node, its ports, and its params.
//    The factories are plain `fn` pointers, so use free functions or *non-capturing*
//    closures (they coerce to `fn`).
// ---------------------------------------------------------------------------

/// Builds the [`NodeDescriptor`] for the `drive` kind.
fn drive_descriptor() -> NodeDescriptor {
    NodeDescriptor::new(
        "drive",
        "effect",
        // ports
        || {
            GraphNode::new(
                "drive",
                vec![Port::input("in", SignalType::Audio)],
                vec![Port::output("out", SignalType::Audio)],
            )
        },
        // default processor
        || Box::new(Drive { amount: 1.0 }),
        // editable parameters (name, min, max, default), in UI order
        || vec![Parameter::new("amount", 0.0, 10.0, 1.0)],
        // processor configured from parameter values (index matches `params()` order)
        |_ctx, values| {
            Box::new(Drive {
                amount: values.first().copied().unwrap_or(1.0),
            })
        },
    )
}

#[test]
fn a_custom_node_registers_and_runs_in_the_engine() {
    // 3. Register it alongside the built-ins.
    let mut registry = Registry::with_builtins();
    registry.register(drive_descriptor());

    // It is now a first-class kind: discoverable, with its ports and params.
    let desc = registry.get("drive").expect("drive is registered");
    assert_eq!(desc.category(), "effect");
    let node = registry.graph_node("drive").unwrap();
    assert_eq!(node.inputs.len(), 1);
    assert_eq!(node.outputs.len(), 1);
    assert_eq!(registry.get("drive").unwrap().params()[0].name, "amount");

    // 4. Use it in a patch: oscillator → drive, built entirely from the registry.
    let mut graph = Graph::new();
    let osc = graph.add_node(registry.graph_node("oscillator").unwrap());
    let drive = graph.add_node(registry.graph_node("drive").unwrap());
    graph.connect(osc, PortId(0), drive, PortId(0)).unwrap();

    let ctx = vb_nodes::BuildContext::default();
    let mut procs: HashMap<_, Box<dyn Processor>> = HashMap::new();
    procs.insert(osc, registry.processor("oscillator").unwrap());
    // A hot drive so the saturation is obvious.
    procs.insert(drive, desc.processor_with(&ctx, &[8.0]));

    let mut engine = Engine::compile(&graph, procs, 128).unwrap();
    engine.process(&ProcessContext {
        sample_rate: 48_000.0,
        frames: 128,
    });

    let out = engine.node_output(drive).unwrap().channel(0).unwrap();
    assert!(out.iter().all(|s| s.is_finite()), "output is finite");
    assert!(
        out.iter().all(|s| s.abs() <= 1.0),
        "tanh keeps the drive bounded to (-1, 1)"
    );
    assert!(
        out.iter().any(|&s| s.abs() > 0.5),
        "a hot drive on a full-scale oscillator saturates loudly"
    );
}

#[test]
fn the_default_processor_uses_the_declared_default_amount() {
    // The parameter default (1.0) is what `processor()` (no values) builds with — a gentle
    // drive that still passes signal without hard saturation on a modest input.
    let desc = drive_descriptor();
    let mut proc = desc.processor();

    let ctx = ProcessContext {
        sample_rate: 48_000.0,
        frames: 4,
    };
    let input = [0.0_f32, 0.5, -0.5, 1.0];
    let mut out = [0.0_f32; 4];
    {
        let ins: [&[f32]; 1] = [&input];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        proc.process(&ctx, &ins, &mut outs);
    }
    // out = tanh(1.0 * in): monotonic, bounded, tanh(0) == 0.
    assert!((out[0]).abs() < 1e-6);
    assert!((out[1] - 0.5_f32.tanh()).abs() < 1e-6);
    assert!((out[3] - 1.0_f32.tanh()).abs() < 1e-6);
}
