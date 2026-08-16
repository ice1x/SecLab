//! Criterion benchmarks on typical patch shapes (task 000023).
//!
//! Everything measured is the per-block hot path: `RtEngine::process` on graphs a
//! real patch would have, the reference `Engine` on the same patch for comparison,
//! and the cost of an idle `poll_swap` (the price every audio block pays even when
//! nothing changed). Run with `cargo bench -p vb-runtime`.

use std::collections::HashMap;

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use vb_core::{Engine, Graph, GraphNode, NodeId, Port, ProcessContext, Processor, SignalType};
use vb_nodes::{BuildContext, Registry};
use vb_runtime::{engine_swap, RtEngine};

const BLOCK: usize = 256;

fn ctx() -> ProcessContext {
    ProcessContext {
        sample_rate: 48_000.0,
        frames: BLOCK,
    }
}

fn audio_in(name: &str) -> Port {
    Port::input(name, SignalType::Audio)
}

fn audio_out(name: &str) -> Port {
    Port::output(name, SignalType::Audio)
}

/// Adds a registry node to the graph and processor map.
fn add(
    registry: &Registry,
    g: &mut Graph,
    procs: &mut HashMap<NodeId, Box<dyn Processor>>,
    kind: &str,
) -> NodeId {
    let id = g.add_node(registry.graph_node(kind).expect("known kind"));
    procs.insert(
        id,
        registry
            .get(kind)
            .expect("known kind")
            .processor_with(&BuildContext::default(), &[]),
    );
    id
}

/// osc → svf → gain: the minimal synth voice.
fn small_voice(registry: &Registry) -> (Graph, HashMap<NodeId, Box<dyn Processor>>, NodeId) {
    let mut g = Graph::new();
    let mut procs = HashMap::new();
    let osc = add(registry, &mut g, &mut procs, "oscillator");
    let svf = add(registry, &mut g, &mut procs, "svf");
    let amp = add(registry, &mut g, &mut procs, "gain");
    g.connect(osc, vb_core::PortId(0), svf, vb_core::PortId(0))
        .unwrap();
    g.connect(svf, vb_core::PortId(0), amp, vb_core::PortId(0))
        .unwrap();
    (g, procs, amp)
}

/// 2 osc → mix → biquad → waveshaper → fb-delay → gain: a real little patch.
fn medium_patch(registry: &Registry) -> (Graph, HashMap<NodeId, Box<dyn Processor>>, NodeId) {
    let mut g = Graph::new();
    let mut procs = HashMap::new();
    let p = vb_core::PortId;
    let osc1 = add(registry, &mut g, &mut procs, "oscillator");
    let osc2 = add(registry, &mut g, &mut procs, "oscillator");
    let mix = add(registry, &mut g, &mut procs, "mix");
    let flt = add(registry, &mut g, &mut procs, "biquad");
    let shape = add(registry, &mut g, &mut procs, "waveshaper");
    let delay = add(registry, &mut g, &mut procs, "fb-delay");
    let amp = add(registry, &mut g, &mut procs, "gain");
    g.connect(osc1, p(0), mix, p(0)).unwrap();
    g.connect(osc2, p(0), mix, p(1)).unwrap();
    g.connect(mix, p(0), flt, p(0)).unwrap();
    g.connect(flt, p(0), shape, p(0)).unwrap();
    g.connect(shape, p(0), delay, p(0)).unwrap();
    g.connect(delay, p(0), amp, p(0)).unwrap();
    (g, procs, amp)
}

