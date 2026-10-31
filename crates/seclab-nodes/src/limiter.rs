//! Limiter: a brickwall ceiling on the signal's peak level.
//!
//! Where the [`crate::compressor::Compressor`] eases level down by a ratio, a limiter is
//! the hard stop: it guarantees the output never exceeds `ceiling_db`. A peak detector
//! reacts instantly when a sample would cross the ceiling and pulls the gain down just
//! enough to hold it there; `release_ms` sets how quickly the gain recovers afterwards.
//! Unlike hard clipping it scales the whole waveform rather than flattening its tips, so it
//! catches transients without the buzzy edge. Put one on the master output as a safety net.

use seclab_core::{ProcessContext, Processor};

/// A feed-forward brickwall peak limiter.
///
/// Input 0 is audio. Output 0 is the input scaled so its peak never exceeds the linear
/// ceiling: gain drops instantly when a peak would cross it and recovers over `release_ms`.
#[derive(Debug, Clone)]
pub struct Limiter {
    /// Linear ceiling the output peak is held at or below.
    ceiling: f32,
    release_ms: f32,
    /// Linear peak-envelope state (instant attack, smoothed release).
    env: f32,
    /// Current gain applied to the signal.
    gain: f32,
}

impl Limiter {
    /// Creates a limiter with the ceiling given in dBFS. `release_ms` is clamped to `>= 0`.
    #[must_use]
    pub fn new(ceiling_db: f32, release_ms: f32) -> Self {
        Self {
            ceiling: 10.0_f32.powf(ceiling_db / 20.0),
            release_ms: release_ms.max(0.0),
            env: 0.0,
            gain: 1.0,
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

impl Processor for Limiter {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let rel = Self::coeff(self.release_ms, sr);
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let rect = x.abs();
            // Peak detector: jump up to the current sample instantly, decay on release.
            self.env = if rect > self.env {
                rect
            } else {
                rel * self.env + (1.0 - rel) * rect
            };

            // Gain needed to hold the envelope at the ceiling (1.0 when already under it).
            let target = if self.env > self.ceiling {
                self.ceiling / self.env
            } else {
                1.0
            };
            // Instant attack (clamp down now so this sample can't exceed the ceiling),
            // smoothed release back toward unity.
            self.gain = if target < self.gain {
                target
            } else {
                rel * self.gain + (1.0 - rel) * target
            };
            *out = x * self.gain;
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
        self.gain = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(l: &mut Limiter, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            l.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn a_signal_below_the_ceiling_passes_unchanged() {
        // Ceiling 0 dBFS (1.0); a 0.3 constant is well under it.
        let mut l = Limiter::new(0.0, 50.0);
        let out = run(&mut l, &[0.3; 24_000]);
        for &s in &out[100..] {
            assert!((s - 0.3).abs() < 1e-4, "under ceiling is transparent: {s}");
        }
    }

    #[test]
    fn a_signal_over_the_ceiling_is_held_at_it() {
        // 2.0 constant into a 0 dBFS ceiling: output must settle to ~1.0.
        let mut l = Limiter::new(0.0, 20.0);
        let out = run(&mut l, &[2.0; 24_000]);
        let settled = out[23_000];
        assert!(
            (settled - 1.0).abs() < 1e-3,
            "held at the ceiling: {settled}"
        );
    }

    #[test]
    fn no_sample_ever_exceeds_the_ceiling() {
        // A loud sine that swings to ±2.0 must never leave the ±1.0 ceiling.
        let input: Vec<f32> = (0..48_000)
            .map(|i| 2.0 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin())
            .collect();
        let mut l = Limiter::new(0.0, 30.0);
        let out = run(&mut l, &input);
        assert!(
            out.iter().all(|&s| s.abs() <= 1.0 + 1e-4),
            "peak stays under the ceiling"
        );
    }

    #[test]
    fn a_lower_ceiling_limits_harder() {
        let peak = |db: f32| {
            let mut l = Limiter::new(db, 20.0);
            let out = run(&mut l, &[2.0; 24_000]);
            out[23_000]
        };
        // -6 dBFS ≈ 0.5, 0 dBFS = 1.0.
        assert!(peak(-6.0) < peak(0.0), "lower ceiling → lower output");
        assert!((peak(-6.0) - 0.5).abs() < 1e-2, "-6 dB ceiling ≈ 0.5");
    }

    #[test]
    fn gain_recovers_after_a_loud_burst() {
        // Loud burst then a quiet tone: once released, the quiet part passes at full level.
        let mut input = vec![2.0_f32; 4_800];
        input.extend(std::iter::repeat_n(0.2_f32, 48_000));
        let mut l = Limiter::new(0.0, 5.0);
        let out = run(&mut l, &input);
        let tail = *out.last().unwrap();
        assert!((tail - 0.2).abs() < 1e-3, "gain recovered to unity: {tail}");
    }

    #[test]
    fn release_is_clamped() {
        let l = Limiter::new(0.0, -10.0);
        assert!(l.release_ms >= 0.0);
    }

    #[test]
    fn reset_restores_unity_gain() {
        let mut l = Limiter::new(0.0, 50.0);
        let _ = run(&mut l, &[2.0; 5_000]);
        l.reset();
        assert_eq!(l.gain, 1.0);
        assert_eq!(l.env, 0.0);
    }
}
