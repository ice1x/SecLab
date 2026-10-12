//! Peaking (bell) EQ: a resonant boost or cut of one band, the parametric mid of an EQ.
//!
//! Where [`crate::ShelfEq`] lifts or drops a whole end of the spectrum and [`crate::Biquad`]'s
//! notch *removes* a band entirely, a peaking filter boosts or cuts a bell-shaped band around a
//! centre frequency by `gain_db`, leaving everything far from it at unity. `q` sets the
//! bandwidth (higher `q` = narrower band). Together with a low and a high shelf this is a basic
//! parametric equaliser.
//!
//! Coefficients are the RBJ Audio EQ Cookbook peakingEQ biquad, run in Direct Form I. At
//! `gain_db = 0` the numerator and denominator coincide (`H(z) = 1`): an exact passthrough.
//! DC and Nyquist always pass at unity — only the band around the centre is shaped — and the
//! magnitude at the centre frequency is exactly `A² = 10^(gain_db/20)`.
//!
//! Primary source: RBJ "Cookbook formulae for audio EQ biquad filter coefficients"
//! (W3C/WebAudio copy, <https://webaudio.github.io/Audio-EQ-Cookbook/audio-eq-cookbook.html>).

use vb_core::{ProcessContext, Processor};

/// Direct Form I biquad normalised coefficients.
#[derive(Debug, Clone, Copy)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// A peaking (bell) equaliser band.
///
/// Input 0 is audio. Output 0 is the signal with the band around `freq_hz` boosted or cut.
#[derive(Debug, Clone)]
pub struct PeakEq {
    freq_hz: f32,
    q: f32,
    gain_db: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl PeakEq {
    /// Creates a peaking band centred at `freq_hz` with bandwidth `q` (clamped `>= 1e-4`) and
    /// `gain_db` of boost (positive) or cut (negative).
    #[must_use]
    pub fn new(freq_hz: f32, q: f32, gain_db: f32) -> Self {
        Self {
            freq_hz,
            q: q.max(1e-4),
            gain_db,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn coeffs(&self, sample_rate: f32) -> Coeffs {
        // RBJ cookbook peakingEQ biquad.
        let a = 10.0_f32.powf(self.gain_db / 40.0); // A = 10^(dBgain/40)
        let f0 = self.freq_hz.clamp(1.0, 0.49 * sample_rate);
        let w0 = core::f32::consts::TAU * (f0 / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * self.q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos;
        let a2 = 1.0 - alpha / a;

        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

impl Processor for PeakEq {
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
            let yn = c.b0 * xn + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
            self.x2 = self.x1;
            self.x1 = xn;
            self.y2 = self.y1;
            self.y1 = yn;
            *o = yn;
        }
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(f: &mut PeakEq, input: &[f32]) -> Vec<f32> {
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
        let input = tone(1_000.0, 2_048);
        let out = run(&mut PeakEq::new(1_000.0, 1.0, 0.0), &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-5, "peaking EQ passes through at 0 dB");
        }
    }

    #[test]
    fn boost_at_the_centre_frequency_is_a_squared() {
        // A tone sitting exactly on the centre is amplified by A² = 10^(dB/20).
        let input = tone(1_000.0, 8_192);
        let out = run(&mut PeakEq::new(1_000.0, 1.0, 12.0), &input);
        let gain = rms(&out[4_096..]) / rms(&input[4_096..]);
        let expected = 10.0_f32.powf(12.0 / 20.0); // ≈ 3.98
        assert!(
            (gain - expected).abs() < expected * 0.05,
            "centre boost ≈ {expected}, got {gain}"
        );
    }

    #[test]
    fn cut_at_the_centre_frequency_attenuates() {
        let input = tone(1_000.0, 8_192);
        let out = run(&mut PeakEq::new(1_000.0, 1.0, -12.0), &input);
        let gain = rms(&out[4_096..]) / rms(&input[4_096..]);
        let expected = 10.0_f32.powf(-12.0 / 20.0); // ≈ 0.251
        assert!(
            (gain - expected).abs() < expected * 0.1,
            "centre cut ≈ {expected}, got {gain}"
        );
    }

    #[test]
    fn frequencies_far_from_the_centre_are_left_alone() {
        // A 50 Hz tone is far below a narrow 5 kHz boost, so it passes at ~unity.
        let input = tone(50.0, 8_192);
        let out = run(&mut PeakEq::new(5_000.0, 4.0, 12.0), &input);
        let gain = rms(&out[4_096..]) / rms(&input[4_096..]);
        assert!(
            (gain - 1.0).abs() < 0.05,
            "far-from-centre is near unity, got {gain}"
        );
    }

    #[test]
    fn a_narrower_q_boosts_a_narrower_band() {
        // At an off-centre probe frequency, a wide band (low Q) still lifts it appreciably
        // while a narrow band (high Q) barely touches it.
        let probe = tone(1_500.0, 8_192); // ~0.58 octave above a 1 kHz band
        let wide = run(&mut PeakEq::new(1_000.0, 0.7, 12.0), &probe);
        let narrow = run(&mut PeakEq::new(1_000.0, 8.0, 12.0), &probe);
        let g_wide = rms(&wide[4_096..]) / rms(&probe[4_096..]);
        let g_narrow = rms(&narrow[4_096..]) / rms(&probe[4_096..]);
        assert!(
            g_wide > g_narrow + 0.1,
            "a wider band lifts the off-centre probe more: wide {g_wide}, narrow {g_narrow}"
        );
    }

    #[test]
    fn output_stays_bounded_and_finite() {
        let input = tone(440.0, 4_096);
        let out = run(&mut PeakEq::new(440.0, 10.0, 24.0), &input);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 100.0),
            "a hot narrow boost stays finite and bounded"
        );
    }

    #[test]
    fn q_is_clamped_positive() {
        let f = PeakEq::new(1_000.0, 0.0, 6.0);
        assert!(f.q >= 1e-4, "q is clamped away from zero");
    }
}
