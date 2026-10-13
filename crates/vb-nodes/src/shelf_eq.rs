//! Shelving EQ: a low- or high-shelf biquad that boosts or cuts one end of the spectrum.
//!
//! Unlike [`crate::Biquad`]'s resonant low-/high-pass (which *remove* a band) and
//! [`crate::Tone`]'s single tilt control, a shelf lifts or drops everything below (low shelf)
//! or above (high shelf) a corner frequency by a fixed `gain_db`, leaving the other end at
//! unity — the bass/treble controls of a mixing desk. At `gain_db = 0` the filter is an exact
//! passthrough.
//!
//! Coefficients are the RBJ Audio EQ Cookbook shelving biquads, run in Direct Form I. The
//! shelf slope is fixed at `S = 1` (a maximally-flat shelf, no response overshoot), so the
//! only controls are the corner `freq_hz` and the `gain_db`.
//!
//! Primary source: RBJ "Cookbook formulae for audio EQ biquad filter coefficients"
//! (W3C/WebAudio copy, <https://webaudio.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html>).

use crate::biquad::{BiquadCoeffs, BiquadState};
use vb_core::{ProcessContext, Processor};

/// Which end of the spectrum the shelf acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShelfMode {
    /// Boost/cut everything below the corner frequency; unity above.
    Low,
    /// Boost/cut everything above the corner frequency; unity below.
    High,
}

/// Maps a stepped parameter value to a [`ShelfMode`] (`>= 0.5` is high shelf, else low).
#[must_use]
pub fn shelf_mode_from(v: f32) -> ShelfMode {
    if v >= 0.5 {
        ShelfMode::High
    } else {
        ShelfMode::Low
    }
}

/// A low- or high-shelf equaliser.
///
/// Input 0 is audio. Output 0 is the shelved signal.
#[derive(Debug, Clone)]
pub struct ShelfEq {
    mode: ShelfMode,
    freq_hz: f32,
    gain_db: f32,
    state: BiquadState,
}

impl ShelfEq {
    /// Creates a shelf of `mode` at corner `freq_hz` with `gain_db` of boost (positive) or
    /// cut (negative).
    #[must_use]
    pub fn new(mode: ShelfMode, freq_hz: f32, gain_db: f32) -> Self {
        Self {
            mode,
            freq_hz,
            gain_db,
            state: BiquadState::default(),
        }
    }

    fn coeffs(&self, sample_rate: f32) -> BiquadCoeffs {
        // RBJ cookbook shelving biquad, shelf slope S = 1.
        let a = 10.0_f32.powf(self.gain_db / 40.0); // A = 10^(dBgain/40)
        let f0 = self.freq_hz.clamp(1.0, 0.49 * sample_rate);
        let w0 = core::f32::consts::TAU * (f0 / sample_rate);
        let (sin, cos) = w0.sin_cos();
        // S = 1 ⇒ α = (sin/2)·√((A + 1/A)(1/S − 1) + 2) = (sin/2)·√2.
        let alpha = sin * 0.5 * core::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        let ap1 = a + 1.0;
        let am1 = a - 1.0;

        let (b0, b1, b2, a0, a1, a2) = match self.mode {
            ShelfMode::Low => (
                a * (ap1 - am1 * cos + two_sqrt_a_alpha),
                2.0 * a * (am1 - ap1 * cos),
                a * (ap1 - am1 * cos - two_sqrt_a_alpha),
                ap1 + am1 * cos + two_sqrt_a_alpha,
                -2.0 * (am1 + ap1 * cos),
                ap1 + am1 * cos - two_sqrt_a_alpha,
            ),
            ShelfMode::High => (
                a * (ap1 + am1 * cos + two_sqrt_a_alpha),
                -2.0 * a * (am1 + ap1 * cos),
                a * (ap1 + am1 * cos - two_sqrt_a_alpha),
                ap1 - am1 * cos + two_sqrt_a_alpha,
                2.0 * (am1 - ap1 * cos),
                ap1 - am1 * cos - two_sqrt_a_alpha,
            ),
        };

        BiquadCoeffs::normalized(b0, b1, b2, a0, a1, a2)
    }
}

