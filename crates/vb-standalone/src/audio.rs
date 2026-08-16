//! Feeding the graph engine into an audio device callback.
//!
//! The device callback hands us interleaved buffers of arbitrary length, while the
//! engine produces fixed-size blocks. [`BlockFeeder`] bridges the two: it runs the
//! engine block by block into a pre-allocated carry buffer and copies samples out into
//! whatever the callback asks for, duplicating the mono sink across all device
//! channels. Everything it does per callback is allocation-free, and it polls the
//! [`EngineReceiver`] at each block boundary so live graph edits are picked up.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use vb_core::{NodeId, ProcessContext};
use vb_runtime::{EngineReceiver, RtEngine};

/// Shared, lock-free handle to the node the feeder plays.
///
/// The editor thread points it at a new sink when a rebuilt engine is installed; the
/// audio thread reads it at each block boundary. `None` (no terminal audio node in
/// the patch) plays silence.
#[derive(Clone)]
pub struct SinkHandle(Arc<AtomicU64>);

/// Sentinel for "no sink": patches whose ids reach this are beyond any real session.
const NO_SINK: u64 = u64::MAX;

impl SinkHandle {
    /// Creates a handle, initially pointing at `id`.
    #[must_use]
    pub fn new(id: Option<NodeId>) -> Self {
        Self(Arc::new(AtomicU64::new(id.map_or(NO_SINK, |n| n.0))))
    }

    /// Retargets the feeder to `id` (or silence for `None`).
    pub fn set(&self, id: Option<NodeId>) {
        self.0.store(id.map_or(NO_SINK, |n| n.0), Ordering::Release);
    }

    /// The current sink, if any.
    #[must_use]
    pub fn get(&self) -> Option<NodeId> {
        match self.0.load(Ordering::Acquire) {
            NO_SINK => None,
            id => Some(NodeId(id)),
        }
    }
}

impl From<NodeId> for SinkHandle {
    fn from(id: NodeId) -> Self {
        Self::new(Some(id))
    }
}

/// Pulls fixed-size engine blocks and serves arbitrary-size interleaved requests.
pub struct BlockFeeder {
    engine: RtEngine,
    receiver: Option<EngineReceiver>,
    /// The node whose channel 0 is played (retargetable from the editor thread).
    sink: SinkHandle,
    ctx: ProcessContext,
    /// One engine block of sink samples, refilled when exhausted. Allocated once here;
    /// the callback path only copies.
    carry: Vec<f32>,
    /// Next unread index into `carry`; `carry.len()` means "empty, refill".
    carry_pos: usize,
}

impl BlockFeeder {
    /// Creates a feeder that plays `sink`'s channel 0 at `sample_rate`.
    ///
    /// `receiver` is optional so the feeder also serves fixed setups (tests, offline
    /// rendering) that never swap engines.
    #[must_use]
    pub fn new(
        engine: RtEngine,
        receiver: Option<EngineReceiver>,
        sink: impl Into<SinkHandle>,
        sample_rate: f32,
    ) -> Self {
        let frames = engine.block_size();
        Self {
            engine,
            receiver,
            sink: sink.into(),
            ctx: ProcessContext {
                sample_rate,
                frames,
            },
            carry: vec![0.0; frames],
            carry_pos: frames, // start empty
        }
    }

    /// Runs one engine block into the carry buffer.
    ///
    /// Installed engines are expected to share this feeder's block size; if one arrives
    /// with a different size the feeder must not panic (we are inside the audio
    /// callback), so it copies what overlaps and pads the rest with silence.
    fn refill(&mut self) {
        if let Some(rx) = &self.receiver {
            rx.poll_swap(&mut self.engine);
        }
        self.engine.process(&self.ctx);
        match self
            .sink
            .get()
            .and_then(|sink| self.engine.node_output(sink))
            .and_then(|b| b.channel(0))
        {
            Some(sink) => {
                let n = sink.len().min(self.carry.len());
                self.carry[..n].copy_from_slice(&sink[..n]);
                self.carry[n..].fill(0.0);
            }
            // The sink vanished after a swap (edited away): play silence over a click.
            None => self.carry.fill(0.0),
        }
        self.carry_pos = 0;
    }

