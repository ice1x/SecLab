//! Compressor: a feed-forward peak dynamics processor.
//!
//! Tames the loud parts of a signal so the quiet parts can sit closer to them. A peak
//! detector tracks the input level; whenever it rises above `threshold_db`, the gain is
//! pulled down by `ratio` (a 4:1 ratio means 4 dB over the threshold becomes 1 dB over),
//! and `makeup_db` lifts the whole thing back up. `attack_ms`/`release_ms` set how fast the
//! detector chases level changes — fast attack clamps transients, slow release breathes.
//! Self-contained: the detector lives inside, so it needs no external sidechain wiring.

use seclab_core::{ProcessContext, Processor};

/// Level below which the detector is treated as silence (≈ -180 dB) to keep `log10` finite.
const FLOOR: f32 = 1e-9;

/// A single-band feed-forward peak compressor.
///
/// Input 0 is audio. Output 0 is the input scaled by a smoothly-varying gain: unity (plus
/// makeup) below the threshold, progressively attenuated above it. `ratio = 1` disables the
/// compression, leaving only the makeup gain.
#[derive(Debug, Clone)]
pub struct Compressor {
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_db: f32,
    /// Linear peak-envelope state, chased with the attack/release coefficients.
    env: f32,
}

impl Compressor {
    /// Creates a compressor. `ratio` is clamped to `>= 1`, the times to `>= 0`.
    #[must_use]
    pub fn new(
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    ) -> Self {
        Self {
            threshold_db,
            ratio: ratio.max(1.0),
            attack_ms: attack_ms.max(0.0),
            release_ms: release_ms.max(0.0),
            makeup_db,
            env: 0.0,
        }
    }

    /// One-pole smoothing coefficient for a time constant of `ms` at sample rate `sr`.
    /// Returns `0` for a zero time (instant follow).
    fn coeff(ms: f32, sr: f32) -> f32 {
        if ms <= 0.0 {
            0.0
        } else {
            (-1.0 / (ms * 0.001 * sr)).exp()
        }
    }
}

impl Processor for Compressor {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let att = Self::coeff(self.attack_ms, sr);
        let rel = Self::coeff(self.release_ms, sr);
        let slope = 1.0 - 1.0 / self.ratio;
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let rect = x.abs();
            // Peak detector: chase upward with the attack coeff, decay with release.
            let c = if rect > self.env { att } else { rel };
            self.env = c * self.env + (1.0 - c) * rect;

            let env_db = 20.0 * self.env.max(FLOOR).log10();
            let over = env_db - self.threshold_db;
            let reduction_db = if over > 0.0 { -over * slope } else { 0.0 };
            let gain = 10.0_f32.powf((reduction_db + self.makeup_db) / 20.0);
            *out = x * gain;
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(c: &mut Compressor, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            c.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn a_signal_below_threshold_passes_through() {
        // -26 dBFS constant, threshold -18 dB, no makeup: nothing to compress.
        let mut c = Compressor::new(-18.0, 4.0, 5.0, 50.0, 0.0);
        let out = run(&mut c, &[0.05; 24_000]);
        // Check the settled tail, past the attack ramp.
        for &s in &out[12_000..] {
            assert!(
                (s - 0.05).abs() < 1e-3,
                "below threshold is transparent, got {s}"
            );
        }
    }

    #[test]
    fn a_loud_signal_above_threshold_is_attenuated() {
        // 0.5 constant = -6 dBFS, threshold -18, ratio 4:1.
        let mut c = Compressor::new(-18.0, 4.0, 5.0, 50.0, 0.0);
        let out = run(&mut c, &[0.5; 24_000]);
        let settled = out[23_000];
        // env ≈ 0.5 → -6.02 dB, over = 11.98, gr = -11.98·0.75 = -8.98 dB → gain 0.356.
        let expected = 0.5 * 10.0_f32.powf(-8.98 / 20.0);
        assert!(settled < 0.5, "output is quieter than input: {settled}");
        assert!(
            (settled - expected).abs() < 0.01,
            "gain reduction matches the ratio: got {settled}, want ≈{expected}"
        );
    }

    #[test]
    fn a_higher_input_gets_more_gain_reduction() {
        let gr = |amp: f32| {
            let mut c = Compressor::new(-18.0, 4.0, 5.0, 50.0, 0.0);
            let out = run(&mut c, &[amp; 24_000]);
            // Instantaneous gain at the settled tail.
            out[23_000] / amp
        };
        // Louder input → the compressor clamps harder → smaller gain factor.
        assert!(gr(0.8) < gr(0.3), "more input, more reduction");
    }

    #[test]
    fn ratio_one_only_applies_makeup() {
        // Ratio 1 = no compression; a +6 dB makeup simply doubles the signal.
        let mut c = Compressor::new(-18.0, 1.0, 5.0, 50.0, 6.0206);
        let out = run(&mut c, &[0.3; 24_000]);
        let settled = out[23_000];
        assert!(
            (settled - 0.6).abs() < 1e-2,
            "ratio 1 + 6 dB makeup doubles: {settled}"
        );
    }

    #[test]
    fn output_stays_finite() {
        let mut c = Compressor::new(-24.0, 8.0, 1.0, 100.0, 3.0);
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.02).sin()).collect();
        let out = run(&mut c, &input);
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn ratio_and_times_are_clamped() {
        let c = Compressor::new(-18.0, 0.2, -5.0, -1.0, 0.0);
        assert!((c.ratio - 1.0).abs() < 1e-6);
        assert!(c.attack_ms >= 0.0 && c.release_ms >= 0.0);
    }
}
