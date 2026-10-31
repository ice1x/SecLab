//! Formant (vowel) filter: shapes any input into a **vocal** timbre by resonating three
//! band-pass "formants" at the frequencies the human vocal tract produces for a given vowel.
//!
//! A vowel is defined acoustically by the first three formant frequencies (F1, F2, F3). This
//! node runs three parallel RBJ band-pass biquads tuned to those frequencies and sums them, so
//! feeding it a buzzy source (a saw, a pulse, noise) makes it "speak" that vowel. The `vowel`
//! parameter (summed with a `vowel` CV input and clamped to `[0, 1]`) **morphs** continuously
//! across the five cardinal vowels **a → e → i → o → u** by interpolating the formant table,
//! so an envelope or LFO on the CV input sweeps the mouth shape. `resonance` scales the Q of
//! all three formants — higher is sharper and more nasal.
//!
//! The formant frequencies are the classic sung-vowel values; the per-formant gains fall off
//! from F1 to F3 as they do in a real voice, so F1 dominates and the higher formants colour it.

use seclab_core::{ProcessContext, Processor};

/// The five cardinal vowels, F1/F2/F3 in Hz (classic sung-vowel formant values).
const VOWELS: [[f32; 3]; 5] = [
    [730.0, 1090.0, 2440.0], // a
    [530.0, 1840.0, 2480.0], // e
    [270.0, 2290.0, 3010.0], // i
    [570.0, 840.0, 2410.0],  // o
    [300.0, 870.0, 2240.0],  // u
];

/// Per-formant output gains: F1 loudest, higher formants progressively quieter.
const GAINS: [f32; 3] = [1.0, 0.55, 0.25];

/// Nominal bandwidths (Hz) of the three formants at `resonance == 1`; Q = freq / bandwidth.
const BANDWIDTHS: [f32; 3] = [90.0, 110.0, 160.0];

/// One RBJ band-pass biquad (constant 0 dB peak gain), retuned per block.
#[derive(Debug, Clone, Default)]
struct BandPass {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BandPass {
    /// Processes one sample at centre `freq` with resonance `q` and the given sample rate.
    fn tick(&mut self, xn: f32, freq: f32, q: f32, sample_rate: f32) -> f32 {
        let f0 = freq.clamp(1.0, 0.49 * sample_rate);
        let w0 = core::f32::consts::TAU * (f0 / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(1e-4));
        let a0 = 1.0 + alpha;
        let b0 = alpha / a0;
        let b2 = -alpha / a0;
        let a1 = -2.0 * cos / a0;
        let a2 = (1.0 - alpha) / a0;

        let yn = b0 * xn + b2 * self.x2 - a1 * self.y1 - a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = xn;
        self.y2 = self.y1;
        self.y1 = yn;
        yn
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A three-formant vocal filter morphing across the five cardinal vowels.
#[derive(Debug, Clone)]
pub struct Formant {
    /// Vowel position in `[0, 1]`: 0 = a … 1 = u.
    vowel: f32,
    /// Q scaling for all three formants (higher = sharper / more nasal).
    resonance: f32,
    bands: [BandPass; 3],
}

impl Formant {
    /// Creates a formant filter resting at vowel position `vowel` with the given `resonance`.
    #[must_use]
    pub fn new(vowel: f32, resonance: f32) -> Self {
        Self {
            vowel: vowel.clamp(0.0, 1.0),
            resonance: resonance.max(0.1),
            bands: Default::default(),
        }
    }

    /// Interpolates the three formant frequencies for vowel position `v` in `[0, 1]`.
    fn formants_at(v: f32) -> [f32; 3] {
        let p = v.clamp(0.0, 1.0) * (VOWELS.len() - 1) as f32;
        let a = p.floor() as usize;
        let b = (a + 1).min(VOWELS.len() - 1);
        let t = p - a as f32;
        let mut out = [0.0; 3];
        for i in 0..3 {
            out[i] = VOWELS[a][i] + (VOWELS[b][i] - VOWELS[a][i]) * t;
        }
        out
    }
}

impl Processor for Formant {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        let vowel_cv = inputs
            .get(1)
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let Some(out) = outputs.first_mut() else {
            return;
        };
        if ctx.sample_rate <= 0.0 {
            for (o, x) in out.iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        // Control-rate: resolve the morphed formant frequencies and per-band Q for this block.
        let v = (self.vowel + vowel_cv).clamp(0.0, 1.0);
        let freqs = Self::formants_at(v);
        let qs = [
            freqs[0] / BANDWIDTHS[0] * self.resonance,
            freqs[1] / BANDWIDTHS[1] * self.resonance,
            freqs[2] / BANDWIDTHS[2] * self.resonance,
        ];

        for n in 0..ctx.frames {
            let xn = input.get(n).copied().unwrap_or(0.0);
            let mut acc = 0.0;
            for i in 0..3 {
                acc += self.bands[i].tick(xn, freqs[i], qs[i], ctx.sample_rate) * GAINS[i];
            }
            if let Some(o) = out.get_mut(n) {
                *o = acc;
            }
        }
    }

    fn reset(&mut self) {
        for b in &mut self.bands {
            b.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: &mut Formant, input: &[f32], vowel_cv: f32) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: input.len(),
        };
        let cv = vec![vowel_cv; input.len()];
        let mut out = vec![0.0; input.len()];
        {
            let ins: [&[f32]; 2] = [input, &cv];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            f.process(&ctx, &ins, &mut outs);
        }
        out
    }

    /// A steady sine at `hz`, one second's worth capped at `frames`.
    fn sine(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (core::f32::consts::TAU * hz * n as f32 / 48_000.0).sin())
            .collect()
    }

