//! Pitch-tracking oscillator: an oscillator whose frequency follows a note.
//!
//! The missing link between [`crate::MidiIn`] and audible pitch. `midi-in` emits the
//! played note as an **absolute frequency** (`pitch_hz`, in Hz), but the plain
//! [`crate::Oscillator`] runs at a fixed frequency and the [`crate::Vco`] takes an
//! *exponential* (1 V/oct) control signal — neither tracks an absolute-Hz input. This node
//! reads a `pitch_hz` control input once per block and retunes its oscillator directly to
//! it, so wiring `midi-in.pitch_hz → pitch-osc.pitch` gives a keyboard-tracking voice.
//!
//! When the control input is unconnected or reads below the audio range (no note has
//! played yet), it falls back to `base_hz`, so it produces a sensible tone at rest instead
//! of silence or a DC offset.
//!
//! A `detune` offset (in semitones, fractional) transposes the tracked pitch — an octave
//! switch (±12, ±24) or a few cents of fine detune. Stacking several pitch-tracking
//! oscillators at slightly different detunes is what gives a Minimoog-style voice its
//! fatness.

use vb_core::{ProcessContext, Processor};

use crate::oscillator::{Oscillator, Waveform};

/// Lowest and highest frequency the oscillator is tuned to (Hz).
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// An oscillator that tracks a `pitch_hz` control input, transposed by `detune`
/// semitones, falling back to `base_hz` when no note is held.
#[derive(Debug, Clone)]
pub struct PitchOsc {
    osc: Oscillator,
    /// Frequency used when the pitch input carries no valid note (Hz).
    base_hz: f32,
    /// Frequency multiplier from the detune offset (`2^(semitones/12)`).
    detune_ratio: f32,
}

impl PitchOsc {
    /// Creates a pitch-tracking oscillator of `waveform`, resting at `base_hz` until a
    /// note arrives, and transposing the tracked pitch by `detune_semitones`.
    #[must_use]
    pub fn new(waveform: Waveform, base_hz: f32, detune_semitones: f32) -> Self {
        Self {
            osc: Oscillator::new(waveform, base_hz),
            base_hz,
            detune_ratio: 2.0_f32.powf(detune_semitones / 12.0),
        }
    }

    /// The frequency the oscillator is currently tuned to (Hz).
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.osc.freq_hz()
    }
}

impl Processor for PitchOsc {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        // Control-rate: retune from the pitch input's value at the block start. A value
        // below the audio range (including the 0 the note source emits before any note)
        // means "no note yet" — hold the base frequency.
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let tracked = if pitch >= MIN_HZ { pitch } else { self.base_hz };
        let freq = (tracked * self.detune_ratio).clamp(MIN_HZ, MAX_HZ);
        self.osc.set_freq_hz(freq);

        // Delegate audio generation: the oscillator ignores inputs and writes outputs[0].
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

    /// Renders a block driven by a constant pitch control value and returns the output.
    fn render(osc: &mut PitchOsc, pitch: Option<f32>, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        {
            let mut slice = [&mut out[..]];
            match pitch {
                Some(p) => osc.process(&ctx, &[&vec![p; frames]], &mut slice),
                None => osc.process(&ctx, &[], &mut slice),
            }
        }
        out
    }

    /// Counts sign changes — proportional to frequency for a simple periodic signal.
    fn zero_crossings(xs: &[f32]) -> usize {
        xs.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    }

    #[test]
    fn tracks_the_pitch_input_frequency() {
        // A higher pitch input produces a higher-frequency output (more zero crossings).
        let low = render(
            &mut PitchOsc::new(Waveform::Sine, 220.0, 0.0),
            Some(200.0),
            4_800,
        );
        let high = render(
            &mut PitchOsc::new(Waveform::Sine, 220.0, 0.0),
            Some(800.0),
            4_800,
        );
        let (lo, hi) = (zero_crossings(&low), zero_crossings(&high));
        assert!(
            hi > lo * 3,
            "4x the pitch → roughly 4x the crossings: {lo} vs {hi}"
        );
    }

    #[test]
    fn the_output_frequency_matches_the_pitch_input() {
        // 480 Hz over 4800 samples at 48 kHz ≈ 48 cycles ≈ 96 zero crossings.
        let out = render(
            &mut PitchOsc::new(Waveform::Sine, 100.0, 0.0),
            Some(480.0),
            4_800,
        );
        let crossings = zero_crossings(&out);
        assert!(
            (94..=98).contains(&crossings),
            "expected ~96 crossings for 480 Hz, got {crossings}"
        );
    }

    #[test]
    fn falls_back_to_base_when_the_pitch_input_is_silent() {
        // A zero pitch (the note source's value before any note) holds the base freq.
        let mut osc = PitchOsc::new(Waveform::Sine, 330.0, 0.0);
        render(&mut osc, Some(0.0), 64);
        assert!((osc.freq_hz() - 330.0).abs() < 1e-3);
    }

    #[test]
    fn a_missing_pitch_input_uses_the_base_frequency() {
        let mut osc = PitchOsc::new(Waveform::Sine, 261.63, 0.0);
        render(&mut osc, None, 64);
        assert!((osc.freq_hz() - 261.63).abs() < 1e-2);
    }

    #[test]
    fn a_valid_note_overrides_the_base_frequency() {
        let mut osc = PitchOsc::new(Waveform::Sine, 100.0, 0.0);
        render(&mut osc, Some(440.0), 64);
        assert!((osc.freq_hz() - 440.0).abs() < 1e-3);
    }

    #[test]
    fn detune_transposes_the_tracked_pitch() {
        // +12 semitones doubles the frequency; -12 halves it.
        let mut up = PitchOsc::new(Waveform::Sine, 100.0, 12.0);
        render(&mut up, Some(220.0), 64);
        assert!(
            (up.freq_hz() - 440.0).abs() < 1e-2,
            "up an octave: {}",
            up.freq_hz()
        );

        let mut down = PitchOsc::new(Waveform::Sine, 100.0, -12.0);
        render(&mut down, Some(220.0), 64);
        assert!(
            (down.freq_hz() - 110.0).abs() < 1e-2,
            "down an octave: {}",
            down.freq_hz()
        );

        // A few cents of fine detune nudges the frequency slightly off the note.
        let mut fine = PitchOsc::new(Waveform::Sine, 100.0, 0.1);
        render(&mut fine, Some(440.0), 64);
        assert!(
            fine.freq_hz() > 440.0 && fine.freq_hz() < 443.0,
            "fine detune: {}",
            fine.freq_hz()
        );
    }

    #[test]
    fn pitch_is_clamped_to_the_audio_range() {
        let mut osc = PitchOsc::new(Waveform::Sine, 200.0, 0.0);
        render(&mut osc, Some(50_000.0), 64);
        assert!((osc.freq_hz() - MAX_HZ).abs() < 1e-3, "clamped to max");
    }

    #[test]
    fn zero_sample_rate_is_safe() {
        let mut osc = PitchOsc::new(Waveform::Saw, 440.0, 0.0);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 8,
        };
        let mut out = [0.0_f32; 8];
        {
            let mut slice = [&mut out[..]];
            osc.process(&ctx, &[&[440.0; 8]], &mut slice);
        }
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
