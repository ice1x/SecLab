//! Integration: the RT engine runs a graph correctly, and agrees with the reference
//! evaluator in `vb-core`.

use std::collections::HashMap;

use vb_core::{
    Engine, Graph, GraphNode, NodeId, Port, PortId, ProcessContext, Processor, SignalType,
};
use vb_nodes::{Biquad, FilterKind, Gain, Oscillator, Waveform};
use vb_runtime::{RtEngine, RtEngineError, MAX_PORTS_PER_NODE};

/// Emits a constant on its single output.
struct Constant(f32);
impl Processor for Constant {
    fn process(&mut self, _ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
        out[0].fill(self.0);
    }
}

/// Scales its single input.
struct Scale(f32);
impl Processor for Scale {
    fn process(&mut self, ctx: &ProcessContext, inp: &[&[f32]], out: &mut [&mut [f32]]) {
        for (o, x) in out[0].iter_mut().zip(inp[0].iter()).take(ctx.frames) {
            *o = *x * self.0;
        }
    }
}

/// Scales its input by a factor that can be updated live via `set_params`.
struct LiveScale(f32);
impl Processor for LiveScale {
    fn process(&mut self, ctx: &ProcessContext, inp: &[&[f32]], out: &mut [&mut [f32]]) {
        for (o, x) in out[0].iter_mut().zip(inp[0].iter()).take(ctx.frames) {
            *o = *x * self.0;
        }
    }
    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&f) = values.first() {
            self.0 = f;
        }
        true
    }
}

/// Sums its two inputs.
struct Sum;
impl Processor for Sum {
    fn process(&mut self, ctx: &ProcessContext, inp: &[&[f32]], out: &mut [&mut [f32]]) {
        for (i, o) in out[0].iter_mut().enumerate().take(ctx.frames) {
            *o = inp[0][i] + inp[1][i];
        }
    }
}

fn ctx(frames: usize) -> ProcessContext {
    ProcessContext {
        sample_rate: 48_000.0,
        frames,
    }
}

fn source(kind: &str) -> GraphNode {
    GraphNode::new(kind, vec![], vec![Port::output("out", SignalType::Audio)])
}

fn mono(kind: &str) -> GraphNode {
    GraphNode::new(
        kind,
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    )
}

#[test]
fn runs_a_const_scale_chain() {
    let mut g = Graph::new();
    let src = g.add_node(source("const"));
    let amp = g.add_node(mono("scale"));
    g.connect(src, PortId(0), amp, PortId(0)).unwrap();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(src, Box::new(Constant(2.0)));
    procs.insert(amp, Box::new(Scale(3.0)));

    let mut engine = RtEngine::compile(&g, procs, 4).unwrap();
    engine.process(&ctx(4));

    assert_eq!(
        engine.node_output(amp).unwrap().channel(0).unwrap(),
        &[6.0; 4]
    );
}

#[test]
fn set_node_params_updates_a_node_live_or_reports_unsupported() {
    let mut g = Graph::new();
    let src = g.add_node(source("const"));
    let amp = g.add_node(mono("live"));
    g.connect(src, PortId(0), amp, PortId(0)).unwrap();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(src, Box::new(Constant(1.0)));
    procs.insert(amp, Box::new(LiveScale(1.0)));
    let mut engine = RtEngine::compile(&g, procs, 4).unwrap();

    engine.process(&ctx(4));
    assert_eq!(
        engine.node_output(amp).unwrap().channel(0).unwrap(),
        &[1.0; 4]
    );

    // A live update reaches the running processor — the next block reflects it, no rebuild.
    assert!(engine.set_node_params(amp, &[0.25]));
    engine.process(&ctx(4));
    assert_eq!(
        engine.node_output(amp).unwrap().channel(0).unwrap(),
        &[0.25; 4]
    );

    // A processor without a live implementation reports false, as does an unknown id.
    assert!(
        !engine.set_node_params(src, &[0.0]),
        "Constant has no set_params"
    );
    assert!(!engine.set_node_params(NodeId(9999), &[0.0]), "unknown id");
}

#[test]
fn routes_multiple_inputs() {
    let mut g = Graph::new();
    let a = g.add_node(source("a"));
    let b = g.add_node(source("b"));
    let sum = g.add_node(GraphNode::new(
        "sum",
        vec![
            Port::input("l", SignalType::Audio),
            Port::input("r", SignalType::Audio),
        ],
        vec![Port::output("out", SignalType::Audio)],
    ));
    g.connect(a, PortId(0), sum, PortId(0)).unwrap();
    g.connect(b, PortId(0), sum, PortId(1)).unwrap();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(a, Box::new(Constant(2.0)));
    procs.insert(b, Box::new(Constant(5.0)));
    procs.insert(sum, Box::new(Sum));

    let mut engine = RtEngine::compile(&g, procs, 3).unwrap();
    engine.process(&ctx(3));

    assert_eq!(
        engine.node_output(sum).unwrap().channel(0).unwrap(),
        &[7.0; 3]
    );
}