    /// RMS of the settled tail (skip the filter's transient).
    fn tail_rms(v: &[f32]) -> f32 {
        let t = &v[v.len() / 2..];
        (t.iter().map(|s| s * s).sum::<f32>() / t.len() as f32).sqrt()
    }

    #[test]
    fn passes_a_finite_signal() {
        let mut f = Formant::new(0.0, 1.0);
        let out = run(&mut f, &sine(730.0, 4096), 0.0);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            tail_rms(&out) > 0.01,
            "an on-formant tone must pass audibly"
        );
    }

    #[test]
    fn resonates_on_the_first_formant() {
        // Vowel "a" has F1 = 730 Hz. A 730 Hz tone must pass far louder than a 6 kHz tone
        // (well above the highest formant), i.e. the filter really is band-pass shaped.
        let mut on = Formant::new(0.0, 1.0);
        let mut off = Formant::new(0.0, 1.0);
        let on_rms = tail_rms(&run(&mut on, &sine(730.0, 8192), 0.0));
        let off_rms = tail_rms(&run(&mut off, &sine(6000.0, 8192), 0.0));
        assert!(
            on_rms > off_rms * 4.0,
            "F1 tone must dominate an off-formant tone: on {on_rms}, off {off_rms}"
        );
    }

    #[test]
    fn different_vowels_pass_different_frequencies() {
        // "i" has a high second formant (F2 = 2290 Hz); "u" has a low one (F2 = 870 Hz).
        // A 2290 Hz probe must pass louder through "i" (vowel = 0.5) than through "u"
        // (vowel = 1.0), proving the vowel morph really retunes the formants.
        let probe = sine(2290.0, 8192);
        let mut vi = Formant::new(0.5, 1.0);
        let mut vu = Formant::new(1.0, 1.0);
        let i_rms = tail_rms(&run(&mut vi, &probe, 0.0));
        let u_rms = tail_rms(&run(&mut vu, &probe, 0.0));
        assert!(
            i_rms > u_rms * 2.0,
            "the /i/ formant must pass 2.29 kHz more than /u/: i {i_rms}, u {u_rms}"
        );
    }

    #[test]
    fn vowel_cv_adds_to_the_parameter() {
        // Param at "a" (0.0) with a CV of +1.0 must respond like the "u" vowel: a low-F2
        // probe (870 Hz, /u/'s F2) passes louder once the CV pushes the morph to /u/.
        let probe = sine(870.0, 8192);
        let mut at_a = Formant::new(0.0, 1.0);
        let mut swept = Formant::new(0.0, 1.0);
        let a_rms = tail_rms(&run(&mut at_a, &probe, 0.0));
        let u_rms = tail_rms(&run(&mut swept, &probe, 1.0));
        assert!(
            u_rms > a_rms,
            "vowel CV must move the formants toward /u/: a {a_rms}, swept {u_rms}"
        );
    }

    #[test]
    fn reset_clears_the_filter_state() {
        let mut f = Formant::new(0.0, 1.0);
        run(&mut f, &sine(730.0, 4096), 0.0);
        f.reset();
        let out = run(&mut f, &vec![0.0; 512], 0.0);
        assert_eq!(
            out.iter().map(|s| s * s).sum::<f32>(),
            0.0,
            "reset then silence must be silent"
        );
    }
}
