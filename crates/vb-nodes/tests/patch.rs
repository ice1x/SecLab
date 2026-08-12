//! Integration: assemble real vb-nodes processors into a vb-core graph and run it.
//!
//! This is the "audible patch" smoke test — it proves the concrete DSP nodes actually
//! execute inside the engine and route signal between each other.

use std::collections::HashMap;

use vb_core::{Engine, Graph, GraphNode, Port, PortId, ProcessContext, Processor, SignalType};
use vb_nodes::{Adsr, Biquad, FilterKind, Gain, Mul, Oscillator, Waveform};

fn audio_out(name: &str) -> Vec<Port> {
    vec![Port::output(name, SignalType::Audio)]
}

#[test]
fn saw_through_lowpass_is_audible() {
    let mut g = Graph::new();
    let osc = g.add_node(GraphNode::new("osc", vec![], audio_out("out")));
    let flt = g.add_node(GraphNode::new(
        "filter",
        vec![Port::input("in", SignalType::Audio)],
        audio_out("out"),
    ));
    g.connect(osc, PortId(0), flt, PortId(0)).unwrap();

    let mut procs: HashMap<_, Box<dyn Processor>> = HashMap::new();
    procs.insert(osc, Box::new(Oscillator::new(Waveform::Saw, 220.0)));
    procs.insert(
        flt,
        Box::new(Biquad::new(FilterKind::LowPass, 2_000.0, 0.707)),
    );

    let mut engine = Engine::compile(&g, procs, 512).unwrap();
    engine.process(&ProcessContext {
        sample_rate: 48_000.0,
        frames: 512,
    });

    let out = engine.node_output(flt).unwrap().channel(0).unwrap();
    assert!(out.iter().all(|s| s.is_finite()));
    assert!(
        out.iter().any(|&s| s.abs() > 0.01),
        "a filtered saw should carry signal, not silence"
    );
}

#[test]
fn envelope_gates_an_oscillator_via_vca() {
    // osc -> Mul.in0, env(gate) -> Mul.in1  (Mul acts as a VCA).
    let mut g = Graph::new();
    let osc = g.add_node(GraphNode::new("osc", vec![], audio_out("out")));
    let env = g.add_node(GraphNode::new(
        "env",
        vec![Port::input("gate", SignalType::Control)],
        vec![Port::output("level", SignalType::Control)],
    ));
    let gate = g.add_node(GraphNode::new(
        "gate",
        vec![],
        vec![Port::output("g", SignalType::Control)],
    ));
    let vca = g.add_node(GraphNode::new(
        "vca",
        vec![
            Port::input("a", SignalType::Audio),
            Port::input("b", SignalType::Control),
        ],
        audio_out("out"),
    ));
    g.connect(gate, PortId(0), env, PortId(0)).unwrap();
    g.connect(osc, PortId(0), vca, PortId(0)).unwrap();
    g.connect(env, PortId(0), vca, PortId(1)).unwrap();

    // A processor that holds a constant gate high.
    struct GateHigh;
    impl Processor for GateHigh {
        fn process(&mut self, ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0].iter_mut().take(ctx.frames).for_each(|o| *o = 1.0);
        }
    }

    let mut procs: HashMap<_, Box<dyn Processor>> = HashMap::new();
    procs.insert(osc, Box::new(Oscillator::new(Waveform::Sine, 440.0)));
    procs.insert(env, Box::new(Adsr::new(0.001, 0.001, 0.8, 0.01)));
    procs.insert(gate, Box::new(GateHigh));
    procs.insert(vca, Box::new(Mul));

    let mut engine = Engine::compile(&g, procs, 256).unwrap();
    // Two blocks so the fast attack settles into sustain.
    let ctx = ProcessContext {
        sample_rate: 48_000.0,
        frames: 256,
    };
    engine.process(&ctx);
    engine.process(&ctx);

    let out = engine.node_output(vca).unwrap().channel(0).unwrap();
    assert!(out.iter().all(|s| s.is_finite()));
    assert!(
        out.iter().any(|&s| s.abs() > 0.1),
        "gated sine should be audible while the gate is high"
    );
}

#[test]
fn gain_scales_a_patch_output() {
    let mut g = Graph::new();
    let osc = g.add_node(GraphNode::new("osc", vec![], audio_out("out")));
    let amp = g.add_node(GraphNode::new(
        "amp",
        vec![Port::input("in", SignalType::Audio)],
        audio_out("out"),
    ));
    g.connect(osc, PortId(0), amp, PortId(0)).unwrap();

    let mut procs: HashMap<_, Box<dyn Processor>> = HashMap::new();
    procs.insert(osc, Box::new(Oscillator::new(Waveform::Sine, 100.0)));
    procs.insert(amp, Box::new(Gain::new(0.0)));

    let mut engine = Engine::compile(&g, procs, 64).unwrap();
    engine.process(&ProcessContext {
        sample_rate: 48_000.0,
        frames: 64,
    });

    // Gain of 0 mutes the patch.
    let out = engine.node_output(amp).unwrap().channel(0).unwrap();
    assert!(out.iter().all(|&s| s == 0.0));
}
