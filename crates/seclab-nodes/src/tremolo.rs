//! Tremolo: periodic amplitude modulation by an internal LFO.
//!
//! A sine LFO at `rate_hz` scales the input's level, `depth` setting how deep the dips go
//! (0 = untouched, 1 = full modulation from unity down to silence). The vintage "shimmer"
//! of an amp tremolo or a rhythmic pulsing pad. Self-contained — the LFO lives inside — so it
//! needs no external modulation wiring.

use seclab_core::{ProcessContext, Processor};

/// A sine-LFO amplitude modulator.
///
/// Input 0 is audio. Output 0 is `in · (1 - depth·(0.5 - 0.5·sin))`, so the gain sweeps
/// between `1` and `1 - depth` at `rate_hz`. `depth = 0` passes the input through.
#[derive(Debug, Clone)]
pub struct Tremolo {
    rate_hz: f32,
    depth: f32,
    phase: f32,
}

impl Tremolo {
    /// Creates a tremolo at `rate_hz` with modulation `depth` (clamped to `0..=1`).
    #[must_use]
    pub fn new(rate_hz: f32, depth: f32) -> Self {
        Self {
            rate_hz,
            depth: depth.clamp(0.0, 1.0),
            phase: 0.0,
        }
    }
}

impl Processor for Tremolo {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let dt = self.rate_hz / sr;
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            // Unipolar LFO in [0, 1]; gain = 1 at the peak, 1 - depth at the trough.
            let lfo = 0.5 - 0.5 * (core::f32::consts::TAU * self.phase).sin();
            let gain = 1.0 - self.depth * lfo;
            *out = x * gain;
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

    fn run(t: &mut Tremolo, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            t.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn depth_zero_passes_the_input_through() {
        let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = run(&mut Tremolo::new(5.0, 0.0), &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "depth 0 is transparent");
        }
    }

    #[test]
    fn it_modulates_the_amplitude_between_unity_and_one_minus_depth() {
        // A DC input of 1.0 makes the output equal the gain envelope directly.
        let out = run(&mut Tremolo::new(50.0, 0.8), &[1.0; 4_800]);
        let max = out.iter().copied().fold(f32::MIN, f32::max);
        let min = out.iter().copied().fold(f32::MAX, f32::min);
        assert!((max - 1.0).abs() < 1e-2, "peak gain ≈ 1, got {max}");
        assert!(
            (min - 0.2).abs() < 1e-2,
            "trough gain ≈ 1 - depth = 0.2, got {min}"
        );
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn a_higher_rate_modulates_faster() {
        // Count gain-envelope minima as a proxy for LFO rate: 4x rate → ~4x troughs.
        let slow = run(&mut Tremolo::new(10.0, 1.0), &[1.0; 4_800]);
        let fast = run(&mut Tremolo::new(40.0, 1.0), &[1.0; 4_800]);
        let troughs = |xs: &[f32]| {
            xs.windows(3)
                .filter(|w| w[1] < w[0] && w[1] <= w[2] && w[1] < 0.05)
                .count()
        };
        assert!(
            troughs(&fast) > troughs(&slow),
            "faster rate → more troughs: {} vs {}",
            troughs(&slow),
            troughs(&fast)
        );
    }

    #[test]
    fn depth_is_clamped() {
        let t = Tremolo::new(5.0, 3.0);
        assert!((t.depth - 1.0).abs() < 1e-6);
    }
}
