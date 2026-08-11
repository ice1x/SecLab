//! A reference executor: run a graph's processors in topological order, block by block.
//!
//! This is the correctness-first, non-real-time evaluator. It copies input samples when
//! routing to keep the borrow checker (and the logic) simple; the allocation-free,
//! RT-safe executor lives in `vb-runtime` (000057). Both share this crate's model.

use std::collections::HashMap;

use crate::buffer::AudioBuffer;
use crate::graph::{Graph, GraphError};
use crate::node::NodeId;
use crate::process::{ProcessContext, Processor};

/// Why building an [`Engine`] can fail.
#[derive(Debug)]
pub enum EngineError {
    /// The graph could not be ordered (e.g. it contains a cycle).
    Graph(GraphError),
    /// No processor was supplied for a node in the graph.
    MissingProcessor(NodeId),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Graph(e) => write!(f, "graph error: {e}"),
            Self::MissingProcessor(id) => write!(f, "no processor supplied for node {id:?}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// A compiled, runnable graph: processors plus the routing needed to drive them.
pub struct Engine {
    order: Vec<NodeId>,
    index: HashMap<NodeId, usize>,
    /// Per node (in `order`), one output buffer whose channels are the node's outputs.
    outputs: Vec<AudioBuffer>,
    /// Per node (in `order`), the processor that produces its outputs.
    processors: Vec<Box<dyn Processor>>,
    /// Per node (in `order`), per input port: the `(node index, output port)` feeding it.
    input_sources: Vec<Vec<Option<(usize, usize)>>>,
    frames: usize,
}

impl Engine {
    /// Compiles `graph` into a runnable engine, taking ownership of one processor per node.
    ///
    /// `frames` is the block size the engine will process.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Graph`] if the graph cannot be ordered (e.g. a cycle) and
    /// [`EngineError::MissingProcessor`] if any node has no processor.
    pub fn compile(
        graph: &Graph,
        mut processors: HashMap<NodeId, Box<dyn Processor>>,
        frames: usize,
    ) -> Result<Self, EngineError> {
        let order = graph.topological_order().map_err(EngineError::Graph)?;
        let index: HashMap<NodeId, usize> =
            order.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let mut outputs = Vec::with_capacity(order.len());
        let mut procs = Vec::with_capacity(order.len());
        let mut input_sources = Vec::with_capacity(order.len());

        for &id in &order {
            let node = graph.node(id).expect("id from topological order is valid");
            outputs.push(AudioBuffer::new(node.outputs.len(), frames));
            procs.push(
                processors
                    .remove(&id)
                    .ok_or(EngineError::MissingProcessor(id))?,
            );

            let sources = (0..node.inputs.len())
                .map(|port| {
                    graph
                        .edges()
                        .iter()
                        .find(|e| e.to_node == id && e.to_port.0 == port)
                        .map(|e| (index[&e.from_node], e.from_port.0))
                })
                .collect();
            input_sources.push(sources);
        }

        Ok(Self {
            order,
            index,
            outputs,
            processors: procs,
            input_sources,
            frames,
        })
    }

    /// Processes one block, updating every node's output buffer in topological order.
    pub fn process(&mut self, ctx: &ProcessContext) {
        for i in 0..self.order.len() {
            // Gather inputs as owned copies so writing `outputs[i]` can't alias a source.
            let inputs: Vec<Vec<f32>> = self.input_sources[i]
                .iter()
                .map(|src| match src {
                    Some((sidx, sport)) => self.outputs[*sidx]
                        .channel(*sport)
                        .expect("resolved source port exists")
                        .to_vec(),
                    None => vec![0.0; self.frames],
                })
                .collect();
            let in_slices: Vec<&[f32]> = inputs.iter().map(Vec::as_slice).collect();

            let out_buf = &mut self.outputs[i];
            out_buf.clear();
            let mut out_channels: Vec<&mut [f32]> = out_buf.channels_mut().collect();

            self.processors[i].process(ctx, &in_slices, &mut out_channels);
        }
    }

    /// Returns the output buffer of `id` after the last [`Engine::process`], if present.
    #[must_use]
    pub fn node_output(&self, id: NodeId) -> Option<&AudioBuffer> {
        self.index.get(&id).map(|&i| &self.outputs[i])
    }
}

#[cfg(test)]
mod tests {
    use super::{Engine, EngineError};
    use crate::graph::Graph;
    use crate::node::{GraphNode, NodeId};
    use crate::port::{Port, PortId};
    use crate::process::{ProcessContext, Processor};
    use crate::signal::SignalType;
    use std::collections::HashMap;

    /// Emits a constant on its single output.
    struct Constant(f32);
    impl Processor for Constant {
        fn process(&mut self, _ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0].fill(self.0);
        }
    }

    /// Scales its single input onto its single output.
    struct Gain(f32);
    impl Processor for Gain {
        fn process(&mut self, ctx: &ProcessContext, inp: &[&[f32]], out: &mut [&mut [f32]]) {
            for (o, x) in out[0].iter_mut().zip(inp[0].iter()).take(ctx.frames) {
                *o = *x * self.0;
            }
        }
    }

    /// Sums its two inputs onto its single output.
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
    fn runs_a_const_gain_chain() {
        let mut g = Graph::new();
        let src = g.add_node(source("const"));
        let amp = g.add_node(mono("gain"));
        g.connect(src, PortId(0), amp, PortId(0)).unwrap();

        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(src, Box::new(Constant(2.0)));
        procs.insert(amp, Box::new(Gain(3.0)));

        let mut engine = Engine::compile(&g, procs, 4).unwrap();
        engine.process(&ctx(4));

        assert_eq!(
            engine.node_output(amp).unwrap().channel(0).unwrap(),
            &[6.0, 6.0, 6.0, 6.0]
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

        let mut engine = Engine::compile(&g, procs, 3).unwrap();
        engine.process(&ctx(3));

        assert_eq!(
            engine.node_output(sum).unwrap().channel(0).unwrap(),
            &[7.0, 7.0, 7.0]
        );
    }

    #[test]
    fn unconnected_input_reads_as_silence() {
        let mut g = Graph::new();
        let amp = g.add_node(mono("gain"));
        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(amp, Box::new(Gain(9.0)));

        let mut engine = Engine::compile(&g, procs, 2).unwrap();
        engine.process(&ctx(2));
        // 9.0 * silence == silence.
        assert_eq!(
            engine.node_output(amp).unwrap().channel(0).unwrap(),
            &[0.0, 0.0]
        );
    }

    #[test]
    fn missing_processor_is_reported() {
        let mut g = Graph::new();
        let src = g.add_node(source("const"));
        let procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        match Engine::compile(&g, procs, 4) {
            Err(EngineError::MissingProcessor(id)) => assert_eq!(id, src),
            Err(other) => panic!("expected MissingProcessor, got {other:?}"),
            Ok(_) => panic!("expected MissingProcessor, got Ok"),
        }
    }

    #[test]
    fn cycle_is_reported_as_graph_error() {
        let mut g = Graph::new();
        let a = g.add_node(mono("a"));
        let b = g.add_node(mono("b"));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        g.connect(b, PortId(0), a, PortId(0)).unwrap();

        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(a, Box::new(Gain(1.0)));
        procs.insert(b, Box::new(Gain(1.0)));
        assert!(matches!(
            Engine::compile(&g, procs, 4),
            Err(EngineError::Graph(_))
        ));
    }
}
