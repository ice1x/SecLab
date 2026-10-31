//! Quantizer: snap a pitch CV to a tuning grid.
//!
//! Takes a frequency (in Hz) on its control input and rounds it to the nearest note of an
//! equal-tempered grid — `divisions` steps per octave, measured from `ref_hz`. Feed it a
//! drifting or continuous pitch (an LFO, a sequencer ramp, a sample-and-hold of noise) and
//! the output lands only on in-tune notes: 12 divisions is the chromatic scale, 1 is pure
//! octaves, 24 is quarter-tones. A non-positive input passes through as `0` (silence has no
//! pitch), keeping the math finite.

use seclab_core::{ProcessContext, Processor};

/// A control-rate pitch quantizer.
///
/// Input 0 is a frequency in Hz (control). Output 0 is that frequency snapped to the nearest
/// step of a `divisions`-per-octave grid anchored at `ref_hz`.
#[derive(Debug, Clone)]
pub struct Quantizer {
    ref_hz: f32,
    divisions: f32,
}

impl Quantizer {
    /// Creates a quantizer. `ref_hz` is clamped to a small positive floor and `divisions`
    /// to `>= 1` so the grid is always well-defined.
    #[must_use]
    pub fn new(ref_hz: f32, divisions: f32) -> Self {
        Self {
            ref_hz: ref_hz.max(1e-3),
            divisions: divisions.max(1.0),
        }
    }

    /// Snaps a single frequency to the grid (non-positive → `0`).
    fn snap(&self, f: f32) -> f32 {
        if f > 0.0 {
            let octaves = (f / self.ref_hz).log2();
            let step = (octaves * self.divisions).round() / self.divisions;
            self.ref_hz * 2.0_f32.powf(step)
        } else {
            0.0
        }
    }
}

impl Processor for Quantizer {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            *out = self.snap(input.get(i).copied().unwrap_or(0.0));
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(q: &mut Quantizer, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            q.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn a_slightly_sharp_note_snaps_to_the_reference() {
        // 445 Hz is within a quarter-semitone of A440 → snaps to 440.
        let out = run(&mut Quantizer::new(440.0, 12.0), &[445.0]);
        assert!((out[0] - 440.0).abs() < 1e-2, "445 → 440, got {}", out[0]);
    }

    #[test]
    fn it_snaps_to_the_nearest_semitone() {
        // ~A#4 (466.16 Hz) is one chromatic step above A440.
        let out = run(&mut Quantizer::new(440.0, 12.0), &[470.0]);
        let a_sharp = 440.0 * 2.0_f32.powf(1.0 / 12.0);
        assert!((out[0] - a_sharp).abs() < 1e-1, "470 → A#4, got {}", out[0]);
    }

    #[test]
    fn one_division_snaps_to_octaves() {
        // 660 Hz is ~0.585 octaves above 440 → rounds up to the octave at 880.
        let out = run(&mut Quantizer::new(440.0, 1.0), &[660.0]);
        assert!((out[0] - 880.0).abs() < 1e-1, "660 → 880, got {}", out[0]);
    }

    #[test]
    fn non_positive_input_is_zero() {
        let out = run(&mut Quantizer::new(440.0, 12.0), &[0.0, -5.0]);
        assert_eq!(out, vec![0.0, 0.0]);
    }

    #[test]
    fn output_lands_only_on_grid_notes() {
        // Any input maps to ref·2^(k/div); check the ratio is an exact grid step.
        let q = Quantizer::new(440.0, 12.0);
        for f in [123.4, 512.0, 999.0, 61.7] {
            let snapped = q.snap(f);
            let steps = (snapped / 440.0).log2() * 12.0;
            assert!(
                (steps - steps.round()).abs() < 1e-4,
                "{f} → off-grid {snapped}"
            );
        }
    }

    #[test]
    fn divisions_and_ref_are_clamped() {
        let q = Quantizer::new(-10.0, 0.0);
        assert!(q.ref_hz > 0.0);
        assert!((q.divisions - 1.0).abs() < 1e-6);
    }
}
