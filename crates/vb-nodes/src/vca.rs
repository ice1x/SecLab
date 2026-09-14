//! Voltage-controlled amplifier: the node that lets a control signal shape audio level.
//!
//! Until now the library had generators, filters and math, but nothing that consumed a
//! control-rate signal to *modulate* audio — so an LFO or envelope had no audible
//! destination. The [`Vca`] closes that gap: its audio input is scaled sample-by-sample
//! by a control input, `out = in * (bias + amount * cv)`. With the control input left
//! unconnected (silence) it is a unity pass-through at `bias = 1`; wire an LFO into the
//! control input for tremolo, or an envelope for a shaped note.

use vb_core::{ProcessContext, Processor};

/// A control-modulated amplifier: `out = in * (bias + amount * cv)`.
#[derive(Debug, Clone, Copy)]
pub struct Vca {
    /// Baseline gain applied when the control signal is zero.
    pub bias: f32,
    /// How strongly the control signal swings the gain.
    pub amount: f32,
}

impl Vca {
    /// Creates a VCA with the given `bias` (gain at `cv = 0`) and modulation `amount`.
    #[must_use]
    pub fn new(bias: f32, amount: f32) -> Self {
        Self { bias, amount }
    }
}

impl Default for Vca {
    /// Unity pass-through that fully opens to the control signal: `bias = 1`,
    /// `amount = 1`.
    fn default() -> Self {
        Self::new(1.0, 1.0)
    }
}

impl Processor for Vca {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        // inputs[0] = audio, inputs[1] = control (silence when unconnected).
        let audio = inputs.first().copied().unwrap_or(&[]);
        let cv = inputs.get(1).copied().unwrap_or(&[]);
        let [out, ..] = outputs else {
            return;
        };
        for i in 0..ctx.frames.min(out.len()) {
            let x = audio.get(i).copied().unwrap_or(0.0);
            let c = cv.get(i).copied().unwrap_or(0.0);
            out[i] = x * (self.bias + self.amount * c);
        }
    }

    /// Live-updates `bias`, `amount` (registry order). The VCA is stateless, so the update is
    /// seamless — the next sample simply uses the new gain.
    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&v) = values.first() {
            self.bias = v;
        }
        if let Some(&v) = values.get(1) {
            self.amount = v;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(vca: &mut Vca, audio: &[f32], cv: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: audio.len(),
        };
        let mut out = vec![0.0; audio.len()];
        {
            let mut slice = [&mut out[..]];
            vca.process(&ctx, &[audio, cv], &mut slice);
        }
        out
    }

    #[test]
    fn silent_control_is_unity_pass_through_at_bias_one() {
        let mut vca = Vca::default();
        let audio = [0.5, -0.5, 1.0, -1.0];
        // cv all zero → out = in * (1 + 1*0) = in.
        assert_eq!(run(&mut vca, &audio, &[0.0; 4]), audio);
    }

    #[test]
    fn control_scales_the_amplitude() {
        // bias 0, amount 1 → out = in * cv (pure amplitude modulation).
        let mut vca = Vca::new(0.0, 1.0);
        let audio = [1.0, 1.0, 1.0, 1.0];
        let cv = [0.0, 0.25, 0.5, 1.0];
        assert_eq!(run(&mut vca, &audio, &cv), vec![0.0, 0.25, 0.5, 1.0]);
    }

    #[test]
    fn bias_and_amount_combine() {
        // out = in * (0.5 + 2*cv).
        let mut vca = Vca::new(0.5, 2.0);
        let audio = [2.0, 2.0];
        let cv = [0.0, 0.25]; // gains 0.5 and 1.0
        assert_eq!(run(&mut vca, &audio, &cv), vec![1.0, 2.0]);
    }

    #[test]
    fn a_missing_control_input_is_treated_as_silence() {
        let mut vca = Vca::default();
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: 3,
        };
        let audio = [1.0, 2.0, 3.0];
        let mut out = vec![0.0; 3];
        {
            let mut slice = [&mut out[..]];
            // Only the audio input is present.
            vca.process(&ctx, &[&audio], &mut slice);
        }
        // cv absent → 0 → out = in * bias = in.
        assert_eq!(out, audio);
    }
}
