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
