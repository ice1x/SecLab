//! Macro node: a nested subgraph that behaves as a single [`Processor`].
//!
//! The Reaktor-style building block (task 000038). A [`MacroSpec`] is a complete
//! inner [`Graph`] plus two boundary declarations: which inner node stands for the
//! macro's inputs (its output ports mirror the macro's input ports, fed from the
//! outside via [`Engine::process_with_feeds`]), and which inner `(node, port)` pairs
//! feed each of the macro's outputs. [`MacroProcessor`] compiles that into an inner
//! engine and implements [`Processor`], so a whole subgraph drops into any outer
//! graph — including another macro (nesting works by construction).
//!
//! The inner executor is the reference [`Engine`], which allocates during
//! `process` — fine for offline/editor use. A flattening pass that inlines macros
//! into the outer graph before RT compilation is the planned zero-allocation path.

use std::collections::HashMap;

use crate::engine::{Engine, EngineError};
use crate::graph::Graph;
use crate::node::NodeId;
use crate::port::PortId;
use crate::process::{ProcessContext, Processor};

/// A subgraph packaged with its boundary wiring.
pub struct MacroSpec {
    /// The inner graph.
    pub graph: Graph,
    /// Inner node standing for the macro's inputs: its output port `i` carries the
    /// macro's input `i`. It needs no real processor — the macro feeds it directly.
    pub input_node: NodeId,
    /// For each macro output, the inner `(node, port)` that produces it.
    pub outputs: Vec<(NodeId, PortId)>,
}

/// Placeholder processor for the input boundary node; never actually runs because
/// the macro feeds that node via [`Engine::process_with_feeds`].
struct BoundaryFeed;

impl Processor for BoundaryFeed {
    fn process(&mut self, _ctx: &ProcessContext, _inputs: &[&[f32]], _outputs: &mut [&mut [f32]]) {}
}

/// A compiled subgraph usable as a single node's processor.
pub struct MacroProcessor {
    engine: Engine,
    input_node: NodeId,
    outputs: Vec<(NodeId, PortId)>,
}

impl MacroProcessor {
    /// Compiles the spec with processors for every inner node except the input
    /// boundary (which gets a placeholder automatically).
    ///
    /// # Errors
    ///
    /// Propagates [`EngineError`] from the inner [`Engine::compile`] (missing
    /// processors, cycles, …).
    pub fn compile(
        spec: MacroSpec,
        mut processors: HashMap<NodeId, Box<dyn Processor>>,
        frames: usize,
    ) -> Result<Self, EngineError> {
        processors
            .entry(spec.input_node)
            .or_insert_with(|| Box::new(BoundaryFeed));
        let engine = Engine::compile(&spec.graph, processors, frames)?;
        Ok(Self {
            engine,
            input_node: spec.input_node,
            outputs: spec.outputs,
        })
    }
}

