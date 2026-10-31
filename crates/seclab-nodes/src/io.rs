//! Plugin audio I/O nodes: the boundary where the host's audio enters and leaves the
//! graph.
//!
//! An [`AudioBus`] is the shared exchange between the host and the graph for one block.
//! The host publishes its incoming audio into the bus and drains the graph's result
//! from it; inside the graph an [`AudioInput`] node reads a bus channel as its output
//! and an [`AudioOutput`] node writes a bus channel from its input. Host-fill and
//! graph-process run in sequence on the same audio thread, so — like [`crate::NoteSource`]
//! — samples are stored as atomics purely to make the handle `Send`/`Sync` and cheap to
//! clone into every rebuilt engine, not to synchronize two threads.
//!
//! The default bus has zero capacity: reads return silence and writes are dropped, so a
//! graph with I/O nodes still runs (silently) before any host has wired real buffers in.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use seclab_core::{ProcessContext, Processor};

/// Number of audio channels the plugin I/O boundary carries (stereo).
pub const MAX_IO_CHANNELS: usize = 2;

/// Shared host↔graph audio exchange for one block, clonable like a mailbox handle.
#[derive(Clone, Default)]
pub struct AudioBus {
    inner: Arc<BusShared>,
}

struct BusShared {
    channels: usize,
    /// Frames per channel each buffer can hold.
    capacity: usize,
    /// Host → graph samples, `channels * capacity` bit patterns, channel-major.
    input: Vec<AtomicU32>,
    /// Graph → host samples, same layout.
    output: Vec<AtomicU32>,
    /// Frames the host last published (informational).
    frames: AtomicUsize,
}

impl Default for BusShared {
    fn default() -> Self {
        Self::sized(MAX_IO_CHANNELS, 0)
    }
}

impl BusShared {
    fn sized(channels: usize, capacity: usize) -> Self {
        let cells = channels * capacity;
        Self {
            channels,
            capacity,
            input: (0..cells).map(|_| AtomicU32::new(0)).collect(),
            output: (0..cells).map(|_| AtomicU32::new(0)).collect(),
            frames: AtomicUsize::new(0),
        }
    }

    /// The cell for `(channel, frame)` in `base`, or `None` when out of range.
    fn slot<'a>(
        &self,
        base: &'a [AtomicU32],
        channel: usize,
        frame: usize,
    ) -> Option<&'a AtomicU32> {
        if channel >= self.channels || frame >= self.capacity {
            return None;
        }
        base.get(channel * self.capacity + frame)
    }
}

impl AudioBus {
    /// Creates a bus holding up to `capacity` frames on [`MAX_IO_CHANNELS`] channels.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(BusShared::sized(MAX_IO_CHANNELS, capacity)),
        }
    }

    /// Channels the bus carries.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.inner.channels
    }

    /// Frames per channel the bus can hold.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }

    /// Host side: publishes the block's incoming audio (one slice per channel).
    /// Extra channels/frames beyond the bus's size are ignored.
    pub fn publish_input(&self, inputs: &[&[f32]], frames: usize) {
        let bus = &self.inner;
        bus.frames.store(frames, Ordering::Release);
        for (channel, buf) in inputs.iter().enumerate().take(bus.channels) {
            for (frame, &v) in buf.iter().enumerate().take(frames) {
                if let Some(cell) = bus.slot(&bus.input, channel, frame) {
                    cell.store(v.to_bits(), Ordering::Relaxed);
                }
            }
        }
    }

    /// Host side: drains the graph's result into `outputs` (one slice per channel).
    /// Channels/frames beyond the bus's size read as silence.
    pub fn collect_output(&self, outputs: &mut [&mut [f32]]) {
        let bus = &self.inner;
        for (channel, buf) in outputs.iter_mut().enumerate() {
            for (frame, slot) in buf.iter_mut().enumerate() {
                *slot = match bus.slot(&bus.output, channel, frame) {
                    Some(cell) => f32::from_bits(cell.load(Ordering::Relaxed)),
                    None => 0.0,
                };
            }
        }
    }

    /// Graph side: reads a bus input `channel` into `out`, padding with silence past the
    /// stored frames.
    fn read_input(&self, channel: usize, out: &mut [f32]) {
        let bus = &self.inner;
        for (frame, slot) in out.iter_mut().enumerate() {
            *slot = match bus.slot(&bus.input, channel, frame) {
                Some(cell) => f32::from_bits(cell.load(Ordering::Relaxed)),
                None => 0.0,
            };
        }
    }

    /// Graph side: writes `samples` into bus output `channel` (out-of-range dropped).
    fn write_output(&self, channel: usize, samples: &[f32]) {
        let bus = &self.inner;
        for (frame, &v) in samples.iter().enumerate() {
            if let Some(cell) = bus.slot(&bus.output, channel, frame) {
                cell.store(v.to_bits(), Ordering::Relaxed);
            }
        }
    }
}

/// A plugin audio-input node: emits one [`AudioBus`] input channel as its output.
pub struct AudioInput {
    bus: AudioBus,
    channel: usize,
}

impl AudioInput {
    /// Reads `channel` of `bus`.
    #[must_use]
    pub fn new(bus: AudioBus, channel: usize) -> Self {
        Self { bus, channel }
    }
}

