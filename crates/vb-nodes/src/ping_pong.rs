//! Ping-pong delay: a stereo echo that bounces between the left and right channels.
//!
//! Takes a mono input and produces a **stereo** pair of outputs. Two cross-coupled delay
//! lines feed each other's feedback, so the first echo lands on the left, the next on the
//! right, the next on the left again — a widening call-and-response that a plain mono
//! [`crate::FbDelay`] can't make. The dry signal sits centred on both channels; `mix` sets
//! how much of the bouncing wet path is heard. Now that the engine carries a true two-channel
//! sink, ending a patch here (or feeding the pair through [`crate::Pan`]/downstream) gives a
//! genuinely stereo delay.
//!
//! Both ring buffers are allocated once in the constructor, sized for the maximum delay at
//! the highest supported sample rate — `process()` never allocates.

use vb_core::{ProcessContext, Processor};

/// Longest selectable delay, in seconds.
pub const MAX_DELAY_S: f32 = 2.0;
/// Highest sample rate the pre-allocated rings cover.
const MAX_SR: f32 = 192_000.0;

/// A stereo ping-pong delay: mono in, left/right out, echoes alternating across the field.
pub struct PingPong {
    delay_s: f32,
    feedback: f32,
    mix: f32,
    ring_l: Vec<f32>,
    ring_r: Vec<f32>,
    write: usize,
}

impl PingPong {
    /// Creates a ping-pong delay. `delay_s` is clamped to `[0, MAX_DELAY_S]`, `feedback` to
    /// `[0, 0.95]`, `mix` (dry↔wet) to `[0, 1]`.
    #[must_use]
    pub fn new(delay_s: f32, feedback: f32, mix: f32) -> Self {
        let cap = (MAX_DELAY_S * MAX_SR) as usize + 1;
        Self {
            delay_s: delay_s.clamp(0.0, MAX_DELAY_S),
            feedback: feedback.clamp(0.0, 0.95),
            mix: mix.clamp(0.0, 1.0),
            ring_l: vec![0.0; cap],
            ring_r: vec![0.0; cap],
            write: 0,
        }
    }

    /// Delay length in samples at `sample_rate`, at least 1, within the ring.
    fn delay_samples(&self, sample_rate: f32) -> usize {
        let n = (self.delay_s * sample_rate).round() as usize;
        n.clamp(1, self.ring_l.len() - 1)
    }
}

impl Processor for PingPong {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        let (left, rest) = outputs.split_at_mut(1);
        let left = &mut left[0];
        let right = match rest.first_mut() {
            Some(r) => r,
            None => return,
        };
        let len = self.ring_l.len();
        let delay = self.delay_samples(ctx.sample_rate.max(1.0));
        let (dry, wet) = (1.0 - self.mix, self.mix);
        for i in 0..ctx.frames {
            let x = input.get(i).copied().unwrap_or(0.0);
            let read = (self.write + len - delay) % len;
            let dl = self.ring_l[read];
            let dr = self.ring_r[read];
            // Cross-couple: input enters the left line; the left echo bounces into the
            // right line, the right echo back into the left — L, R, L, R…
            self.ring_l[self.write] = self.feedback.mul_add(dr, x);
            self.ring_r[self.write] = self.feedback * dl;
            if let Some(o) = left.get_mut(i) {
                *o = dry * x + wet * dl;
            }
            if let Some(o) = right.get_mut(i) {
                *o = dry * x + wet * dr;
            }
            self.write = (self.write + 1) % len;
        }
    }

    fn reset(&mut self) {
        self.ring_l.fill(0.0);
        self.ring_r.fill(0.0);
        self.write = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(pp: &mut PingPong, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut l = vec![0.0_f32; input.len()];
        let mut r = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            pp.process(&ctx, &ins, &mut outs);
        }
        (l, r)
    }

    #[test]
    fn echoes_alternate_between_left_and_right() {
        // 1 ms = 48 samples, feedback 0.5, fully wet.
        let mut pp = PingPong::new(0.001, 0.5, 1.0);
        let mut input = vec![0.0_f32; 300];
        input[0] = 1.0;
        let (l, r) = run(&mut pp, &input);
        // First echo lands left at D, second right at 2D, third left at 3D, fourth right 4D.
        assert!(
            (l[48] - 1.0).abs() < 1e-6,
            "first echo on the left: {}",
            l[48]
        );
        assert!(r[48].abs() < 1e-6, "nothing on the right yet");
        assert!(
            (r[96] - 0.5).abs() < 1e-6,
            "second echo on the right: {}",
            r[96]
        );
        assert!(l[96].abs() < 1e-6, "left is quiet at 2D");
        assert!((l[144] - 0.25).abs() < 1e-6, "third echo back on the left");
        assert!((r[192] - 0.125).abs() < 1e-6, "fourth echo on the right");
    }

    #[test]
    fn fully_dry_is_the_mono_input_on_both_channels() {
        let mut pp = PingPong::new(0.001, 0.7, 0.0);
        let mut input = vec![0.0_f32; 200];
        input[0] = 1.0;
        input[10] = -0.5;
        let (l, r) = run(&mut pp, &input);
        for i in 0..input.len() {
            assert!((l[i] - input[i]).abs() < 1e-6, "dry left == input at {i}");
            assert!((r[i] - input[i]).abs() < 1e-6, "dry right == input at {i}");
        }
    }

    #[test]
    fn the_wet_echoes_decay_with_feedback() {
        let mut pp = PingPong::new(0.001, 0.5, 1.0);
        let mut input = vec![0.0_f32; 400];
        input[0] = 1.0;
        let (l, r) = run(&mut pp, &input);
        // Peak magnitudes L,R,L,R,… shrink by the feedback each bounce.
        assert!(l[48] > r[96] && r[96] > l[144], "each bounce quieter");
        assert!(l[144] > r[192], "monotone decay continues");
    }

    #[test]
    fn parameters_are_clamped() {
        let pp = PingPong::new(99.0, 5.0, 9.0);
        assert!((pp.delay_s - MAX_DELAY_S).abs() < 1e-6);
        assert!((pp.feedback - 0.95).abs() < 1e-6);
        assert!((pp.mix - 1.0).abs() < 1e-6);
    }

    #[test]
    fn reset_clears_both_lines() {
        let mut pp = PingPong::new(0.001, 0.6, 1.0);
        let mut input = vec![0.0_f32; 100];
        input[0] = 1.0;
        run(&mut pp, &input);
        pp.reset();
        let (l, r) = run(&mut pp, &vec![0.0_f32; 200]);
        assert!(
            l.iter().chain(r.iter()).all(|&s| s == 0.0),
            "no stale echoes"
        );
    }
}