impl Processor for MacroProcessor {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        self.engine
            .process_with_feeds(ctx, &[(self.input_node, inputs)]);
        for (out, (node, port)) in outputs.iter_mut().zip(&self.outputs) {
            match self
                .engine
                .node_output(*node)
                .and_then(|buf| buf.channel(port.0))
            {
                Some(src) => {
                    let n = src.len().min(out.len()).min(ctx.frames);
                    out[..n].copy_from_slice(&src[..n]);
                }
                None => out[..ctx.frames].fill(0.0),
            }
        }
    }

    fn reset(&mut self) {
        self.engine.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::GraphNode;
    use crate::port::Port;
    use crate::signal::SignalType;

    fn audio_in(name: &str) -> Port {
        Port::input(name, SignalType::Audio)
    }
    fn audio_out(name: &str) -> Port {
        Port::output(name, SignalType::Audio)
    }

    /// out = in * gain.
    struct Gain(f32);
    impl Processor for Gain {
        fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], out: &mut [&mut [f32]]) {
            for i in 0..ctx.frames {
                out[0][i] = inputs[0][i] * self.0;
            }
        }
    }

    /// Stateful ramp: adds an ever-increasing offset (proves state survives blocks).
    struct Counter(f32);
    impl Processor for Counter {
        fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], out: &mut [&mut [f32]]) {
            for i in 0..ctx.frames {
                out[0][i] = inputs[0][i] + self.0;
            }
            self.0 += 1.0;
        }
        fn reset(&mut self) {
            self.0 = 0.0;
        }
    }

    const CTX: ProcessContext = ProcessContext {
        sample_rate: 48_000.0,
        frames: 4,
    };

    /// A macro wrapping `input → gain(0.5) → output`.
    fn half_gain_macro() -> MacroProcessor {
        let mut g = Graph::new();
        let inp = g.add_node(GraphNode::new("macro-in", vec![], vec![audio_out("in0")]));
        let amp = g.add_node(GraphNode::new(
            "gain",
            vec![audio_in("in")],
            vec![audio_out("out")],
        ));
        g.connect(inp, PortId(0), amp, PortId(0)).unwrap();

        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(amp, Box::new(Gain(0.5)));
        MacroProcessor::compile(
            MacroSpec {
                graph: g,
                input_node: inp,
                outputs: vec![(amp, PortId(0))],
            },
            procs,
            4,
        )
        .unwrap()
    }

    #[test]
    fn a_macro_behaves_like_its_flat_subgraph() {
        let mut macro_node = half_gain_macro();
        let input = [1.0_f32, -2.0, 3.0, 0.5];
        let mut out = [0.0_f32; 4];
        macro_node.process(&CTX, &[&input], &mut [&mut out]);
        assert_eq!(out, [0.5, -1.0, 1.5, 0.25]);
    }

    #[test]
    fn multi_port_boundaries_route_correctly() {
        // Two inputs crossed to two outputs: out0 = in1, out1 = in0 * 0.5.
        let mut g = Graph::new();
        let inp = g.add_node(GraphNode::new(
            "macro-in",
            vec![],
            vec![audio_out("in0"), audio_out("in1")],
        ));
        let amp = g.add_node(GraphNode::new(
            "gain",
            vec![audio_in("in")],
            vec![audio_out("out")],
        ));
        g.connect(inp, PortId(0), amp, PortId(0)).unwrap();

        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(amp, Box::new(Gain(0.5)));
        let mut macro_node = MacroProcessor::compile(
            MacroSpec {
                graph: g,
                input_node: inp,
                // out0 taps the boundary's second port directly; out1 the gain.
                outputs: vec![(inp, PortId(1)), (amp, PortId(0))],
            },
            procs,
            4,
        )
        .unwrap();

        let in0 = [1.0_f32; 4];
        let in1 = [7.0_f32; 4];
        let (mut out0, mut out1) = ([0.0_f32; 4], [0.0_f32; 4]);
        macro_node.process(&CTX, &[&in0, &in1], &mut [&mut out0, &mut out1]);
        assert_eq!(out0, [7.0; 4], "passthrough of the second input");
        assert_eq!(out1, [0.5; 4], "first input through the gain");
    }

    #[test]
    fn inner_state_persists_across_blocks_and_reset_clears_it() {
        let mut g = Graph::new();
        let inp = g.add_node(GraphNode::new("macro-in", vec![], vec![audio_out("in0")]));
        let counter = g.add_node(GraphNode::new(
            "counter",
            vec![audio_in("in")],
            vec![audio_out("out")],
        ));
        g.connect(inp, PortId(0), counter, PortId(0)).unwrap();

        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(counter, Box::new(Counter(0.0)));
        let mut macro_node = MacroProcessor::compile(
            MacroSpec {
                graph: g,
                input_node: inp,
                outputs: vec![(counter, PortId(0))],
            },
            procs,
            4,
        )
        .unwrap();

        let input = [0.0_f32; 4];
        let mut out = [0.0_f32; 4];
        for expected in [0.0_f32, 1.0, 2.0] {
            macro_node.process(&CTX, &[&input], &mut [&mut out]);
            assert_eq!(out, [expected; 4]);
        }
        macro_node.reset();
        macro_node.process(&CTX, &[&input], &mut [&mut out]);
        assert_eq!(out, [0.0; 4], "reset reached the inner processor");
    }

    #[test]
    fn macros_nest() {
        // Outer macro: input → inner macro (gain 0.5) → output. Result: 0.25 overall
        // when wrapped once more below.
        let mut g = Graph::new();
        let inp = g.add_node(GraphNode::new("macro-in", vec![], vec![audio_out("in0")]));
        let inner = g.add_node(GraphNode::new(
            "inner-macro",
            vec![audio_in("in")],
            vec![audio_out("out")],
        ));
        g.connect(inp, PortId(0), inner, PortId(0)).unwrap();

        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(inner, Box::new(half_gain_macro()));
        let mut outer = MacroProcessor::compile(
            MacroSpec {
                graph: g,
                input_node: inp,
                outputs: vec![(inner, PortId(0))],
            },
            procs,
            4,
        )
        .unwrap();

        let input = [1.0_f32; 4];
        let mut out = [0.0_f32; 4];
        outer.process(&CTX, &[&input], &mut [&mut out]);
        assert_eq!(out, [0.5; 4], "one nesting level: half gain");

        // And a macro is Send like any processor (engines cross threads whole).
        fn assert_send<T: Send>(_: &T) {}
        assert_send(&outer);
    }
}