impl Processor for ShelfEq {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        if ctx.sample_rate <= 0.0 {
            for (o, x) in outputs[0].iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        let c = self.coeffs(ctx.sample_rate);
        for (i, o) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let xn = input.get(i).copied().unwrap_or(0.0);
            *o = self.state.step(&c, xn);
        }
    }

    fn reset(&mut self) {
        self.state.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(f: &mut ShelfEq, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            f.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * hz * i as f32 / SR).sin())
            .collect()
    }

    fn rms(xs: &[f32]) -> f32 {
        (xs.iter().map(|x| x * x).sum::<f32>() / xs.len() as f32).sqrt()
    }

    #[test]
    fn zero_gain_is_an_exact_passthrough() {
        // At gain 0, A = 1 so the numerator and denominator coincide (H(z) = 1): output
        // equals input for both shelf modes.
        for mode in [ShelfMode::Low, ShelfMode::High] {
            let input = tone(440.0, 2_048);
            let out = run(&mut ShelfEq::new(mode, 1_000.0, 0.0), &input);
            for (a, b) in input.iter().zip(&out) {
                assert!((a - b).abs() < 1e-5, "{mode:?} passes through at 0 dB");
            }
        }
    }

    #[test]
    fn low_shelf_boosts_dc_by_a_squared() {
        // A low shelf lifts everything below the corner; its DC gain is A² = 10^(dB/20).
        let out = run(
            &mut ShelfEq::new(ShelfMode::Low, 1_000.0, 12.0),
            &[1.0; 4_096],
        );
        let settled = out[4_095];
        let expected = 10.0_f32.powf(12.0 / 20.0); // ≈ 3.98
        assert!(
            (settled - expected).abs() < expected * 0.02,
            "low-shelf +12 dB DC gain ≈ {expected}, got {settled}"
        );
    }

    #[test]
    fn low_shelf_cut_attenuates_dc() {
        let out = run(
            &mut ShelfEq::new(ShelfMode::Low, 1_000.0, -12.0),
            &[1.0; 4_096],
        );
        let settled = out[4_095];
        let expected = 10.0_f32.powf(-12.0 / 20.0); // ≈ 0.251
        assert!(
            (settled - expected).abs() < 0.02,
            "low-shelf −12 dB DC gain ≈ {expected}, got {settled}"
        );
    }

    #[test]
    fn low_shelf_leaves_dc_but_a_high_shelf_leaves_it_alone() {
        // A high shelf acts only above its corner, so it leaves DC at unity.
        let out = run(
            &mut ShelfEq::new(ShelfMode::High, 1_000.0, 12.0),
            &[1.0; 4_096],
        );
        let settled = out[4_095];
        assert!(
            (settled - 1.0).abs() < 0.02,
            "high-shelf leaves DC at unity, got {settled}"
        );
    }

    #[test]
    fn high_shelf_boosts_treble() {
        // Well above the 1 kHz corner, a +12 dB high shelf raises the level toward A² ≈ 4×.
        let input = tone(8_000.0, 8_192);
        let out = run(&mut ShelfEq::new(ShelfMode::High, 1_000.0, 12.0), &input);
        // Compare the settled tail (skip the transient) against the input RMS.
        let gain = rms(&out[4_096..]) / rms(&input[4_096..]);
        let expected = 10.0_f32.powf(12.0 / 20.0);
        assert!(
            (gain - expected).abs() < expected * 0.1,
            "high-shelf +12 dB treble gain ≈ {expected}, got {gain}"
        );
    }

    #[test]
    fn output_stays_bounded_and_finite() {
        let input = tone(220.0, 4_096);
        let out = run(&mut ShelfEq::new(ShelfMode::Low, 500.0, 24.0), &input);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 100.0),
            "a hot shelf stays finite and bounded"
        );
    }

    #[test]
    fn mode_mapping_is_stepped_at_a_half() {
        assert_eq!(shelf_mode_from(0.0), ShelfMode::Low);
        assert_eq!(shelf_mode_from(0.49), ShelfMode::Low);
        assert_eq!(shelf_mode_from(0.5), ShelfMode::High);
        assert_eq!(shelf_mode_from(1.0), ShelfMode::High);
    }
}
