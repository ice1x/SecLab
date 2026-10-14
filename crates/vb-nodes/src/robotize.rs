//! Robotization: the classic phase-vocoder "robot voice" — replace every FFT bin's phase with a
//! fixed value, keeping only its magnitude, then resynthesise.
//!
//! In the DAFX formulation (Zölzer, *DAFX*, the robotization effect) each short-time frame keeps
//! its magnitude spectrum `|X(k)|` and discards the measured phase. Every resynthesised grain is
//! therefore identical in phase, so the overlap-add is a train of identical pulses spaced one hop
//! apart. That imposes a constant period of [`crate::stft::STEP_SIZE`] samples on the output: the
//! pitch collapses to `sample_rate / STEP_SIZE` (and its harmonics) **regardless of the input's
//! pitch**, while the magnitude spectrum — the formant / timbre envelope — is preserved. The ear
//! hears a monotone, buzzy "robot" version of the source.
//!
//! The reference MATLAB sets the spectrum to `abs(f)` and uses an `fftshift` before the FFT and
//! after the inverse FFT so the zero-phase impulse lands in the centre of the synthesis window
//! (where the Hann taper is unity) instead of at the frame edges (where it is zero). The shared
//! [`crate::stft`] engine applies no such shift, so this node folds the half-frame circular shift
//! into the frequency domain: a delay of `FFT_SIZE/2` samples multiplies bin `k` by
//! `exp(-j·2π·k·(N/2)/N) = (-1)^k`, which keeps the synthesised pulse real and centred. Bins are
//! doubled for the engine's half-spectrum convention (`synthesize` drops the negative half without
//! compensating), exactly as the identity round-trip does.
//!
//! Input 0 is audio. Output 0 is the robotised (and dry-mixed) signal, delayed by
//! [`Robotize::latency_frames`]. The effect is memoryless per frame — no phase accumulation.

use crate::stft::{Stft, HALF, LATENCY};
use vb_core::{ProcessContext, Processor};

/// A phase-vocoder robotizer (zero-phase resynthesis).
pub struct Robotize {
    mix: f32,
    stft: Stft,
}

impl Robotize {
    /// Creates a robotizer with dry/wet `mix` (0 = dry, 1 = fully robotised).
    #[must_use]
    pub fn new(mix: f32) -> Self {
        Self {
            mix: mix.clamp(0.0, 1.0),
            stft: Stft::new(),
        }
    }

    /// Replaces the analysed half-spectrum with its zero-phase (robotised) form in place: each bin
    /// becomes `2·|X(k)|·(-1)^k` (magnitude only; the `(-1)^k` centres the pulse, the `2` restores
    /// the half-spectrum convention), imaginary part zero.
    fn transform(&mut self) {
        for k in 0..=HALF {
            let re = self.stft.fft_work[2 * k];
            let im = self.stft.fft_work[2 * k + 1];
            let mag = (re * re + im * im).sqrt();
            let sign = if k % 2 == 0 { 2.0 } else { -2.0 };
            self.stft.fft_work[2 * k] = mag * sign;
            self.stft.fft_work[2 * k + 1] = 0.0;
        }
    }
}