    /// Fills an interleaved output buffer with `channels` channels.
    ///
    /// Allocation-free: only copies from the carry buffer, refilling it block by block.
    pub fn fill(&mut self, data: &mut [f32], channels: usize) {
        if channels == 0 {
            return;
        }
        for frame in data.chunks_mut(channels) {
            if self.carry_pos >= self.carry.len() {
                self.refill();
            }
            let sample = self.carry[self.carry_pos];
            self.carry_pos += 1;
            frame.fill(sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use vb_core::{Graph, GraphNode, Port, Processor, SignalType};
    use vb_runtime::engine_swap;

    use super::*;

    /// Emits an increasing ramp: 0, 1, 2, … across blocks (stateful).
    struct Ramp(f32);
    impl Processor for Ramp {
        fn process(&mut self, ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
            for o in out[0].iter_mut().take(ctx.frames) {
                *o = self.0;
                self.0 += 1.0;
            }
        }
    }

    struct Constant(f32);
    impl Processor for Constant {
        fn process(&mut self, _ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
            out[0].fill(self.0);
        }
    }

    fn engine_with(proc_: Box<dyn Processor>, frames: usize) -> (RtEngine, NodeId) {
        let mut g = Graph::new();
        let src = g.add_node(GraphNode::new(
            "src",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
        procs.insert(src, proc_);
        (RtEngine::compile(&g, procs, frames).unwrap(), src)
    }

    #[test]
    fn fill_is_continuous_across_mismatched_sizes() {
        // Engine blocks of 4, callback asks for 6 frames at a time: the ramp must stay
        // gapless across the block boundary.
        let (engine, sink) = engine_with(Box::new(Ramp(0.0)), 4);
        let mut feeder = BlockFeeder::new(engine, None, sink, 48_000.0);

        let mut first = [0.0_f32; 6];
        let mut second = [0.0_f32; 6];
        feeder.fill(&mut first, 1);
        feeder.fill(&mut second, 1);

        assert_eq!(first, [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(second, [6.0, 7.0, 8.0, 9.0, 10.0, 11.0]);
    }

    #[test]
    fn fill_duplicates_mono_across_channels() {
        let (engine, sink) = engine_with(Box::new(Ramp(0.0)), 4);
        let mut feeder = BlockFeeder::new(engine, None, sink, 48_000.0);

        // Stereo interleaved: L R L R …
        let mut data = [0.0_f32; 8];
        feeder.fill(&mut data, 2);
        assert_eq!(data, [0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
    }

    #[test]
    fn live_swap_is_picked_up_at_a_block_boundary() {
        let (engine, sink) = engine_with(Box::new(Constant(1.0)), 4);
        let (tx, rx) = engine_swap();
        let mut feeder = BlockFeeder::new(engine, Some(rx), sink, 48_000.0);

        let mut before = [0.0_f32; 4];
        feeder.fill(&mut before, 1);
        assert_eq!(before, [1.0; 4]);

        let (replacement, _) = engine_with(Box::new(Constant(2.0)), 4);
        tx.install(replacement);

        let mut after = [0.0_f32; 4];
        feeder.fill(&mut after, 1);
        assert_eq!(after, [2.0; 4], "swap lands at the next block boundary");
        assert!(tx.collect());
    }

    #[test]
    fn sink_handle_retargets_between_blocks() {
        let (engine, sink) = engine_with(Box::new(Constant(1.0)), 4);
        let handle = SinkHandle::new(Some(sink));
        let mut feeder = BlockFeeder::new(engine, None, handle.clone(), 48_000.0);

        let mut on = [0.0_f32; 4];
        feeder.fill(&mut on, 1);
        assert_eq!(on, [1.0; 4]);

        // Point the handle at nothing: the next block is silence, not a panic.
        handle.set(None);
        let mut off = [9.0_f32; 4];
        feeder.fill(&mut off, 1);
        assert_eq!(off, [0.0; 4]);

        handle.set(Some(sink));
        let mut back = [0.0_f32; 4];
        feeder.fill(&mut back, 1);
        assert_eq!(back, [1.0; 4]);
    }

    #[test]
    fn zero_channels_is_a_no_op() {
        let (engine, sink) = engine_with(Box::new(Ramp(0.0)), 4);
        let mut feeder = BlockFeeder::new(engine, None, sink, 48_000.0);
        feeder.fill(&mut [], 0);
    }

    #[test]
    fn mismatched_block_size_swap_degrades_to_silence_not_panic() {
        let (engine, sink) = engine_with(Box::new(Constant(1.0)), 4);
        let (tx, rx) = engine_swap();
        let mut feeder = BlockFeeder::new(engine, Some(rx), sink, 48_000.0);

        // A misbehaving editor installs an engine with a *smaller* block size.
        let (short_engine, _) = engine_with(Box::new(Constant(2.0)), 2);
        tx.install(short_engine);

        let mut data = [9.0_f32; 4];
        feeder.fill(&mut data, 1);
        // The overlapping half carries signal, the tail pads with silence — no panic.
        assert_eq!(&data[..2], &[2.0, 2.0]);
        assert_eq!(&data[2..], &[0.0, 0.0]);
        assert!(tx.collect());
    }
}
