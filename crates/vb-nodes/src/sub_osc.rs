//! Sub oscillator: a band-limited square one or two octaves below the note.
//!
//! Reinforces the low end by tracking the played pitch and sounding a square wave `octaves`
//! below it — the classic trick for a fat analogue bass. It reads the same `pitch_hz` control
//! input as [`crate::PitchOsc`] and falls back to `base_hz` at rest. The square is
//! PolyBLEP-band-limited (it wraps [`crate::Oscillator`]), so even a deep sub stays clean.

use vb_core::{ProcessContext, Processor};

use crate::oscillator::{Oscillator, Waveform};

/// Below this the pitch input counts as "no note".
const MIN_HZ: f32 = 20.0;
/// Upper bound on the (already divided-down) sub frequency.
const MAX_HZ: f32 = 20_000.0;

/// A pitch-tracking sub-octave square oscillator.
///
/// Input 0 is a `pitch_hz` control value. Output 0 is a band-limited square at
/// `pitch / 2^octaves` (or `base_hz / 2^octaves` when no note is held).
#[derive(Debug, Clone)]
pub struct SubOsc {
    osc: Oscillator,
    base_hz: f32,
    /// How many octaves below the tracked pitch to sound (1 or 2).
    octaves: u32,
}

impl SubOsc {
    /// Creates a sub oscillator. `octaves` is clamped to `1..=2`.
    #[must_use]
    pub fn new(base_hz: f32, octaves: u32) -> Self {
        let octaves = octaves.clamp(1, 2);
        let divisor = (1u32 << octaves) as f32;
        Self {
            osc: Oscillator::new(Waveform::Square, (base_hz / divisor).clamp(MIN_HZ, MAX_HZ)),
            base_hz,
            octaves,
        }
    }

    /// The frequency the sub is currently tuned to (Hz).
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.osc.freq_hz()
    }
}

impl Processor for SubOsc {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let tracked = if pitch >= MIN_HZ { pitch } else { self.base_hz };
        let divisor = (1u32 << self.octaves) as f32;
        self.osc
            .set_freq_hz((tracked / divisor).clamp(MIN_HZ, MAX_HZ));
        self.osc.process(ctx, &[], outputs);
    }

    fn reset(&mut self) {
        self.osc.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(s: &mut SubOsc, pitch: Option<f32>, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0_f32; frames];
        {
            let mut slice = [&mut out[..]];
            match pitch {
                Some(p) => s.process(&ctx, &[&[p]], &mut slice),
                None => s.process(&ctx, &[], &mut slice),
            }
        }
        out
    }

    #[test]
    fn one_octave_down_halves_the_frequency() {
        let mut s = SubOsc::new(440.0, 1);
        let _ = render(&mut s, Some(440.0), 64);
        assert!(
            (s.freq_hz() - 220.0).abs() < 1e-3,
            "440 → 220, got {}",
            s.freq_hz()
        );
    }

    #[test]
    fn two_octaves_down_quarters_the_frequency() {
        let mut s = SubOsc::new(440.0, 2);
        let _ = render(&mut s, Some(440.0), 64);
        assert!(
            (s.freq_hz() - 110.0).abs() < 1e-3,
            "440 → 110, got {}",
            s.freq_hz()
        );
    }

    #[test]
    fn it_falls_back_to_base_hz_at_rest() {
        let mut s = SubOsc::new(440.0, 1);
        let _ = render(&mut s, None, 64);
        assert!((s.freq_hz() - 220.0).abs() < 1e-3, "rest → base/2");
    }

    #[test]
    fn output_is_audible_and_finite() {
        let out = render(&mut SubOsc::new(220.0, 1), Some(220.0), 4_800);
        assert!(out.iter().all(|s| s.is_finite()));
        let peak = out.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.5, "a square sub is loud, peak {peak}");
    }

    #[test]
    fn octaves_are_clamped() {
        assert_eq!(SubOsc::new(440.0, 0).octaves, 1);
        assert_eq!(SubOsc::new(440.0, 9).octaves, 2);
    }
}
