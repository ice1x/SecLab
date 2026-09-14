//! The allocation-free graph executor used on the audio thread.

use std::collections::HashMap;
use std::fmt;

use vb_core::{AudioBuffer, Graph, GraphError, NodeId, ProcessContext, Processor};

/// Maximum number of ports a node may expose on one side.
///
/// The per-block slice tables handed to a processor live on the stack so that
/// [`RtEngine::process`] never touches the heap; this is their fixed width.
pub const MAX_PORTS_PER_NODE: usize = 16;

/// Why building an [`RtEngine`] can fail.
#[derive(Debug)]
pub enum RtEngineError {
    /// The graph could not be ordered (e.g. it contains a cycle).
    Graph(GraphError),
    /// No processor was supplied for a node in the graph.
    MissingProcessor(NodeId),
    /// A node exceeds [`MAX_PORTS_PER_NODE`] on one side.
    TooManyPorts {
        /// The offending node.
        node: NodeId,
        /// How many ports it declared on the wider side.
        ports: usize,
    },
}

impl fmt::Display for RtEngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(e) => write!(f, "graph error: {e}"),
            Self::MissingProcessor(id) => write!(f, "no processor supplied for node {id:?}"),
            Self::TooManyPorts { node, ports } => write!(
                f,
                "node {node:?} has {ports} ports on one side, over the limit of {MAX_PORTS_PER_NODE}"
            ),
        }
    }
}

impl std::error::Error for RtEngineError {}

/// A compiled graph that can be driven from an audio callback.
///
/// Everything that can be sized ahead of time is allocated in [`RtEngine::compile`]:
/// each node's output buffer, a staging buffer for its inputs, and the routing table
/// resolving every input port to the output that feeds it. [`RtEngine::process`] then
/// only copies samples between buffers that already exist.
///
/// Changing the block size reallocates and therefore must happen off the audio thread
/// (see [`RtEngine::set_block_size`]).
pub struct RtEngine {
    order: Vec<NodeId>,
    index: HashMap<NodeId, usize>,
    /// Per node (in `order`), one buffer whose channels are that node's outputs.
    outputs: Vec<AudioBuffer>,
    /// Per node (in `order`), a buffer staging that node's inputs for the block.
    scratch: Vec<AudioBuffer>,
    /// Per node (in `order`), the processor producing its outputs.
    processors: Vec<Box<dyn Processor>>,
    /// Per node (in `order`), per input port: the `(node index, output port)` feeding it.
    input_sources: Vec<Vec<Option<(usize, usize)>>>,
    /// Optional map from a **document** node id (a host's stable id) to this engine's processor
    /// index, so a host can update a running node's parameters by the id it tracks without knowing
    /// the runtime id. Populated by [`alias_doc_ids`](Self::alias_doc_ids); empty otherwise.
    doc_index: HashMap<NodeId, usize>,
    frames: usize,
}

impl RtEngine {
    /// Compiles `graph` into an engine ready for the audio thread.
    ///
    /// # Errors
    ///
    /// Returns [`RtEngineError::Graph`] if the graph cannot be ordered,
    /// [`RtEngineError::MissingProcessor`] if a node has no processor, and
    /// [`RtEngineError::TooManyPorts`] if a node exceeds [`MAX_PORTS_PER_NODE`].
    pub fn compile(
        graph: &Graph,
        mut processors: HashMap<NodeId, Box<dyn Processor>>,
        frames: usize,
    ) -> Result<Self, RtEngineError> {
        let order = graph.topological_order().map_err(RtEngineError::Graph)?;
        let index: HashMap<NodeId, usize> =
            order.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let mut outputs = Vec::with_capacity(order.len());
        let mut scratch = Vec::with_capacity(order.len());
        let mut procs = Vec::with_capacity(order.len());
        let mut input_sources = Vec::with_capacity(order.len());

        for &id in &order {
            let node = graph.node(id).expect("id from topological order is valid");
            let widest = node.inputs.len().max(node.outputs.len());
            if widest > MAX_PORTS_PER_NODE {
                return Err(RtEngineError::TooManyPorts {
                    node: id,
                    ports: widest,
                });
            }

            outputs.push(AudioBuffer::new(node.outputs.len(), frames));
            scratch.push(AudioBuffer::new(node.inputs.len(), frames));
            procs.push(
                processors
                    .remove(&id)
                    .ok_or(RtEngineError::MissingProcessor(id))?,
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
            scratch,
            processors: procs,
            input_sources,
            doc_index: HashMap::new(),
            frames,
        })
    }

