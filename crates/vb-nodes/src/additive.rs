//! Additive oscillator: a tone built by summing harmonics.
//!
//! Instead of shaping one waveform, this stacks `partials` sine harmonics of a tracked pitch,
//! their amplitudes rolling off as `1/k^tilt` (k = harmonic number). `tilt = 1` gives the
//! sawtooth-like `1/k` spectrum, `0` an equal-amplitude "organ" buzz, `2` a soft, mellow
//! tone. The sum is band-limited by construction — only harmonics below Nyquist are added —
//! so it never aliases however high it is played. Reads a `pitch_hz` control input and falls
//! back to `base_hz` when no note is held, exactly like [`crate::PitchOsc`].

use vb_core::{ProcessContext, Processor};

/// Below this the pitch input counts as "no note" (the 0 a note source emits at rest).
const MIN_HZ: f32 = 20.0;
/// Upper bound on the tracked pitch.
const MAX_HZ: f32 = 20_000.0;

/// A band-limited additive (harmonic-sum) oscillator.
///
/// Input 0 is a `pitch_hz` control value. Output 0 is the sum of up to `partials` sine
/// harmonics with `1/k^tilt` amplitudes, normalised so the peak stays within `[-1, 1]`.
#[derive(Debug, Clone)]
pub struct Additive {
    base_hz: f32,
    partials: u32,
    tilt: f32,
    phase: f32,
}

impl Additive {
    /// Creates an additive oscillator. `partials` is clamped to `>= 1`, `tilt` to `>= 0`.
    #[must_use]
    pub fn new(base_hz: f32, partials: u32, tilt: f32) -> Self {
        Self {
            base_hz,
            partials: partials.max(1),
            tilt: tilt.max(0.0),
            phase: 0.0,
        }
    }
}

impl Processor for Additive {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        // Control-rate pitch tracking (see PitchOsc): hold base_hz below the audio range.
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let freq = if pitch >= MIN_HZ { pitch } else { self.base_hz }.clamp(MIN_HZ, MAX_HZ);
        let dt = freq / sr;

        // Only harmonics whose frequency stays below Nyquist — this is the band limit.
        let max_k = ((sr * 0.5) / freq).floor().max(1.0) as u32;
        let kmax = self.partials.min(max_k);

        // Normalise by the sum of amplitudes so |output| <= 1 (|Σ aₖ·sin| <= Σ aₖ).
        let amp = |k: u32| (k as f32).powf(-self.tilt);
        let norm: f32 = (1..=kmax).map(amp).sum::<f32>().max(1e-9);

        for out in outputs[0].iter_mut().take(ctx.frames) {
            let mut acc = 0.0;
            for k in 1..=kmax {
                acc += amp(k) * (core::f32::consts::TAU * k as f32 * self.phase).sin();
            }
            *out = acc / norm;
            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(a: &mut Additive, pitch: Option<f32>, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0_f32; frames];
        {
            let mut slice = [&mut out[..]];
            match pitch {
                Some(p) => a.process(&ctx, &[&[p]], &mut slice),
                None => a.process(&ctx, &[], &mut slice),
            }
        }
        out
    }

    /// Total variation divided by RMS — a scale-invariant proxy for spectral richness.
    /// Amplitude-normalisation cancels out, but harmonics still raise variation faster than
    /// RMS (a k-th harmonic contributes k× the slope), so a brighter tone scores higher.
    fn roughness(xs: &[f32]) -> f32 {
        let tv: f32 = xs.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
        let rms = (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt();
        tv / rms.max(1e-9)
    }

    #[test]
    fn one_partial_is_a_pure_sine() {
        let out = render(&mut Additive::new(440.0, 1, 1.0), None, 480);
        for (i, &s) in out.iter().enumerate() {
            let want = (core::f32::consts::TAU * 440.0 * i as f32 / SR).sin();
            assert!((s - want).abs() < 1e-4, "1 partial = sine at i={i}");
        }
    }

    #[test]
    fn more_partials_are_spectrally_richer() {
        let simple = render(&mut Additive::new(220.0, 1, 1.0), None, 4_800);
        let rich = render(&mut Additive::new(220.0, 16, 1.0), None, 4_800);
        assert!(
            roughness(&rich) > roughness(&simple),
            "adding harmonics increases roughness (TV/RMS): {} vs {}",
            roughness(&simple),
            roughness(&rich)
        );
    }

    #[test]
    fn it_tracks_the_pitch_input() {
        let low = render(&mut Additive::new(110.0, 8, 1.0), Some(220.0), 4_800);
        let high = render(&mut Additive::new(110.0, 8, 1.0), Some(880.0), 4_800);
        let zc = |xs: &[f32]| xs.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!(zc(&high) > zc(&low), "higher pitch → more cycles");
    }

    #[test]
    fn output_stays_bounded_and_finite() {
        // A very high note leaves room for only a few harmonics; must not blow up.
        let out = render(&mut Additive::new(440.0, 32, 0.0), Some(10_000.0), 2_000);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            out.iter().all(|s| s.abs() <= 1.0001),
            "normalised to [-1, 1]"
        );
    }

    #[test]
    fn parameters_are_clamped() {
        let a = Additive::new(440.0, 0, -1.0);
        assert_eq!(a.partials, 1);
        assert!(a.tilt >= 0.0);
    }
}
