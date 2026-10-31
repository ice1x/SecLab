//! Allpass filter: unity magnitude at every frequency, only the phase is shaped.
//!
//! An allpass passes all frequencies at the same level but delays each by a
//! frequency-dependent amount. On its own it sounds almost like the dry signal, but it is the
//! building block of phasers (a chain of allpass stages swept by an LFO), Schroeder reverbs
//! and fractional-delay/dispersion effects — anywhere phase matters but the spectrum must stay
//! flat. `freq_hz` sets where the phase swings through −180°, and `q` how sharply.
//!
//! Coefficients are the RBJ Audio EQ Cookbook APF biquad, run in Direct Form I. Because the
//! numerator is the denominator with its middle term mirrored (`b = {a2, a1, a0}`), the
//! magnitude response is exactly unity at all frequencies; at the centre frequency the phase
//! is −180°, so a settled tone there comes out inverted.
//!
//! Primary source: RBJ "Cookbook formulae for audio EQ biquad filter coefficients"
//! (W3C/WebAudio copy, <https://webaudio.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html>).

use crate::biquad::{BiquadCoeffs, BiquadState};
use seclab_core::{ProcessContext, Processor};

/// A second-order allpass filter.
///
/// Input 0 is audio. Output 0 is the input with its phase shaped but its magnitude untouched.
#[derive(Debug, Clone)]
pub struct Allpass {
    freq_hz: f32,
    q: f32,
    state: BiquadState,
}

impl Allpass {
    /// Creates an allpass centred at `freq_hz` with sharpness `q` (clamped `>= 1e-4`).
    #[must_use]
    pub fn new(freq_hz: f32, q: f32) -> Self {
        Self {
            freq_hz,
            q: q.max(1e-4),
            state: BiquadState::default(),
        }
    }

    fn coeffs(&self, sample_rate: f32) -> BiquadCoeffs {
        // RBJ cookbook APF biquad.
        let f0 = self.freq_hz.clamp(1.0, 0.49 * sample_rate);
        let w0 = core::f32::consts::TAU * (f0 / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * self.q);

        let b0 = 1.0 - alpha;
        let b1 = -2.0 * cos;
        let b2 = 1.0 + alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha;

        BiquadCoeffs::normalized(b0, b1, b2, a0, a1, a2)
    }
}

impl Processor for Allpass {
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

    fn run(f: &mut Allpass, input: &[f32]) -> Vec<f32> {
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
    fn magnitude_is_flat_at_every_frequency() {
        // Whatever the tone, the allpass preserves its level (unity magnitude).
        for hz in [120.0, 1_000.0, 5_000.0, 12_000.0] {
            let input = tone(hz, 8_192);
            let out = run(&mut Allpass::new(1_000.0, 0.7), &input);
            let gain = rms(&out[4_096..]) / rms(&input[4_096..]);
            assert!(
                (gain - 1.0).abs() < 0.02,
                "magnitude is unity at {hz} Hz, got {gain}"
            );
        }
    }

    #[test]
    fn phase_inverts_a_settled_tone_at_the_centre() {
        // At the centre frequency the allpass phase is −180°, so a settled tone comes out
        // as its own negation.
        let input = tone(1_000.0, 8_192);
        let out = run(&mut Allpass::new(1_000.0, 0.7), &input);
        let max_diff = input[6_000..]
            .iter()
            .zip(&out[6_000..])
            .map(|(x, y)| (y - (-x)).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_diff < 0.03,
            "a centre-frequency tone is inverted in steady state (max diff {max_diff})"
        );
    }

    #[test]
    fn it_is_not_a_passthrough_off_centre() {
        // Away from the centre the phase shift is partial, so the output is neither the input
        // nor its negation — the filter actually does something.
        let input = tone(300.0, 8_192);
        let out = run(&mut Allpass::new(3_000.0, 1.0), &input);
        let diff_pos = input[4_096..]
            .iter()
            .zip(&out[4_096..])
            .map(|(x, y)| (y - x).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff_pos > 0.05,
            "the allpass shifts the phase (max diff {diff_pos})"
        );
        // But the level is still preserved.
        let gain = rms(&out[4_096..]) / rms(&input[4_096..]);
        assert!(
            (gain - 1.0).abs() < 0.02,
            "still unity magnitude, got {gain}"
        );
    }

    #[test]
    fn output_stays_bounded_and_finite() {
        let input = tone(440.0, 4_096);
        let out = run(&mut Allpass::new(440.0, 15.0), &input);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 10.0),
            "a sharp allpass stays finite and bounded"
        );
    }

    #[test]
    fn q_is_clamped_positive() {
        let f = Allpass::new(1_000.0, 0.0);
        assert!(f.q >= 1e-4, "q is clamped away from zero");
    }
}
