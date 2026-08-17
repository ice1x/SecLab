//! Feeding the graph engine into an audio device callback.
//!
//! The device callback hands us interleaved buffers of arbitrary length, while the
//! engine produces fixed-size blocks. [`BlockFeeder`] bridges the two: it runs the
//! engine block by block into a pre-allocated carry buffer and copies samples out into
//! whatever the callback asks for, duplicating the mono sink across all device
//! channels. Everything it does per callback is allocation-free, and it polls the
//! [`EngineReceiver`] at each block boundary so live graph edits are picked up.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use vb_core::{NodeId, ProcessContext};
use vb_runtime::{EngineReceiver, RtEngine};

use crate::scope::Scope;

/// Lock-free DSP load and xrun counters, shared between the audio and UI threads.
///
/// The audio thread publishes how much of each block's real-time budget the engine
/// consumed (smoothed) and counts blocks that blew the budget entirely (xruns). The
/// UI thread only reads. No locks, no allocation on the publishing side.
#[derive(Clone, Default)]
pub struct LoadMonitor {
    inner: Arc<LoadShared>,
}

#[derive(Default)]
struct LoadShared {
    /// Smoothed load in permille of the block budget (1000 = exactly real time).
    load_permille: AtomicU32,
    /// Blocks whose processing took longer than their real-time budget.
    xruns: AtomicU32,
}

impl LoadMonitor {
    /// Creates a monitor with zeroed counters.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Smoothed DSP load as a fraction of the block budget (1.0 = exactly real time).
    #[must_use]
    pub fn load(&self) -> f32 {
        self.inner.load_permille.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Number of blocks that overran their real-time budget.
    #[must_use]
    pub fn xruns(&self) -> u32 {
        self.inner.xruns.load(Ordering::Relaxed)
    }

    /// Publishes one block's measurement (audio-thread side, allocation-free).
    fn publish(&self, smoothed_fraction: f32, overran: bool) {
        let permille = (smoothed_fraction * 1000.0).clamp(0.0, u32::MAX as f32) as u32;
        self.inner.load_permille.store(permille, Ordering::Relaxed);
        if overran {
            self.inner.xruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

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
    /// Load/xrun reporting, if attached.
    monitor: Option<LoadMonitor>,
    /// Exponentially smoothed load fraction (audio-thread local).
    smoothed_load: f32,
    /// Visualization tap, if attached.
    scope: Option<Scope>,
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
            monitor: None,
            smoothed_load: 0.0,
            scope: None,
        }
    }

    /// Attaches a load/xrun monitor; measurements start with the next block.
    #[must_use]
    pub fn with_monitor(mut self, monitor: LoadMonitor) -> Self {
        self.monitor = Some(monitor);
        self
    }

    /// Attaches a visualization tap; the played signal is mirrored into it.
    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = Some(scope);
        self
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
        let started = self.monitor.is_some().then(Instant::now);
        self.engine.process(&self.ctx);
        if let (Some(monitor), Some(started)) = (&self.monitor, started) {
            let budget_s = self.ctx.frames as f32 / self.ctx.sample_rate.max(1.0);
            let fraction = started.elapsed().as_secs_f32() / budget_s;
            // Light smoothing so the meter reads steadily instead of flickering.
            self.smoothed_load = 0.9 * self.smoothed_load + 0.1 * fraction;
            monitor.publish(self.smoothed_load, fraction > 1.0);
        }
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
        if let Some(scope) = &self.scope {
            scope.push(&self.carry);
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

    /// Burns wall-clock time each block to overrun the real-time budget.
    struct Slow(std::time::Duration);
    impl Processor for Slow {
        fn process(&mut self, _ctx: &ProcessContext, _in: &[&[f32]], out: &mut [&mut [f32]]) {
            std::thread::sleep(self.0);
            out[0].fill(0.5);
        }
    }

    #[test]
    fn monitor_reads_zero_before_any_block() {
        let monitor = LoadMonitor::new();
        assert_eq!(monitor.load(), 0.0);
        assert_eq!(monitor.xruns(), 0);
    }

    #[test]
    fn fast_blocks_report_low_load_and_no_xruns() {
        let (engine, sink) = engine_with(Box::new(Constant(1.0)), 64);
        let monitor = LoadMonitor::new();
        let mut feeder =
            BlockFeeder::new(engine, None, sink, 48_000.0).with_monitor(monitor.clone());

        let mut data = [0.0_f32; 256];
        feeder.fill(&mut data, 1); // 4 blocks of 64 frames

        // A constant-fill engine is far faster than real time: 64 frames @ 48 kHz
        // is a 1.33 ms budget.
        assert!(
            monitor.load() >= 0.0 && monitor.load() < 1.0,
            "{}",
            monitor.load()
        );
        assert_eq!(monitor.xruns(), 0);
    }

    #[test]
    fn overrunning_blocks_count_as_xruns() {
        // 64 frames @ 48 kHz = 1.33 ms budget; each block burns ~10 ms.
        let (engine, sink) = engine_with(Box::new(Slow(std::time::Duration::from_millis(10))), 64);
        let monitor = LoadMonitor::new();
        let mut feeder =
            BlockFeeder::new(engine, None, sink, 48_000.0).with_monitor(monitor.clone());

        let mut data = [0.0_f32; 192];
        feeder.fill(&mut data, 1); // 3 blocks, each overruns

        assert_eq!(monitor.xruns(), 3);
        assert!(monitor.load() > 1.0, "{}", monitor.load());
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
