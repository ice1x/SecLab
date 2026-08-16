//! Proves the central real-time claim: the audio-thread path — `RtEngine::process`
//! *and* `EngineReceiver::poll_swap` — performs no heap allocation and no deallocation.
//!
//! A counting global allocator is installed for this test binary. A global allocator sees
//! *every* thread, including the test harness's own bookkeeping, so counting is armed per
//! thread: only traffic on the thread running the measured loop is tallied. Setup is free
//! to allocate as much as it likes — the window opens after it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;

use vb_core::{Graph, GraphNode, NodeId, Port, PortId, ProcessContext, Processor, SignalType};
use vb_nodes::{Adsr, Biquad, FilterKind, Mul, Oscillator, Waveform};
use vb_runtime::{engine_swap, RtEngine};

thread_local! {
    /// Whether this thread is inside the measured window.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    /// Allocations this thread made while armed.
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    /// Deallocations this thread made while armed.
    static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

/// Bumps `counter` if this thread is armed.
///
/// The thread-locals are const-initialised and hold no destructor, so touching them
/// cannot itself allocate or re-enter the allocator. `try_with` keeps this sound even
/// during thread teardown, when the locals may already be gone.
fn note(counter: &'static std::thread::LocalKey<Cell<usize>>) {
    let armed = COUNTING.try_with(Cell::get).unwrap_or(false);
    if armed {
        let _ = counter.try_with(|n| n.set(n.get() + 1));
    }
}

struct CountingAllocator;

// SAFETY: every method forwards to the system allocator unchanged; the counters only
// observe traffic and never affect the returned pointers.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(&ALLOCATIONS);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(&DEALLOCATIONS);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(&ALLOCATIONS);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(&ALLOCATIONS);
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

    // A replacement engine (same patch, silent oscillator level is irrelevant — only
    // the swap mechanics matter), parked in the pending mailbox before the window so
    // the measured loop performs one real swap plus many no-op polls.
    let (tx, rx) = engine_swap();
    let mut procs2: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs2.insert(osc, Box::new(Oscillator::new(Waveform::Saw, 110.0)));
    procs2.insert(
        flt,
        Box::new(Biquad::new(FilterKind::LowPass, 2_000.0, 0.707)),
    );
    procs2.insert(gate, Box::new(GateHigh));
    procs2.insert(env, Box::new(Adsr::new(0.001, 0.001, 0.8, 0.01)));
    procs2.insert(vca, Box::new(Mul));
    tx.install(RtEngine::compile(&graph, procs2, 256).unwrap());

    // Warm up outside the measured window.
    engine.process(&ctx);

    ALLOCATIONS.with(|n| n.set(0));
    DEALLOCATIONS.with(|n| n.set(0));
    COUNTING.with(|c| c.set(true));
    let mut swaps = 0;
    for _ in 0..64 {
        if rx.poll_swap(&mut engine) {
            swaps += 1;
        }
        engine.process(&ctx);
    }
    COUNTING.with(|c| c.set(false));

    assert_eq!(
        swaps, 1,
        "exactly one pending engine should have been swapped in"
    );
    assert_eq!(
        ALLOCATIONS.with(Cell::get),
        0,
        "the audio-thread path (process + poll_swap) must not allocate"
    );
    assert_eq!(
        DEALLOCATIONS.with(Cell::get),
        0,
        "the audio-thread path must not free memory either — retirees are parked"
    );

    // And it still produced audio.
    let out = engine.node_output(vca).unwrap().channel(0).unwrap();
    assert!(out.iter().any(|&s| s.abs() > 0.01));

    // The retired engine is freed on the editor side.
    assert!(tx.collect());
}
