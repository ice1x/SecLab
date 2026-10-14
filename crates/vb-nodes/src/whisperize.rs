//! Whisperization: the phase-vocoder "whisper voice" — the complement of [`crate::robotize`].
//!
//! Where robotization replaces every FFT bin's phase with a *fixed* value (zero) to impose a
//! single pitch, whisperization replaces each bin's phase with a *random* value while keeping its
//! magnitude `|X(k)|`. Destroying the phase coherence — between bins and between frames — removes
//! any periodic (pitched) structure, so the output has no pitch at all; but because the magnitude
//! spectrum is preserved, the formant / timbre envelope survives. A sung vowel comes out as the
//! same vowel *whispered*: the resonances are there, the pitch is gone. This is the DAFX (Zölzer)
//! whisperization effect (`ft = abs(f) .* exp(i·random_phase)`).
//!
//! The random phase is drawn from a seeded xorshift32 PRNG (the same generator and seed as
//! [`crate::noise`]), so a patch renders identically every time and [`Whisperize::reset`] replays
//! from the seed — determinism is part of the node contract. Bins are doubled for the shared
//! engine's half-spectrum convention (`synthesize` keeps the positive half and takes the real part
//! of the inverse); random phase on the positive half alone yields a valid real output.
//!
//! Input 0 is audio. Output 0 is the whispered (and dry-mixed) signal, delayed by
//! [`Whisperize::latency_frames`]. The effect is memoryless per frame apart from the PRNG state —
//! no phase accumulation.

use crate::stft::{Stft, HALF, LATENCY};
use vb_core::{ProcessContext, Processor};

/// The xorshift32 seed; shared with [`crate::noise`] so determinism is part of the contract.
const SEED: u32 = 0x9e37_79b9;

/// A phase-vocoder whisperizer (random-phase resynthesis).
pub struct Whisperize {
    mix: f32,
    rng: u32,
    stft: Stft,
}

impl Whisperize {
    /// Creates a whisperizer with dry/wet `mix` (0 = dry, 1 = fully whispered).
    #[must_use]
    pub fn new(mix: f32) -> Self {
        Self {
            mix: mix.clamp(0.0, 1.0),
            rng: SEED,
            stft: Stft::new(),
        }
    }

    /// Next uniform phase in `[0, 2π)` from the xorshift32 PRNG.
    fn next_phase(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        core::f32::consts::TAU * (x as f32 / u32::MAX as f32)
    }

    /// Replaces the analysed half-spectrum with its random-phase (whispered) form in place: each
    /// bin keeps its magnitude (doubled for the half-spectrum convention) but takes a fresh random
    /// phase.
    fn transform(&mut self) {
        for k in 0..=HALF {
            let re = self.stft.fft_work[2 * k];
            let im = self.stft.fft_work[2 * k + 1];
            let mag = 2.0 * (re * re + im * im).sqrt();
            let theta = self.next_phase();
            self.stft.fft_work[2 * k] = mag * theta.cos();
            self.stft.fft_work[2 * k + 1] = mag * theta.sin();
        }
    }
}

impl Processor for Whisperize {
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
        self.rng = SEED;
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

    const SR: f32 = 48_000.0;

    fn run(p: &mut Whisperize, input: &[f32]) -> Vec<f32> {
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
        assert_eq!(Whisperize::new(1.0).latency_frames(), LATENCY);
    }

    #[test]
    fn it_scrambles_phase_removing_tonal_coherence() {
        // Whisperization keeps the magnitude spectrum but draws a fresh random phase every frame.
        // For a pitched tone the *coherent* (phase-stable) tonal component is therefore destroyed
        // even though the energy stays in the same band: integrated over a long window the
        // coherent amplitude at the tone frequency collapses relative to the signal's actual
        // power. "Tonality" = 2·|coherent amplitude|² / mean-power is ~1 for a clean sine and
        // drops toward 0 as the phase is scrambled. (An autocorrelation-at-the-period test does
        // NOT work here: a single phase-randomised bin is still a sinusoid at that frequency, so
        // it stays periodic — the defining effect is the loss of phase coherence, not of band.)
        let n = 24_576;
        let settle = 8_192;
        let input = tone(440.0, n);

        let tonality = |xs: &[f32]| {
            let coh = mag(xs, 440.0);
            let power = xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32;
            2.0 * coh * coh / power
        };

        let dry = tonality(&input[settle..]);
        let out = run(&mut Whisperize::new(1.0), &input);
        let wet = tonality(&out[settle..]);

        assert!(dry > 0.9, "the dry 440 Hz tone is fully tonal: {dry}");
        assert!(
            wet < 0.3 * dry,
            "whisperizing destroys tonal coherence: dry {dry}, wet {wet}"
        );
    }

    #[test]
    fn it_keeps_the_spectral_band_of_the_source() {
        // Magnitude is preserved, so the energy stays in the source's frequency band: a 440 Hz
        // input whispers to noise *around 440 Hz*, with essentially nothing relocated to 5 kHz
        // where the source had no energy.
        let out = run(&mut Whisperize::new(1.0), &tone(440.0, 16_384));
        let tail = &out[8_192..];
        let near = mag(tail, 440.0);
        let far = mag(tail, 5_000.0);
        assert!(
            near > 8.0 * far,
            "energy stays in the source band: near(440) = {near}, far(5k) = {far}"
        );
    }

    #[test]
    fn generation_is_deterministic_and_reset_replays_it() {
        let input = tone(330.0, 8_192);
        let first = run(&mut Whisperize::new(1.0), &input);

        let mut b = Whisperize::new(1.0);
        assert_eq!(run(&mut b, &input), first, "same seed, same whisper");

        let mut a = Whisperize::new(1.0);
        let _ = run(&mut a, &input);
        a.reset();
        assert_eq!(run(&mut a, &input), first, "reset replays from the seed");
    }

    #[test]
    fn silence_in_silence_out() {
        let out = run(&mut Whisperize::new(1.0), &vec![0.0; 8_192]);
        assert!(out.iter().all(|s| s.abs() < 1e-5), "silence stays silent");
    }

    #[test]
    fn a_dry_mix_passes_the_input_through() {
        let input = tone(440.0, 4_096);
        let out = run(&mut Whisperize::new(0.0), &input);
        for i in LATENCY..4_096 {
            assert!(
                (out[i] - input[i - LATENCY]).abs() < 1e-6,
                "dry mix is the delayed input"
            );
        }
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let out = run(&mut Whisperize::new(1.0), &tone(220.0, 16_384));
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 8.0),
            "whispered output stays finite and bounded"
        );
    }

    #[test]
    fn set_params_updates_and_clamps() {
        let mut w = Whisperize::new(1.0);
        assert!(w.set_params(&[0.5]));
        assert!((w.mix - 0.5).abs() < 1e-6);
        assert!(w.set_params(&[9.0]));
        assert!((w.mix - 1.0).abs() < 1e-6, "mix clamps to 1");
        assert!(w.set_params(&[-9.0]));
        assert!((w.mix - 0.0).abs() < 1e-6, "mix clamps to 0");
    }
}
