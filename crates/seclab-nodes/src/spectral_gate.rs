//! Spectral gate: silences the frequency bins that sit more than a threshold below the loudest
//! bin of each frame — a frequency-domain noise gate / spectral denoiser.
//!
//! A time-domain gate opens or closes on the *whole* signal's level; a spectral gate makes that
//! decision **per frequency bin**. Each short-time frame it finds the loudest bin, then keeps only
//! the bins within `threshold_db` of that peak and zeroes the rest. A steady tone over a hiss
//! floor comes out clean (the tone's bin survives, the broadband hiss between partials is gated);
//! pushed harder it becomes a creative "spectral gate" that strips a sound down to its strongest
//! partials. The threshold is relative to the per-frame peak, so it tracks level automatically.
//!
//! Input 0 is audio. Output 0 is the gated (and dry-mixed) signal, delayed by
//! [`SpectralGate::latency_frames`].
//!
//! Built on the shared [`crate::stft`] overlap-add engine: the per-frame transform keeps each
//! surviving bin's analysed complex value (doubled for the half-spectrum convention) and zeroes
//! the rest, so the kept content is reconstructed at unit gain with its phase intact — the gate is
//! memoryless per frame (no phase accumulation needed).

use crate::stft::{Stft, HALF, LATENCY};
use seclab_core::{ProcessContext, Processor};

/// A spectral noise gate.
pub struct SpectralGate {
    threshold_db: f32,
    mix: f32,
    stft: Stft,
}

impl SpectralGate {
    /// Creates a spectral gate keeping bins within `threshold_db` (negative dB, e.g. −40) of each
    /// frame's peak bin, with dry/wet `mix` (0 = dry, 1 = fully gated).
    #[must_use]
    pub fn new(threshold_db: f32, mix: f32) -> Self {
        Self {
            threshold_db: threshold_db.clamp(-120.0, 0.0),
            mix: mix.clamp(0.0, 1.0),
            stft: Stft::new(),
        }
    }

    /// Gates the analysed half-spectrum in place: keep bins at or above `peak · 10^(thr/20)`,
    /// zero the rest. Surviving bins are doubled for the engine's half-spectrum convention.
    fn transform(&mut self, threshold_db: f32) {
        let mut peak = 1e-12_f32;
        for k in 0..=HALF {
            let re = self.stft.fft_work[2 * k];
            let im = self.stft.fft_work[2 * k + 1];
            peak = peak.max((re * re + im * im).sqrt());
        }
        let thresh = peak * 10.0_f32.powf(threshold_db / 20.0);
        for k in 0..=HALF {
            let re = self.stft.fft_work[2 * k];
            let im = self.stft.fft_work[2 * k + 1];
            let g = if (re * re + im * im).sqrt() >= thresh {
                2.0
            } else {
                0.0
            };
            self.stft.fft_work[2 * k] = re * g;
            self.stft.fft_work[2 * k + 1] = im * g;
        }
    }
}

