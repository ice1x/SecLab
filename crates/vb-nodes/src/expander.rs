//! Downward expander: a feed-forward dynamics processor that pushes the quiet parts quieter.
//!
//! The mirror image of [`crate::Compressor`]. A peak detector tracks the input level; whenever
//! it falls *below* `threshold_db`, the gain is pulled down so soft sounds (hiss, bleed, the
//! tail of a note) drop away, widening the dynamic range. Unlike [`crate::Gate`]'s hard
//! open/shut, the attenuation grows smoothly with `ratio`: each dB the input sits below the
//! threshold costs `ratio` dB of output, so `ratio = 1` is transparent and larger values
//! expand harder. `range_db` caps how far the gain can fall (a floor, so the expander never
//! mutes completely). `attack_ms`/`release_ms` set how fast the detector tracks level.
//!
//! Static curve (dB), with threshold `T` and ratio `R`, for an input level `x < T`:
//! `output = T + R·(x − T)`, so the applied gain is `(R − 1)·(x − T)` dB — zero at the
//! threshold, increasingly negative below it. Above the threshold the signal passes unchanged.
//!
//! Primary sources: MathWorks `expander` block and DSP Concepts `DownwardExpanderCore`.

use vb_core::{ProcessContext, Processor};

/// Level below which the detector is treated as silence (≈ -180 dB) to keep `log10` finite.
const FLOOR: f32 = 1e-9;

/// A single-band feed-forward downward expander.
///
/// Input 0 is audio. Output 0 is the input scaled by a smoothly-varying gain: unity at and
/// above the threshold, progressively attenuated below it (down to `-range_db`). `ratio = 1`
/// disables the expansion.
#[derive(Debug, Clone)]
pub struct Expander {
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    range_db: f32,
    /// Linear peak-envelope state, chased with the attack/release coefficients.
    env: f32,
}

impl Expander {
    /// Creates a downward expander. `ratio` is clamped to `>= 1`, the times and `range_db`
    /// to `>= 0`.
    #[must_use]
    pub fn new(
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        range_db: f32,
    ) -> Self {
        Self {
            threshold_db,
            ratio: ratio.max(1.0),
            attack_ms: attack_ms.max(0.0),
            release_ms: release_ms.max(0.0),
            range_db: range_db.max(0.0),
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

impl Processor for Expander {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let att = Self::coeff(self.attack_ms, sr);
        let rel = Self::coeff(self.release_ms, sr);
        let slope = self.ratio - 1.0;
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let rect = x.abs();
            // Peak detector: chase upward with the attack coeff, decay with release.
            let c = if rect > self.env { att } else { rel };
            self.env = c * self.env + (1.0 - c) * rect;

            let env_db = 20.0 * self.env.max(FLOOR).log10();
            let under = env_db - self.threshold_db;
            // Below threshold (under < 0): attenuate by (ratio − 1)·under, floored at −range_db.
            let reduction_db = if under < 0.0 {
                (slope * under).max(-self.range_db)
            } else {
                0.0
            };
            let gain = 10.0_f32.powf(reduction_db / 20.0);
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

    fn run(e: &mut Expander, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            e.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn a_signal_above_threshold_passes_through() {
        // 0.5 = -6 dBFS, threshold -18: above, so nothing is expanded.
        let mut e = Expander::new(-18.0, 2.0, 5.0, 50.0, 40.0);
        let out = run(&mut e, &[0.5; 24_000]);
        for &s in &out[12_000..] {
            assert!(
                (s - 0.5).abs() < 1e-3,
                "above threshold is transparent, got {s}"
            );
        }
    }

    #[test]
    fn a_quiet_signal_below_threshold_is_attenuated_by_the_ratio() {
        // 0.05 = -26.02 dBFS, threshold -18, ratio 2:1, range 40.
        let mut e = Expander::new(-18.0, 2.0, 5.0, 50.0, 40.0);
        let out = run(&mut e, &[0.05; 24_000]);
        let settled = out[23_000];
        // under = -26.02 - (-18) = -8.02; reduction = (2-1)·-8.02 = -8.02 dB → gain 0.3975.
        let expected = 0.05 * 10.0_f32.powf(-8.02 / 20.0);
        assert!(settled < 0.05, "below threshold is attenuated: {settled}");
        assert!(
            (settled - expected).abs() < 1e-3,
            "attenuation matches the ratio: got {settled}, want ≈{expected}"
        );
    }

    #[test]
    fn a_lower_input_gets_more_attenuation() {
        let gain = |amp: f32| {
            let mut e = Expander::new(-18.0, 2.0, 5.0, 50.0, 60.0);
            let out = run(&mut e, &[amp; 24_000]);
            out[23_000] / amp
        };
        // Quieter input (further below threshold) → smaller gain factor.
        assert!(gain(0.02) < gain(0.05), "lower input, more expansion");
    }

    #[test]
    fn ratio_one_is_transparent() {
        // Ratio 1 → no expansion anywhere, even well below the threshold.
        let mut e = Expander::new(-18.0, 1.0, 5.0, 50.0, 40.0);
        let out = run(&mut e, &[0.02; 24_000]);
        for &s in &out[12_000..] {
            assert!((s - 0.02).abs() < 1e-4, "ratio 1 passes through, got {s}");
        }
    }

    #[test]
    fn range_caps_the_attenuation() {
        // Very quiet input (-60 dBFS) with a strong ratio would exceed the range; the
        // reduction is floored at -range_db, so the gain never drops below 10^(-range/20).
        let range_db = 24.0;
        let mut e = Expander::new(-18.0, 8.0, 5.0, 50.0, range_db);
        let out = run(&mut e, &[0.001; 24_000]);
        let gain = out[23_000] / 0.001;
        let floor = 10.0_f32.powf(-range_db / 20.0);
        assert!(
            gain >= floor - 1e-4,
            "gain is floored at the range: got {gain}, floor {floor}"
        );
        assert!(
            (gain - floor).abs() < 1e-3,
            "a deep-below input hits exactly the range floor: {gain} vs {floor}"
        );
    }

    #[test]
    fn output_stays_finite() {
        let mut e = Expander::new(-24.0, 6.0, 1.0, 100.0, 50.0);
        let input: Vec<f32> = (0..48_000).map(|i| 0.1 * (i as f32 * 0.02).sin()).collect();
        let out = run(&mut e, &input);
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn ratio_times_and_range_are_clamped() {
        let e = Expander::new(-18.0, 0.2, -5.0, -1.0, -3.0);
        assert!((e.ratio - 1.0).abs() < 1e-6);
        assert!(e.attack_ms >= 0.0 && e.release_ms >= 0.0 && e.range_db >= 0.0);
    }
}
