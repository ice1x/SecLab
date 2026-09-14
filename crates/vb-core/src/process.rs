//! The node processing contract — the behavioural extension point for DSP nodes.

/// Context passed to a processor for one block of audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessContext {
    /// Sample rate in Hz.
    pub sample_rate: f32,
    /// Number of valid frames (samples per channel) in the block.
    pub frames: usize,
}

/// A DSP node: reads input channel buffers and writes output channel buffers for one
/// block. This is the behavioural side that concrete nodes (in `vb-nodes`) implement;
/// the structural side (ports, wiring) lives in [`crate::node::GraphNode`] and
/// [`crate::graph::Graph`]. Splitting the two keeps the model serializable and the
/// processing trait free of graph concerns (SOLID / dependency inversion).
///
/// `Send` is part of the contract: engines are compiled on an editor thread and handed
/// to the audio thread whole, so every processor must be movable across threads.
pub trait Processor: Send {
    /// Processes one block. `inputs[c]` and `outputs[c]` are channel `c`'s buffers,
    /// each at least `ctx.frames` samples long.
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]);

    /// Resets internal state (filter memory, envelope phase, …). Default: no-op.
    fn reset(&mut self) {}

    /// Applies new parameter `values` (in the node's registry `params()` order) to the running
    /// processor **in place**, preserving DSP state — oscillator phase, filter memory, envelope
    /// stage — so a live knob turn or automation move takes effect without a click.
    ///
    /// Returns `true` if the processor applied the values, `false` (the default) if it does not
    /// support a live update. A caller that gets `false` must fall back to rebuilding the node to
    /// change its parameters. Missing trailing values keep their current field (mirrors how the
    /// registry's `processor_with` tolerates a short list).
    ///
    /// Called on the audio thread, so an implementation must be allocation-free and non-blocking:
    /// write fields and recompute cheap per-sample coefficients only.
    fn set_params(&mut self, _values: &[f32]) -> bool {
        false
    }

    /// Latency in frames the node introduces. Default: none.
    #[must_use]
    fn latency_frames(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessContext, Processor};

    /// Minimal processor used to exercise the contract: output = input * gain.
    struct Gain {
        gain: f32,
    }

    impl Processor for Gain {
        fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
            for (o, x) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
                *o = *x * self.gain;
            }
        }
    }

    #[test]
    fn processor_writes_expected_block() {
        let mut node = Gain { gain: 2.0 };
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: 4,
        };
        let input = [1.0_f32, 2.0, 3.0, 4.0];
        let mut out = [0.0_f32; 4];
        {
            let ins: [&[f32]; 1] = [&input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            node.process(&ctx, &ins, &mut outs);
        }
        assert_eq!(out, [2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn default_latency_is_zero() {
        let node = Gain { gain: 1.0 };
        assert_eq!(node.latency_frames(), 0);
    }
}
