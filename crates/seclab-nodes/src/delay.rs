//! Feedback delay: an echo line that is also the graph's legal feedback boundary.
//!
//! The node's [`seclab_core::GraphNode`] is created with `feedback_node`, so wiring a
//! loop through its input is allowed — the loop then carries one block of latency by
//! construction. Internally it is a classic circular delay line with a feedback
//! coefficient, so it echoes on its own even without an external loop.
//!
//! The ring buffer is allocated once in the constructor (engine builds happen on the
//! editor thread), sized for the maximum delay at the highest supported sample rate —
//! `process()` never allocates.

use seclab_core::{ProcessContext, Processor};

/// Longest selectable delay, in seconds.
pub const MAX_DELAY_S: f32 = 2.0;
/// Highest sample rate the pre-allocated ring covers.
const MAX_SR: f32 = 192_000.0;

/// Circular delay line with internal feedback.
pub struct FbDelay {
    delay_s: f32,
    feedback: f32,
    ring: Vec<f32>,
    write: usize,
}

impl FbDelay {
    /// Creates a delay of `delay_s` seconds with `feedback` in [0, 0.95].
    #[must_use]
    pub fn new(delay_s: f32, feedback: f32) -> Self {
        Self {
            delay_s: delay_s.clamp(0.0, MAX_DELAY_S),
            feedback: feedback.clamp(0.0, 0.95),
            ring: vec![0.0; (MAX_DELAY_S * MAX_SR) as usize + 1],
            write: 0,
        }
    }

    /// Delay length in samples at `sample_rate`, at least 1, within the ring.
    fn delay_samples(&self, sample_rate: f32) -> usize {
        let n = (self.delay_s * sample_rate).round() as usize;
        n.clamp(1, self.ring.len() - 1)
    }
}

impl Processor for FbDelay {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (Some(input), Some(out)) = (inputs.first(), outputs.first_mut()) else {
            return;
        };
        let len = self.ring.len();
        let delay = self.delay_samples(ctx.sample_rate.max(1.0));
        for i in 0..ctx.frames {
            let read = (self.write + len - delay) % len;
            let delayed = self.ring[read];
            out[i] = delayed;
            self.ring[self.write] = self.feedback.mul_add(delayed, input[i]);
            self.write = (self.write + 1) % len;
        }
    }

    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.write = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(delay: &mut FbDelay, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0; input.len()];
        delay.process(&ctx, &[input], &mut [&mut out]);
        out
    }

    #[test]
    fn impulse_comes_back_after_the_delay_time() {
        // 10 ms at 48 kHz = 480 samples.
        let mut d = FbDelay::new(0.01, 0.0);
        let mut input = vec![0.0_f32; 1_000];
        input[0] = 1.0;
        let out = run(&mut d, &input);
        assert_eq!(out[480], 1.0, "the echo lands exactly at 480 samples");
        let nonzero = out.iter().filter(|&&s| s != 0.0).count();
        assert_eq!(nonzero, 1, "no feedback: exactly one echo");
    }

    #[test]
    fn feedback_produces_decaying_echoes() {
        let mut d = FbDelay::new(0.001, 0.5); // 48 samples
        let mut input = vec![0.0_f32; 300];
        input[0] = 1.0;
        let out = run(&mut d, &input);
        assert!((out[48] - 1.0).abs() < 1e-6, "first echo");
        assert!((out[96] - 0.5).abs() < 1e-6, "second echo at half");
        assert!((out[144] - 0.25).abs() < 1e-6, "third echo at a quarter");
    }

    #[test]
    fn zero_delay_still_has_one_sample_of_latency() {
        // The point of the node: even at delay 0 the loop is never instantaneous.
        let mut d = FbDelay::new(0.0, 0.0);
        let input = [1.0_f32, 0.0, 0.0, 0.0];
        let out = run(&mut d, &input);
        assert_eq!(out, [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn reset_clears_the_history() {
        let mut d = FbDelay::new(0.001, 0.5);
        let mut input = vec![0.0_f32; 100];
        input[0] = 1.0;
        run(&mut d, &input);

        d.reset();
        let silence = run(&mut d, &vec![0.0_f32; 200]);
        assert!(silence.iter().all(|&s| s == 0.0), "no stale echoes");
    }
}
