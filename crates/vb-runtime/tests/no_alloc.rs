//! Proves the central real-time claim: `RtEngine::process` performs no heap allocation.
//!
//! A counting global allocator is installed for this test binary. Allocations are only
//! tallied inside the measured window, so setup is free to allocate as much as it likes.
//! This file deliberately holds a single test — the counter is process-wide, so a second
//! test running in parallel would pollute it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use vb_core::{Graph, GraphNode, NodeId, Port, PortId, ProcessContext, Processor, SignalType};
use vb_nodes::{Adsr, Biquad, FilterKind, Mul, Oscillator, Waveform};
use vb_runtime::RtEngine;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

struct CountingAllocator;

// SAFETY: every method forwards to the system allocator unchanged; the counters only
// observe traffic and never affect the returned pointers.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn audio_out() -> Vec<Port> {
    vec![Port::output("out", SignalType::Audio)]
}

#[test]
fn processing_a_block_does_not_allocate() {
    // A patch with fan-in, state and several node kinds — osc -> filter -> vca <- adsr.
    let mut graph = Graph::new();
    let osc = graph.add_node(GraphNode::new("oscillator", vec![], audio_out()));
    let flt = graph.add_node(GraphNode::new(
        "biquad",
        vec![Port::input("in", SignalType::Audio)],
        audio_out(),
    ));
    let gate = graph.add_node(GraphNode::new(
        "gate",
        vec![],
        vec![Port::output("g", SignalType::Control)],
    ));
    let env = graph.add_node(GraphNode::new(
        "adsr",
        vec![Port::input("gate", SignalType::Control)],
        vec![Port::output("level", SignalType::Control)],
    ));
    let vca = graph.add_node(GraphNode::new(
        "vca",
        vec![
            Port::input("a", SignalType::Audio),
            Port::input("b", SignalType::Control),
        ],
        audio_out(),
    ));
    graph.connect(osc, PortId(0), flt, PortId(0)).unwrap();
    graph.connect(gate, PortId(0), env, PortId(0)).unwrap();
    graph.connect(flt, PortId(0), vca, PortId(0)).unwrap();
    graph.connect(env, PortId(0), vca, PortId(1)).unwrap();

    struct GateHigh;
    impl Processor for GateHigh {
        fn process(&mut self, ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0].iter_mut().take(ctx.frames).for_each(|o| *o = 1.0);
        }
    }

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(osc, Box::new(Oscillator::new(Waveform::Saw, 220.0)));
    procs.insert(
        flt,
        Box::new(Biquad::new(FilterKind::LowPass, 2_000.0, 0.707)),
    );
    procs.insert(gate, Box::new(GateHigh));
    procs.insert(env, Box::new(Adsr::new(0.001, 0.001, 0.8, 0.01)));
    procs.insert(vca, Box::new(Mul));

    let mut engine = RtEngine::compile(&graph, procs, 256).unwrap();
    let ctx = ProcessContext {
        sample_rate: 48_000.0,
        frames: 256,
    };

    // Warm up outside the measured window.
    engine.process(&ctx);

    ALLOCATIONS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    for _ in 0..64 {
        engine.process(&ctx);
    }
    COUNTING.store(false, Ordering::SeqCst);

    assert_eq!(
        ALLOCATIONS.load(Ordering::SeqCst),
        0,
        "RtEngine::process must not allocate on the audio thread"
    );

    // And it still produced audio.
    let out = engine.node_output(vca).unwrap().channel(0).unwrap();
    assert!(out.iter().any(|&s| s.abs() > 0.01));
}