#[test]
fn unconnected_input_reads_as_silence() {
    let mut g = Graph::new();
    let amp = g.add_node(mono("scale"));
    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(amp, Box::new(Scale(9.0)));

    let mut engine = RtEngine::compile(&g, procs, 2).unwrap();
    engine.process(&ctx(2));
    assert_eq!(
        engine.node_output(amp).unwrap().channel(0).unwrap(),
        &[0.0; 2]
    );
}

#[test]
fn missing_processor_is_reported() {
    let mut g = Graph::new();
    let src = g.add_node(source("const"));
    match RtEngine::compile(&g, HashMap::new(), 4) {
        Err(RtEngineError::MissingProcessor(id)) => assert_eq!(id, src),
        Err(other) => panic!("expected MissingProcessor, got {other:?}"),
        Ok(_) => panic!("expected MissingProcessor, got Ok"),
    }
}

#[test]
fn a_cycle_is_reported() {
    let mut g = Graph::new();
    let a = g.add_node(mono("a"));
    let b = g.add_node(mono("b"));
    g.connect(a, PortId(0), b, PortId(0)).unwrap();
    g.connect(b, PortId(0), a, PortId(0)).unwrap();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(a, Box::new(Scale(1.0)));
    procs.insert(b, Box::new(Scale(1.0)));
    assert!(matches!(
        RtEngine::compile(&g, procs, 4),
        Err(RtEngineError::Graph(_))
    ));
}

#[test]
fn a_node_over_the_port_limit_is_rejected() {
    let too_many: Vec<Port> = (0..=MAX_PORTS_PER_NODE)
        .map(|i| Port::input(format!("in{i}"), SignalType::Audio))
        .collect();
    let mut g = Graph::new();
    let wide = g.add_node(GraphNode::new("wide", too_many, vec![]));

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(wide, Box::new(Constant(0.0)));
    assert!(matches!(
        RtEngine::compile(&g, procs, 4),
        Err(RtEngineError::TooManyPorts { .. })
    ));
}

#[test]
fn set_block_size_resizes_and_resets() {
    let mut g = Graph::new();
    let src = g.add_node(source("const"));
    let amp = g.add_node(mono("scale"));
    g.connect(src, PortId(0), amp, PortId(0)).unwrap();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(src, Box::new(Constant(2.0)));
    procs.insert(amp, Box::new(Scale(3.0)));

    let mut engine = RtEngine::compile(&g, procs, 4).unwrap();
    engine.set_block_size(8);
    assert_eq!(engine.block_size(), 8);
    engine.process(&ctx(8));
    assert_eq!(
        engine.node_output(amp).unwrap().channel(0).unwrap(),
        &[6.0; 8]
    );
}

/// The RT engine must be a drop-in for the reference evaluator: same graph, same
/// processors, same samples out.
#[test]
fn matches_the_reference_engine_on_a_real_patch() {
    let build = || {
        let mut g = Graph::new();
        let osc = g.add_node(source("oscillator"));
        let flt = g.add_node(mono("biquad"));
        let amp = g.add_node(mono("gain"));
        g.connect(osc, PortId(0), flt, PortId(0)).unwrap();
        g.connect(flt, PortId(0), amp, PortId(0)).unwrap();
        (g, osc, flt, amp)
    };
    let (graph, osc, flt, amp) = build();

    let mut reference_procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    reference_procs.insert(osc, Box::new(Oscillator::new(Waveform::Saw, 220.0)));
    reference_procs.insert(
        flt,
        Box::new(Biquad::new(FilterKind::LowPass, 2_000.0, 0.707)),
    );
    reference_procs.insert(amp, Box::new(Gain::new(0.5)));

    let mut rt_procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    rt_procs.insert(osc, Box::new(Oscillator::new(Waveform::Saw, 220.0)));
    rt_procs.insert(
        flt,
        Box::new(Biquad::new(FilterKind::LowPass, 2_000.0, 0.707)),
    );
    rt_procs.insert(amp, Box::new(Gain::new(0.5)));

    let mut reference = Engine::compile(&graph, reference_procs, 256).unwrap();
    let mut rt = RtEngine::compile(&graph, rt_procs, 256).unwrap();

    // Several blocks so filter/oscillator state has to line up too.
    for _ in 0..4 {
        reference.process(&ctx(256));
        rt.process(&ctx(256));
    }

    let expected = reference.node_output(amp).unwrap().channel(0).unwrap();
    let actual = rt.node_output(amp).unwrap().channel(0).unwrap();
    assert_eq!(expected, actual);
    assert!(
        actual.iter().any(|&s| s.abs() > 0.01),
        "patch should carry signal"
    );
}

