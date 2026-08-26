//! Tone: a one-knob tilt EQ that trades low end for high end around a pivot.
//!
//! Splits the signal into a low band and a high band at `pivot_hz` (a one-pole crossover)
//! and recombines them with complementary gains: `tilt = 0` is flat (`low + high = input`),
//! `tilt > 0` boosts the highs and cuts the lows (brighter), `tilt < 0` does the reverse
//! (darker). One control shapes the whole spectrum — the classic "tilt" tone control.

use std::f32::consts::TAU;
use vb_core::{ProcessContext, Processor};

/// A tilt tone control.
///
/// Input 0 is audio, output 0 is the tilted signal. `tilt` in `[-1, 1]` sets the slope
/// (positive = brighter), `pivot_hz` sets the crossover frequency the tilt rotates around.
#[derive(Debug, Clone)]
pub struct Tone {
    /// Tilt amount, clamped to `[-1, 1]`.
    tilt: f32,
    /// Crossover frequency in Hz, clamped to a sane audible range.
    pivot_hz: f32,
    /// One-pole low-pass state (the low band).
    lp: f32,
}

impl Tone {
    /// Creates a tone control. `tilt` is clamped to `[-1, 1]`, `pivot_hz` to `[20, 18000]`.
    #[must_use]
    pub fn new(tilt: f32, pivot_hz: f32) -> Self {
        Self {
            tilt: tilt.clamp(-1.0, 1.0),
            pivot_hz: pivot_hz.clamp(20.0, 18_000.0),
            lp: 0.0,
        }
    }
}

impl Processor for Tone {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (Some(input), Some(out)) = (inputs.first(), outputs.first_mut()) else {
            return;
        };
        let sr = ctx.sample_rate.max(1.0);
        // One-pole low-pass coefficient for the crossover.
        let a = (-TAU * self.pivot_hz / sr).exp();
        // Complementary band gains: flat at tilt 0, ±band emphasis otherwise.
        let lo_gain = 1.0 - self.tilt;
        let hi_gain = 1.0 + self.tilt;
        for (o, &x) in out.iter_mut().zip(input.iter()).take(ctx.frames) {
            self.lp = (1.0 - a) * x + a * self.lp;
            let high = x - self.lp;
            *o = self.lp * lo_gain + high * hi_gain;
        }
    }

    fn reset(&mut self) {
        self.lp = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (TAU * freq * i as f32 / SR).sin()).collect()
    }

    fn run(t: &mut Tone, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            t.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn rms(xs: &[f32]) -> f32 {
        (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt()
    }

    #[test]
    fn flat_at_zero_tilt_is_transparent() {
        let input = sine(1_000.0, 4_800);
        let mut t = Tone::new(0.0, 700.0);
        let out = run(&mut t, &input);
        // low + high == input by construction, regardless of the crossover.
        for (o, x) in out.iter().zip(&input).skip(100) {
            assert!((o - x).abs() < 1e-4, "flat passes through: {o} vs {x}");
        }
    }

    #[test]
    fn positive_tilt_brightens_boosting_highs_over_lows() {
        let low = sine(100.0, 9_600);
        let high = sine(6_000.0, 9_600);
        let mut tl = Tone::new(0.8, 700.0);
        let mut th = Tone::new(0.8, 700.0);
        let out_low = run(&mut tl, &low);
        let out_high = run(&mut th, &high);
        // Second half, past the filter transient.
        let lo = rms(&out_low[4_800..]);
        let hi = rms(&out_high[4_800..]);
        assert!(lo < 0.5, "lows cut by positive tilt: {lo}");
        assert!(hi > 1.2, "highs boosted by positive tilt: {hi}");
    }

    #[test]
    fn negative_tilt_darkens_boosting_lows_over_highs() {
        let low = sine(100.0, 9_600);
        let high = sine(6_000.0, 9_600);
        let mut tl = Tone::new(-0.8, 700.0);
        let mut th = Tone::new(-0.8, 700.0);
        let lo = rms(&run(&mut tl, &low)[4_800..]);
        let hi = rms(&run(&mut th, &high)[4_800..]);
        assert!(lo > 1.2, "lows boosted by negative tilt: {lo}");
        assert!(hi < 0.5, "highs cut by negative tilt: {hi}");
    }

    #[test]
    fn params_are_clamped() {
        assert_eq!(Tone::new(5.0, 700.0).tilt, 1.0);
        assert_eq!(Tone::new(-5.0, 700.0).tilt, -1.0);
        assert_eq!(Tone::new(0.0, 1.0).pivot_hz, 20.0);
        assert_eq!(Tone::new(0.0, 99_000.0).pivot_hz, 18_000.0);
    }

    #[test]
    fn reset_clears_the_filter_state() {
        let mut t = Tone::new(0.5, 700.0);
        let _ = run(&mut t, &sine(1_000.0, 100));
        t.reset();
        assert_eq!(t.lp, 0.0);
    }
}
