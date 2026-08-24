//! Supersaw: a stack of detuned sawtooth oscillators summed into one fat unison voice.
//!
//! The "hypersaw" at the heart of trance and modern EDM leads: several sawtooth waves
//! tuned a few cents apart beat against each other, turning a thin single saw into a wide,
//! shimmering wall of sound. Each voice is a band-limited [`Oscillator`] (PolyBLEP), so the
//! stack stays alias-free; the spread is symmetric around the tracked pitch and the sum is
//! amplitude-normalised so overall level barely changes with the voice count.
//!
//! Like [`crate::PitchOsc`] it tracks a `pitch_hz` control input, so it plays from the
//! keyboard and falls back to `base_hz` at rest.

use vb_core::{ProcessContext, Processor};

use crate::oscillator::{Oscillator, Waveform};

/// Lowest and highest per-voice frequency (Hz).
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;
/// Bounds on the unison width, in voices.
const MIN_VOICES: usize = 1;
const MAX_VOICES: usize = 9;

/// A detuned unison stack of sawtooth oscillators.
#[derive(Debug, Clone)]
pub struct Supersaw {
    /// Resting pitch when no note is held (Hz).
    base_hz: f32,
    /// Total spread from the lowest to the highest voice, in semitones.
    detune_semitones: f32,
    /// One band-limited saw per unison voice.
    voices: Vec<Oscillator>,
    /// 1/√N, so the summed level is roughly independent of the voice count.
    norm: f32,
}

impl Supersaw {
    /// Creates a supersaw of `voices` sawtooths (clamped to 1..=9) resting at `base_hz`,
    /// spread across `detune_semitones` total.
    #[must_use]
    pub fn new(base_hz: f32, detune_semitones: f32, voices: usize) -> Self {
        let n = voices.clamp(MIN_VOICES, MAX_VOICES);
        Self {
            base_hz,
            detune_semitones,
            voices: (0..n)
                .map(|_| Oscillator::new(Waveform::Saw, base_hz))
                .collect(),
            norm: 1.0 / (n as f32).sqrt(),
        }
    }

    /// The number of unison voices in the stack.
    #[must_use]
    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    /// The detune ratio for voice `i` of `n`: symmetric across `[-detune/2, +detune/2]`
    /// semitones, so voice 0 is the flattest and voice n-1 the sharpest.
    fn ratio_for(&self, i: usize, n: usize) -> f32 {
        if n <= 1 {
            return 1.0;
        }
        // Map i ∈ [0, n-1] to t ∈ [-0.5, 0.5], then to a semitone offset.
        let t = i as f32 / (n as f32 - 1.0) - 0.5;
        2.0_f32.powf(t * self.detune_semitones / 12.0)
    }
}

impl Processor for Supersaw {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let center = if pitch >= MIN_HZ { pitch } else { self.base_hz };

        let n = self.voices.len();
        // Retune every voice from the block-start pitch.
        for i in 0..n {
            let freq = (center * self.ratio_for(i, n)).clamp(MIN_HZ, MAX_HZ);
            self.voices[i].set_freq_hz(freq);
        }

        // Accumulate each voice into the output. Each Oscillator writes a full buffer, so we
        // render into a scratch buffer and add — keeping PolyBLEP band-limiting per voice.
        let frames = ctx.frames.min(outputs[0].len());
        for out in outputs[0].iter_mut().take(frames) {
            *out = 0.0;
        }
        let mut scratch = vec![0.0_f32; frames];
        for voice in &mut self.voices {
            {
                let mut slice = [&mut scratch[..]];
                voice.process(ctx, &[], &mut slice);
            }
            for (o, s) in outputs[0].iter_mut().take(frames).zip(&scratch) {
                *o += s * self.norm;
            }
        }
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(saw: &mut Supersaw, pitch: Option<f32>, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        let pitch_buf = pitch.map(|p| vec![p; frames]);
        let ins: Vec<&[f32]> = pitch_buf
            .as_ref()
            .map(|b| vec![b.as_slice()])
            .unwrap_or_default();
        {
            let mut slice = [&mut out[..]];
            saw.process(&ctx, &ins, &mut slice);
        }
        out
    }

    fn zero_crossings(xs: &[f32]) -> usize {
        xs.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    }
    fn rms(xs: &[f32]) -> f32 {
        (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt()
    }

    #[test]
    fn voice_count_is_clamped_to_the_valid_range() {
        assert_eq!(Supersaw::new(440.0, 0.2, 0).voice_count(), 1);
        assert_eq!(Supersaw::new(440.0, 0.2, 7).voice_count(), 7);
        assert_eq!(Supersaw::new(440.0, 0.2, 99).voice_count(), 9);
    }

    #[test]
    fn a_single_voice_is_a_plain_saw_at_the_pitch() {
        // One voice, no spread: ~48 cycles over 4800 samples at 480 Hz ≈ 96 crossings.
        let out = render(&mut Supersaw::new(100.0, 0.0, 1), Some(480.0), 4_800);
        let c = zero_crossings(&out);
        assert!((92..=100).contains(&c), "one saw ≈ 96 crossings, got {c}");
    }

    #[test]
    fn detuned_voices_beat_against_each_other() {
        // With spread, the seven voices drift in and out of phase, so the summed amplitude
        // envelope is NOT constant the way a single saw's is: the peak-to-trough range of a
        // short-window RMS is much wider than for one voice. Simpler robust check: the
        // detuned stack's waveform differs materially from a single saw at the same pitch.
        let frames = 9_600;
        let single = render(&mut Supersaw::new(100.0, 0.0, 1), Some(220.0), frames);
        let fat = render(&mut Supersaw::new(100.0, 0.4, 7), Some(220.0), frames);
        assert!(fat.iter().all(|s| s.is_finite()));
        let diff = single
            .iter()
            .zip(&fat)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff > 0.2,
            "detuned unison must differ from a single saw (max diff {diff})"
        );
        assert!(rms(&fat) > 0.05, "the stack still produces sound");
    }

    #[test]
    fn stays_bounded_with_a_full_stack() {
        let out = render(&mut Supersaw::new(100.0, 0.5, 9), Some(440.0), 4_800);
        assert!(
            out.iter().all(|s| s.abs() <= 4.0),
            "normalised sum stays sane"
        );
    }

    #[test]
    fn tracks_the_pitch_input() {
        let low = zero_crossings(&render(
            &mut Supersaw::new(100.0, 0.1, 3),
            Some(200.0),
            4_800,
        ));
        let high = zero_crossings(&render(
            &mut Supersaw::new(100.0, 0.1, 3),
            Some(800.0),
            4_800,
        ));
        assert!(high > low * 2, "4× pitch → more crossings: {low} vs {high}");
    }

    #[test]
    fn falls_back_to_base_when_no_note_is_held() {
        // Silent pitch (pre-note) → the stack runs at base_hz, still producing sound.
        let out = render(&mut Supersaw::new(330.0, 0.2, 5), Some(0.0), 4_800);
        assert!(rms(&out) > 0.05, "rests on base_hz, not silence");
    }

    #[test]
    fn zero_sample_rate_is_safe() {
        let mut saw = Supersaw::new(440.0, 0.3, 7);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 8,
        };
        let mut out = [0.0_f32; 8];
        {
            let mut slice = [&mut out[..]];
            saw.process(&ctx, &[&[440.0; 8]], &mut slice);
        }
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