impl Processor for AudioInput {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        if let [out, ..] = outputs {
            self.bus.read_input(self.channel, &mut out[..ctx.frames]);
        }
    }
}

/// A plugin audio-output node: writes its input into one [`AudioBus`] output channel and
/// passes it through so the node stays tappable and chainable.
pub struct AudioOutput {
    bus: AudioBus,
    channel: usize,
}

impl AudioOutput {
    /// Writes `channel` of `bus`.
    #[must_use]
    pub fn new(bus: AudioBus, channel: usize) -> Self {
        Self { bus, channel }
    }
}

impl Processor for AudioOutput {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let src = inputs
            .first()
            .map_or(&[][..], |s| &s[..ctx.frames.min(s.len())]);
        self.bus.write_output(self.channel, src);
        if let [out, ..] = outputs {
            let n = src.len().min(out.len());
            let tail = ctx.frames.min(out.len());
            out[..n].copy_from_slice(&src[..n]);
            out[n..tail].fill(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(frames: usize) -> ProcessContext {
        ProcessContext {
            sample_rate: 48_000.0,
            frames,
        }
    }

    #[test]
    fn input_node_emits_the_hosts_published_channel() {
        let bus = AudioBus::new(4);
        // Host publishes distinct L/R blocks.
        let left = [1.0, 2.0, 3.0, 4.0];
        let right = [-1.0, -2.0, -3.0, -4.0];
        bus.publish_input(&[&left, &right], 4);

        let mut node_l = AudioInput::new(bus.clone(), 0);
        let mut node_r = AudioInput::new(bus.clone(), 1);
        let mut out = [0.0; 4];
        node_l.process(&ctx(4), &[], &mut [&mut out]);
        assert_eq!(out, left);
        node_r.process(&ctx(4), &[], &mut [&mut out]);
        assert_eq!(out, right);
    }

    #[test]
    fn output_node_writes_the_bus_and_passes_through() {
        let bus = AudioBus::new(4);
        let mut node = AudioOutput::new(bus.clone(), 1);
        let signal = [0.5, -0.5, 0.25, -0.25];
        let mut out = [9.0; 4];
        node.process(&ctx(4), &[&signal], &mut [&mut out]);

        // Pass-through: the output port mirrors the input.
        assert_eq!(out, signal);
        // And the host reads it back from channel 1 (channel 0 stays silent).
        let (mut l, mut r) = ([9.0; 4], [9.0; 4]);
        bus.collect_output(&mut [&mut l, &mut r]);
        assert_eq!(l, [0.0; 4], "nothing was written to channel 0");
        assert_eq!(r, signal);
    }

    #[test]
    fn input_reads_and_output_writes_round_trip_through_the_bus() {
        // A one-node "effect": read the host input, gain it, write it back.
        let bus = AudioBus::new(8);
        let host_in = [0.2, 0.4, 0.6, 0.8];
        bus.publish_input(&[&host_in], 4);

        let mut input = AudioInput::new(bus.clone(), 0);
        let mut output = AudioOutput::new(bus.clone(), 0);
        let mut mid = [0.0; 4];
        input.process(&ctx(4), &[], &mut [&mut mid]);
        for s in &mut mid {
            *s *= 2.0;
        }
        let mut passthru = [0.0; 4];
        output.process(&ctx(4), &[&mid], &mut [&mut passthru]);

        let mut drained = [0.0; 4];
        bus.collect_output(&mut [&mut drained]);
        assert_eq!(drained, [0.4, 0.8, 1.2, 1.6]);
    }

    #[test]
    fn the_default_bus_is_silent_and_absorbs_writes() {
        let bus = AudioBus::default();
        assert_eq!(bus.capacity(), 0);
        assert_eq!(bus.channels(), MAX_IO_CHANNELS);

        let mut input = AudioInput::new(bus.clone(), 0);
        let mut out = [7.0; 4];
        input.process(&ctx(4), &[], &mut [&mut out]);
        assert_eq!(out, [0.0; 4], "no capacity → silence");

        // Writing to a zero-capacity bus must not panic.
        let mut output = AudioOutput::new(bus.clone(), 0);
        let signal = [1.0, 2.0, 3.0, 4.0];
        let mut passthru = [0.0; 4];
        output.process(&ctx(4), &[&signal], &mut [&mut passthru]);
        assert_eq!(
            passthru, signal,
            "pass-through still works with no bus capacity"
        );
    }

    #[test]
    fn out_of_range_channels_are_ignored() {
        let bus = AudioBus::new(4);
        // Channel index at/above MAX_IO_CHANNELS reads silence and drops writes.
        let mut high_in = AudioInput::new(bus.clone(), MAX_IO_CHANNELS);
        let mut out = [3.0; 4];
        high_in.process(&ctx(4), &[], &mut [&mut out]);
        assert_eq!(out, [0.0; 4]);

        let mut high_out = AudioOutput::new(bus.clone(), MAX_IO_CHANNELS);
        high_out.process(&ctx(4), &[&[1.0, 1.0, 1.0, 1.0]], &mut [&mut [0.0; 4][..]]);
        let (mut l, mut r) = ([0.0; 4], [0.0; 4]);
        bus.collect_output(&mut [&mut l, &mut r]);
        assert_eq!(
            (l, r),
            ([0.0; 4], [0.0; 4]),
            "nothing landed on a real channel"
        );
    }
}