/// Emits 1.0 on its very first sample, then silence.
struct ImpulseOnce {
    fired: bool,
}
impl Processor for ImpulseOnce {
    fn process(&mut self, ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
        out[0][..ctx.frames].fill(0.0);
        if !self.fired {
            out[0][0] = 1.0;
            self.fired = true;
        }
    }
}

/// out = in0 + 0.5 * in1.
struct SumHalf;
impl Processor for SumHalf {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], out: &mut [&mut [f32]]) {
        for i in 0..ctx.frames {
            out[0][i] = inputs[0][i] + 0.5 * inputs[1][i];
        }
    }
}

/// Copies input to output (sits on the feedback boundary).
struct Passthrough;
impl Processor for Passthrough {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], out: &mut [&mut [f32]]) {
        out[0][..ctx.frames].copy_from_slice(&inputs[0][..ctx.frames]);
    }
}

#[test]
fn rt_engine_runs_feedback_loops_one_block_late() {
    let mut g = Graph::new();
    let imp = g.add_node(GraphNode::new(
        "impulse",
        vec![],
        vec![Port::output("out", SignalType::Audio)],
    ));
    let sum = g.add_node(GraphNode::new(
        "sum-half",
        vec![
            Port::input("a", SignalType::Audio),
            Port::input("b", SignalType::Audio),
        ],
        vec![Port::output("out", SignalType::Audio)],
    ));
    let fb = g.add_node(GraphNode::feedback_node(
        "fb",
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    ));
    g.connect(imp, PortId(0), sum, PortId(0)).unwrap();
    g.connect(fb, PortId(0), sum, PortId(1)).unwrap();
    g.connect(sum, PortId(0), fb, PortId(0)).unwrap();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(imp, Box::new(ImpulseOnce { fired: false }));
    procs.insert(sum, Box::new(SumHalf));
    procs.insert(fb, Box::new(Passthrough));
    let mut rt = RtEngine::compile(&g, procs, 4).expect("feedback graph compiles in RT");

    for expected in [1.0_f32, 0.5, 0.25, 0.125] {
        rt.process(&ctx(4));
        let out = rt.node_output(sum).unwrap().channel(0).unwrap();
        assert!(
            (out[0] - expected).abs() < 1e-6,
            "expected {expected}, got {}",
            out[0]
        );
    }
}

#[test]
fn a_macro_node_runs_inside_the_rt_engine() {
    use vb_core::{MacroProcessor, MacroSpec};

    // Inner subgraph: boundary → double gain (via two 0.5-inverse… use Gain 2.0
    // from vb-nodes) → out.
    let mut inner = Graph::new();
    let boundary = inner.add_node(GraphNode::new(
        "macro-in",
        vec![],
        vec![Port::output("in0", SignalType::Audio)],
    ));
    let amp = inner.add_node(GraphNode::new(
        "gain",
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    ));
    inner.connect(boundary, PortId(0), amp, PortId(0)).unwrap();
    let mut inner_procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    inner_procs.insert(amp, Box::new(Gain::new(2.0)));
    let macro_proc = MacroProcessor::compile(
        MacroSpec {
            graph: inner,
            input_node: boundary,
            outputs: vec![(amp, PortId(0))],
        },
        inner_procs,
        64,
    )
    .unwrap();

    // Outer RT graph: constant source → macro → sink read.
    struct Constant(f32);
    impl Processor for Constant {
        fn process(&mut self, ctx: &ProcessContext, _i: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0][..ctx.frames].fill(self.0);
        }
    }
    let mut outer = Graph::new();
    let src = outer.add_node(GraphNode::new(
        "const",
        vec![],
        vec![Port::output("out", SignalType::Audio)],
    ));
    let mac = outer.add_node(GraphNode::new(
        "macro",
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    ));
    outer.connect(src, PortId(0), mac, PortId(0)).unwrap();

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(src, Box::new(Constant(0.25)));
    procs.insert(mac, Box::new(macro_proc));
    let mut rt = RtEngine::compile(&outer, procs, 64).unwrap();

    rt.process(&ctx(64));
    let out = rt.node_output(mac).unwrap().channel(0).unwrap();
    assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6), "0.25 * 2.0");
}
