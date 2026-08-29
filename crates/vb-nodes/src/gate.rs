//! Noise gate: mute the signal while it sits below a threshold.
//!
//! A peak detector tracks the input level; while it stays under `threshold_db` the gate
//! closes and the output fades to silence, and when the signal rises back above the
//! threshold the gate opens again. `attack_ms` sets how fast it opens (fast = snappy),
//! `release_ms` how fast it closes (slow = a natural tail). It is the mirror image of the
//! [`crate::compressor::Compressor`]: instead of taming the loud parts it removes the quiet
//! ones — hum, hiss, or bleed between notes.

use vb_core::{ProcessContext, Processor};

/// Level below which the detector is treated as silence (≈ -180 dB) to keep `log10` finite.
const FLOOR: f32 = 1e-9;

/// A feed-forward noise gate.
///
/// Input 0 is audio. Output 0 is the input multiplied by a gain that smoothly opens to
/// unity while the level is above `threshold_db` and closes to zero below it.
#[derive(Debug, Clone)]
pub struct Gate {
    threshold_db: f32,
    attack_ms: f32,
    release_ms: f32,
    /// Linear peak-envelope state (tracks the input level).
    env: f32,
    /// Current gate gain, ramped toward the open/closed target.
    gain: f32,
}

impl Gate {
    /// Creates a gate. The times are clamped to `>= 0`.
    #[must_use]
    pub fn new(threshold_db: f32, attack_ms: f32, release_ms: f32) -> Self {
        Self {
            threshold_db,
            attack_ms: attack_ms.max(0.0),
            release_ms: release_ms.max(0.0),
            env: 0.0,
            gain: 0.0,
        }
    }

    /// One-pole smoothing coefficient for a time constant of `ms` at sample rate `sr`.
    fn coeff(ms: f32, sr: f32) -> f32 {
        if ms <= 0.0 {
            0.0
        } else {
            (-1.0 / (ms * 0.001 * sr)).exp()
        }
    }
}

impl Processor for Gate {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let att = Self::coeff(self.attack_ms, sr);
        let rel = Self::coeff(self.release_ms, sr);
        // The detector follows level quickly so the open/close decision is prompt; the
        // audible attack/release lives in the gain ramp below.
        let det = Self::coeff(1.0, sr);
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let rect = x.abs();
            self.env = det * self.env + (1.0 - det) * rect;

            let env_db = 20.0 * self.env.max(FLOOR).log10();
            let target = if env_db >= self.threshold_db {
                1.0
            } else {
                0.0
            };
            // Ramp toward the target: attack coeff while opening, release while closing.
            let c = if target > self.gain { att } else { rel };
            self.gain = c * self.gain + (1.0 - c) * target;
            *out = x * self.gain;
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
        self.gain = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(g: &mut Gate, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            g.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn a_loud_signal_passes_once_the_gate_opens() {
        // 0.5 = -6 dBFS, threshold -24 dB: well above, gate opens to unity.
        let mut g = Gate::new(-24.0, 1.0, 20.0);
        let out = run(&mut g, &[0.5; 24_000]);
        let settled = out[23_000];
        assert!(
            (settled - 0.5).abs() < 1e-3,
            "open gate is transparent: {settled}"
        );
    }

    #[test]
    fn a_quiet_signal_below_threshold_is_muted() {
        // 0.01 ≈ -40 dBFS, threshold -24 dB: below, gate stays closed.
        let mut g = Gate::new(-24.0, 1.0, 20.0);
        let out = run(&mut g, &[0.01; 24_000]);
        let settled = out[23_000];
        assert!(settled.abs() < 1e-4, "closed gate mutes: {settled}");
    }

    #[test]
    fn the_gate_closes_when_the_signal_drops_below_threshold() {
        // Loud for the first half, silent for the second: the tail must fade out.
        let mut input = vec![0.5_f32; 24_000];
        input.extend(std::iter::repeat_n(0.0_f32, 24_000));
        let mut g = Gate::new(-24.0, 1.0, 10.0);
        let out = run(&mut g, &input);
        assert!((out[23_000] - 0.5).abs() < 1e-2, "open while loud");
        assert!(out[47_999].abs() < 1e-3, "closed after silence");
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let mut g = Gate::new(-30.0, 2.0, 50.0);
        let input: Vec<f32> = (0..48_000).map(|i| 0.8 * (i as f32 * 0.02).sin()).collect();
        let out = run(&mut g, &input);
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 0.8 + 1e-6));
    }

    #[test]
    fn times_are_clamped() {
        let g = Gate::new(-24.0, -5.0, -1.0);
        assert!(g.attack_ms >= 0.0 && g.release_ms >= 0.0);
    }

    #[test]
    fn reset_closes_the_gate() {
        let mut g = Gate::new(-24.0, 1.0, 20.0);
        let _ = run(&mut g, &[0.5; 5_000]);
        g.reset();
        assert_eq!(g.gain, 0.0);
        assert_eq!(g.env, 0.0);
    }
}