impl Processor for Robotize {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        if ctx.sample_rate <= 0.0 {
            for (o, x) in outputs[0].iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        let mix = self.mix;
        for (i, o) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let dry = self.stft.dry_delay(x);
            let (wet, frame_ready) = self.stft.push(x);
            if frame_ready {
                self.stft.forward();
                self.transform();
                self.stft.synthesize();
            }
            *o = mix * wet + (1.0 - mix) * dry;
        }
    }

    fn reset(&mut self) {
        self.stft.reset();
    }

    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&m) = values.first() {
            self.mix = m.clamp(0.0, 1.0);
        }
        true
    }

    fn latency_frames(&self) -> usize {
        LATENCY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stft::STEP_SIZE;

    const SR: f32 = 48_000.0;

    fn run(p: &mut Robotize, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            p.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * hz * i as f32 / SR).sin())
            .collect()
    }

    /// Normalised autocorrelation of `xs` at `lag`: 1.0 for a signal exactly periodic with that
    /// period, near 0 for an unrelated lag.
    fn autocorr(xs: &[f32], lag: usize) -> f32 {
        let mut num = 0.0_f32;
        let mut den = 0.0_f32;
        for n in 0..xs.len() - lag {
            num += xs[n] * xs[n + lag];
            den += xs[n] * xs[n];
        }
        num / den
    }

    #[test]
    fn latency_is_the_fft_minus_one_hop() {
        assert_eq!(Robotize::new(1.0).latency_frames(), LATENCY);
    }

    #[test]
    fn it_imposes_a_fixed_period_of_one_hop_regardless_of_input_pitch() {
        // Robotization collapses the output to a pulse train spaced one synthesis hop apart, so the
        // output is periodic with period STEP_SIZE whatever the input pitch. We prove it two ways:
        //  * the robotised output's autocorrelation at lag STEP_SIZE is ~1 (periodic there);
        //  * the dry input tone (period ≠ STEP_SIZE) is NOT periodic at that lag;
        //  * a different input pitch robotises to the SAME hop period.
        let n = 24_576;
        let settle = 8_192;

        let a_in = tone(440.0, n);
        let a_out = run(&mut Robotize::new(1.0), &a_in);
        let a_robot = autocorr(&a_out[settle..], STEP_SIZE);
        let a_dry = autocorr(&a_in[settle..], STEP_SIZE);
        assert!(
            a_robot > 0.85,
            "robotised 440 Hz is periodic at the hop: autocorr(STEP_SIZE) = {a_robot}"
        );
        assert!(
            a_dry < 0.5,
            "the dry 440 Hz tone is not periodic at the hop: autocorr(STEP_SIZE) = {a_dry}"
        );

        let b_out = run(&mut Robotize::new(1.0), &tone(587.33, n));
        let b_robot = autocorr(&b_out[settle..], STEP_SIZE);
        assert!(
            b_robot > 0.85,
            "a different input pitch robotises to the same hop period: autocorr = {b_robot}"
        );
    }

    #[test]
    fn silence_in_silence_out() {
        let out = run(&mut Robotize::new(1.0), &vec![0.0; 8_192]);
        assert!(out.iter().all(|s| s.abs() < 1e-5), "silence stays silent");
    }

    #[test]
    fn a_dry_mix_passes_the_input_through() {
        let input = tone(440.0, 4_096);
        let out = run(&mut Robotize::new(0.0), &input);
        for i in LATENCY..4_096 {
            assert!(
                (out[i] - input[i - LATENCY]).abs() < 1e-6,
                "dry mix is the delayed input"
            );
        }
    }

    #[test]
    fn it_preserves_the_spectral_envelope_roughly_at_unit_level() {
        // The magnitude spectrum (timbre) is kept, so a unit tone robotises to a bounded,
        // audible signal — not silence and not a blow-up.
        let out = run(&mut Robotize::new(1.0), &tone(440.0, 16_384));
        let rms =
            (out[8_192..].iter().map(|s| s * s).sum::<f32>() / out[8_192..].len() as f32).sqrt();
        assert!(
            (0.05..2.0).contains(&rms),
            "robotised output is audible and bounded, rms = {rms}"
        );
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let out = run(&mut Robotize::new(1.0), &tone(220.0, 16_384));
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 8.0),
            "robotised output stays finite and bounded"
        );
    }

    #[test]
    fn set_params_updates_and_clamps() {
        let mut r = Robotize::new(1.0);
        assert!(r.set_params(&[0.5]));
        assert!((r.mix - 0.5).abs() < 1e-6);
        assert!(r.set_params(&[9.0]));
        assert!((r.mix - 1.0).abs() < 1e-6, "mix clamps to 1");
        assert!(r.set_params(&[-9.0]));
        assert!((r.mix - 0.0).abs() < 1e-6, "mix clamps to 0");
    }
}