/// 8 parallel osc → gain chains merged pairwise: a wide patch (~27 nodes).
fn wide_patch(registry: &Registry) -> (Graph, HashMap<NodeId, Box<dyn Processor>>, NodeId) {
    let mut g = Graph::new();
    let mut procs = HashMap::new();
    let p = vb_core::PortId;
    let mut outs: Vec<NodeId> = Vec::new();
    for _ in 0..8 {
        let osc = add(registry, &mut g, &mut procs, "oscillator");
        let amp = add(registry, &mut g, &mut procs, "gain");
        g.connect(osc, p(0), amp, p(0)).unwrap();
        outs.push(amp);
    }
    // Merge 8 → 2 mergers (4 inputs each) → 1 merger.
    let m1 = add(registry, &mut g, &mut procs, "merger");
    let m2 = add(registry, &mut g, &mut procs, "merger");
    for (i, id) in outs.iter().take(4).enumerate() {
        g.connect(*id, p(0), m1, p(i)).unwrap();
    }
    for (i, id) in outs.iter().skip(4).enumerate() {
        g.connect(*id, p(0), m2, p(i)).unwrap();
    }
    let root = add(registry, &mut g, &mut procs, "merger");
    g.connect(m1, p(0), root, p(0)).unwrap();
    g.connect(m2, p(0), root, p(1)).unwrap();
    (g, procs, root)
}

/// Builds a fresh processor map for `graph`'s nodes from the registry.
fn procs_for(registry: &Registry, graph: &Graph) -> HashMap<NodeId, Box<dyn Processor>> {
    graph
        .node_ids()
        .map(|id| {
            let kind = &graph.node(id).unwrap().kind;
            (
                id,
                registry
                    .get(kind)
                    .unwrap()
                    .processor_with(&BuildContext::default(), &[]),
            )
        })
        .collect()
}

fn bench_process(c: &mut Criterion) {
    let registry = Registry::with_builtins();
    let context = ctx();

    let mut group = c.benchmark_group("rt_process_block");
    for (name, builder) in [
        ("small_voice_3_nodes", small_voice as fn(&Registry) -> _),
        ("medium_patch_7_nodes", medium_patch),
        ("wide_patch_27_nodes", wide_patch),
    ] {
        let (graph, procs, _) = builder(&registry);
        let mut engine = RtEngine::compile(&graph, procs, BLOCK).unwrap();
        group.bench_function(name, |b| {
            b.iter(|| {
                engine.process(black_box(&context));
            });
        });
    }
    group.finish();

    // Reference engine on the same medium patch, for the RT-vs-reference gap.
    let mut group = c.benchmark_group("reference_process_block");
    let (graph, _, _) = medium_patch(&registry);
    let mut reference = Engine::compile(&graph, procs_for(&registry, &graph), BLOCK).unwrap();
    group.bench_function("medium_patch_7_nodes", |b| {
        b.iter(|| {
            reference.process(black_box(&context));
        });
    });
    group.finish();
}

fn bench_swap_overhead(c: &mut Criterion) {
    // The cost every block pays: polling an empty mailbox before processing.
    let mut g = Graph::new();
    let src = g.add_node(GraphNode::new("src", vec![], vec![audio_out("out")]));
    let dst = g.add_node(GraphNode::new(
        "dst",
        vec![audio_in("in")],
        vec![audio_out("out")],
    ));
    g.connect(src, vb_core::PortId(0), dst, vb_core::PortId(0))
        .unwrap();

    struct Silence;
    impl Processor for Silence {
        fn process(&mut self, ctx: &ProcessContext, _i: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0][..ctx.frames].fill(0.0);
        }
    }
    struct Copy;
    impl Processor for Copy {
        fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0][..ctx.frames].copy_from_slice(&inputs[0][..ctx.frames]);
        }
    }
    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    procs.insert(src, Box::new(Silence));
    procs.insert(dst, Box::new(Copy));
    let mut engine = RtEngine::compile(&g, procs, BLOCK).unwrap();

    let (_tx, rx) = engine_swap();
    c.bench_function("poll_swap_empty_mailbox", |b| {
        b.iter(|| {
            black_box(rx.poll_swap(black_box(&mut engine)));
        });
    });
}

criterion_group!(benches, bench_process, bench_swap_overhead);
criterion_main!(benches);