impl Processor for SpectralGate {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        if ctx.sample_rate <= 0.0 {
            for (o, x) in outputs[0].iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        let threshold_db = self.threshold_db;
        let mix = self.mix;
        for (i, o) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let dry = self.stft.dry_delay(x);
            let (wet, frame_ready) = self.stft.push(x);
            if frame_ready {
                self.stft.forward();
                self.transform(threshold_db);
                self.stft.synthesize();
            }
            *o = mix * wet + (1.0 - mix) * dry;
        }
    }

    fn reset(&mut self) {
        self.stft.reset();
    }

    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&t) = values.first() {
            self.threshold_db = t.clamp(-120.0, 0.0);
        }
        if let Some(&m) = values.get(1) {
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

    const SR: f32 = 48_000.0;

    fn run(p: &mut SpectralGate, input: &[f32]) -> Vec<f32> {
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

    fn mag(xs: &[f32], hz: f32) -> f32 {
        let w = core::f32::consts::TAU * hz / SR;
        let (mut re, mut im) = (0.0_f32, 0.0_f32);
        for (n, &x) in xs.iter().enumerate() {
            re += x * (w * n as f32).cos();
            im -= x * (w * n as f32).sin();
        }
        (re * re + im * im).sqrt() / xs.len() as f32
    }

    #[test]
    fn latency_is_the_fft_minus_one_hop() {
        assert_eq!(SpectralGate::new(-40.0, 1.0).latency_frames(), LATENCY);
    }

    #[test]
    fn a_low_threshold_reconstructs_the_input() {
        // −110 dB keeps essentially everything, so the gate is a faithful reconstruction.
        let input = tone(440.0, 16_384);
        let out = run(&mut SpectralGate::new(-110.0, 1.0), &input);
        let m = mag(&out[8_192..], 440.0);
        assert!(
            (m - 0.5).abs() < 0.08,
            "near-open gate reconstructs 440 Hz, got {m}"
        );
    }

    #[test]
    fn it_gates_a_quiet_partial_below_the_threshold_but_keeps_a_louder_one_above() {
        // A loud 440 Hz tone plus a quiet 3 kHz tone 40 dB down. With a −30 dB threshold the quiet
        // partial is below the gate (relative to the 440 Hz peak) and is removed; the loud one
        // survives. Dropping the threshold to −50 dB (below the partial) lets it back through —
        // proving it is the threshold, not something else, doing the gating.
        let n = 32_768;
        let mut input = tone(440.0, n);
        for (x, q) in input.iter_mut().zip(tone(3_000.0, n)) {
            *x += 0.01 * q; // −40 dB partial
        }
        let it = &input[16_384..];
        let in_partial = mag(it, 3_000.0);

        let gated = run(&mut SpectralGate::new(-30.0, 1.0), &input);
        let gt = &gated[16_384..];
        assert!(
            mag(gt, 440.0) > 0.35,
            "the loud tone survives: {}",
            mag(gt, 440.0)
        );
        assert!(
            mag(gt, 3_000.0) < 0.3 * in_partial,
            "the −40 dB partial is gated at −30 dB: in {in_partial}, out {}",
            mag(gt, 3_000.0)
        );

        let open = run(&mut SpectralGate::new(-50.0, 1.0), &input);
        assert!(
            mag(&open[16_384..], 3_000.0) > 0.5 * in_partial,
            "the partial returns when the threshold drops below it"
        );
    }

    #[test]
    fn silence_in_silence_out() {
        let out = run(&mut SpectralGate::new(-40.0, 1.0), &vec![0.0; 8_192]);
        assert!(out.iter().all(|s| s.abs() < 1e-5), "silence stays silent");
    }

    #[test]
    fn a_dry_mix_passes_the_input_through() {
        let input = tone(440.0, 4_096);
        let out = run(&mut SpectralGate::new(-10.0, 0.0), &input);
        for i in LATENCY..4_096 {
            assert!(
                (out[i] - input[i - LATENCY]).abs() < 1e-6,
                "dry mix is the delayed input"
            );
        }
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let out = run(&mut SpectralGate::new(-20.0, 1.0), &tone(220.0, 16_384));
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 8.0),
            "gated output stays finite and bounded"
        );
    }

    #[test]
    fn set_params_updates_and_clamps() {
        let mut g = SpectralGate::new(-40.0, 1.0);
        assert!(g.set_params(&[-24.0, 0.5]));
        assert!((g.threshold_db + 24.0).abs() < 1e-6);
        assert!((g.mix - 0.5).abs() < 1e-6);
        assert!(g.set_params(&[10.0, 9.0]));
        assert!(
            (g.threshold_db - 0.0).abs() < 1e-6,
            "threshold clamps to 0 dB"
        );
        assert!((g.mix - 1.0).abs() < 1e-6);
    }
}
