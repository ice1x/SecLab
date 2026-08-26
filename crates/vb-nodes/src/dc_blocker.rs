//! DC blocker: a one-pole/one-zero high-pass that strips any constant offset.
//!
//! Rectifiers, wavefolders and asymmetric waveshapers leave a DC component on the signal
//! that wastes headroom and thumps speakers. This node removes it with the classic
//! `y[n] = x[n] - x[n-1] + R * y[n-1]` difference equation: the zero at DC kills the
//! offset while `R` (just below 1) places the corner a few Hz up, so the audible band
//! passes essentially untouched.

use vb_core::{ProcessContext, Processor};

/// A DC-removing high-pass filter.
///
/// Input 0 is audio, output 0 is the same signal with its DC offset removed. `r` sets the
/// pole radius: closer to `1.0` lowers the corner frequency (more low end preserved).
#[derive(Debug, Clone)]
pub struct DcBlocker {
    /// Pole radius, clamped to `0.0..=0.9999`.
    r: f32,
    /// Previous input sample `x[n-1]`.
    prev_in: f32,
    /// Previous output sample `y[n-1]`.
    prev_out: f32,
}

impl DcBlocker {
    /// Creates a DC blocker with pole radius `r` (clamped to `0.0..=0.9999`).
    #[must_use]
    pub fn new(r: f32) -> Self {
        Self {
            r: r.clamp(0.0, 0.9999),
            prev_in: 0.0,
            prev_out: 0.0,
        }
    }
}

impl Processor for DcBlocker {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (Some(input), Some(out)) = (inputs.first(), outputs.first_mut()) else {
            return;
        };
        for (o, &x) in out.iter_mut().zip(input.iter()).take(ctx.frames) {
            let y = x - self.prev_in + self.r * self.prev_out;
            self.prev_in = x;
            self.prev_out = y;
            *o = y;
        }
    }

    fn reset(&mut self) {
        self.prev_in = 0.0;
        self.prev_out = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const SR: f32 = 48_000.0;

    fn run(b: &mut DcBlocker, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            b.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn rms(xs: &[f32]) -> f32 {
        (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt()
    }

    #[test]
    fn a_constant_offset_decays_to_zero() {
        let mut b = DcBlocker::new(0.995);
        let out = run(&mut b, &[1.0; 4_000]);
        // The steady-state output of a pure DC input is 0; the tail should be tiny.
        assert!(out[0] > 0.5, "the first sample still passes the step edge");
        assert!(
            out.last().unwrap().abs() < 1e-3,
            "DC removed: {}",
            out.last().unwrap()
        );
    }

    #[test]
    fn it_removes_a_dc_offset_from_a_sine_but_keeps_the_tone() {
        let freq = 220.0;
        let n = 4_800;
        let offset = 0.5;
        let input: Vec<f32> = (0..n)
            .map(|i| offset + 0.5 * (TAU * freq * i as f32 / SR).sin())
            .collect();
        let mut b = DcBlocker::new(0.995);
        let out = run(&mut b, &input);
        // Mean (DC) of the second half is ~0 after the transient settles.
        let tail = &out[n / 2..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 0.02, "DC removed from sine: mean {mean}");
        // The AC energy of the tone is preserved (220 Hz is well above the corner).
        assert!(rms(tail) > 0.3, "tone preserved: rms {}", rms(tail));
    }

    #[test]
    fn r_is_clamped_below_one() {
        assert!((DcBlocker::new(2.0).r - 0.9999).abs() < 1e-6);
        assert_eq!(DcBlocker::new(-1.0).r, 0.0);
    }

    #[test]
    fn reset_clears_the_filter_state() {
        let mut b = DcBlocker::new(0.99);
        let _ = run(&mut b, &[1.0; 100]);
        b.reset();
        assert_eq!(b.prev_in, 0.0);
        assert_eq!(b.prev_out, 0.0);
    }
}