    /// Records aliases from **document** node ids to processor indices, given `remap` (document id
    /// → runtime id, from the builder) — the runtime id is resolved to this engine's index. After
    /// this, [`set_doc_params`](Self::set_doc_params) can address a node by its document id. Ids in
    /// `remap` that this engine does not contain are ignored.
    pub fn alias_doc_ids(&mut self, remap: &HashMap<NodeId, NodeId>) {
        self.doc_index.clear();
        self.doc_index.reserve(remap.len());
        for (&doc_id, &runtime_id) in remap {
            if let Some(&i) = self.index.get(&runtime_id) {
                self.doc_index.insert(doc_id, i);
            }
        }
    }

    /// Processes one block in topological order.
    ///
    /// Performs no heap allocation, takes no locks and never grows a collection, so it is
    /// safe to call from an audio callback.
    pub fn process(&mut self, ctx: &ProcessContext) {
        let Self {
            order,
            outputs,
            scratch,
            processors,
            input_sources,
            ..
        } = self;

        for i in 0..order.len() {
            // Stage this node's inputs into its own buffer so the processor can read them
            // while it writes its outputs. Unconnected inputs read as silence.
            {
                let stage = &mut scratch[i];
                for (port, source) in input_sources[i].iter().enumerate() {
                    let dst = stage
                        .channel_mut(port)
                        .expect("scratch sized from input count");
                    match source {
                        Some((node, out_port)) => {
                            let src = outputs[*node]
                                .channel(*out_port)
                                .expect("routing resolved at compile time");
                            dst.copy_from_slice(src);
                        }
                        None => dst.fill(0.0),
                    }
                }
            }

            let in_count = input_sources[i].len();
            let mut in_slices: [&[f32]; MAX_PORTS_PER_NODE] = [&[]; MAX_PORTS_PER_NODE];
            for (port, slot) in in_slices.iter_mut().enumerate().take(in_count) {
                *slot = scratch[i]
                    .channel(port)
                    .expect("scratch sized from input count");
            }

            let out_count = outputs[i].channels();
            outputs[i].clear();
            let mut out_slices: [&mut [f32]; MAX_PORTS_PER_NODE] =
                std::array::from_fn(|_| &mut [][..]);
            for (slot, channel) in out_slices.iter_mut().zip(outputs[i].channels_mut()) {
                *slot = channel;
            }

            processors[i].process(ctx, &in_slices[..in_count], &mut out_slices[..out_count]);
        }
    }

    /// The block size (frames) this engine is sized for.
    #[must_use]
    pub fn block_size(&self) -> usize {
        self.frames
    }

    /// Resizes every buffer to `frames` and resets all processors.
    ///
    /// This reallocates, so call it from the host's setup path — never from the audio
    /// callback.
    pub fn set_block_size(&mut self, frames: usize) {
        self.frames = frames;
        for buffer in self.outputs.iter_mut().chain(self.scratch.iter_mut()) {
            *buffer = AudioBuffer::new(buffer.channels(), frames);
        }
        self.reset();
    }

    /// Resets every processor's internal state.
    pub fn reset(&mut self) {
        for processor in &mut self.processors {
            processor.reset();
        }
    }

    /// Applies new parameter `values` to the node `id` **in place** (preserving its DSP state),
    /// via [`Processor::set_params`]. Returns `true` if the node exists and applied them, `false`
    /// if the id is unknown or the processor does not support a live update (the caller then
    /// rebuilds to change that node). Allocation-free — safe to call on the audio thread.
    pub fn set_node_params(&mut self, id: NodeId, values: &[f32]) -> bool {
        match self.index.get(&id) {
            Some(&i) => self.processors[i].set_params(values),
            None => false,
        }
    }

    /// Like [`set_node_params`](Self::set_node_params) but addresses the node by its **document**
    /// id (see [`alias_doc_ids`](Self::alias_doc_ids)). Returns `false` if the document id is not
    /// aliased or the processor does not support a live update. Allocation-free.
    pub fn set_doc_params(&mut self, doc_id: NodeId, values: &[f32]) -> bool {
        match self.doc_index.get(&doc_id) {
            Some(&i) => self.processors[i].set_params(values),
            None => false,
        }
    }

    /// Returns the output buffer of `id` after the last [`RtEngine::process`].
    #[must_use]
    pub fn node_output(&self, id: NodeId) -> Option<&AudioBuffer> {
        self.index.get(&id).map(|&i| &self.outputs[i])
    }
}
