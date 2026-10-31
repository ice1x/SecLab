//! Ring modulator: multiply the input by an internal sine carrier.
//!
//! Amplitude-modulating one signal by another sine produces sum-and-difference sidebands
//! (`f_in ± f_carrier`) with the original frequencies suppressed — the inharmonic, metallic,
//! bell-and-robot-voice sound of the classic ring modulator. Unlike the bare [`crate::Mul`]
//! node (which multiplies two patched signals), this is a self-contained effect: it carries
//! its own carrier oscillator at `freq_hz` and a dry/wet `mix`.

use seclab_core::{ProcessContext, Processor};

/// A ring modulator with an internal sine carrier.
///
/// Input 0 is audio. Output 0 is `in·(1-mix) + (in·carrier)·mix`, where `carrier` is a sine
/// at `freq_hz`. `mix = 0` passes the input through; `mix = 1` is full ring modulation.
#[derive(Debug, Clone)]
pub struct RingMod {
    freq_hz: f32,
    mix: f32,
    phase: f32,
}

impl RingMod {
    /// Creates a ring modulator at `freq_hz` with dry/wet `mix` (clamped to `0..=1`).
    #[must_use]
    pub fn new(freq_hz: f32, mix: f32) -> Self {
        Self {
            freq_hz,
            mix: mix.clamp(0.0, 1.0),
            phase: 0.0,
        }
    }
}

impl Processor for RingMod {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let dt = self.freq_hz / sr;
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let carrier = (core::f32::consts::TAU * self.phase).sin();
            let wet = x * carrier;
            *out = x * (1.0 - self.mix) + wet * self.mix;
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

    fn run(rm: &mut RingMod, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            rm.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(freq: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * freq * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn mix_zero_passes_the_input_through() {
        let input = tone(220.0, 512);
        let out = run(&mut RingMod::new(200.0, 0.0), &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "dry passes through: {a} vs {b}");
        }
    }

    #[test]
    fn full_wet_multiplies_by_the_carrier_and_stays_bounded() {
        // With a unit-amplitude input and a sine carrier, the product stays within ±1.
        let input = tone(220.0, 4_800);
        let out = run(&mut RingMod::new(500.0, 1.0), &input);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() <= 1.0 + 1e-6),
            "bounded"
        );
        // The result differs materially from the input (sidebands, carrier nulls).
        let diff = input
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff > 0.3,
            "ring modulation reshapes the signal (max diff {diff})"
        );
    }

    #[test]
    fn the_carrier_frequency_changes_the_result() {
        let input = tone(220.0, 4_800);
        let a = run(&mut RingMod::new(300.0, 1.0), &input);
        let b = run(&mut RingMod::new(900.0, 1.0), &input);
        let diff = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff > 0.2,
            "different carriers → different output (max diff {diff})"
        );
    }

    #[test]
    fn mix_is_clamped_and_zero_sample_rate_is_safe() {
        let rm = RingMod::new(200.0, 5.0); // mix clamped to 1
        assert!((rm.mix - 1.0).abs() < 1e-6);
        let mut rm = RingMod::new(200.0, 1.0);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 8,
        };
        let mut out = [0.0_f32; 8];
        {
            let mut outs: [&mut [f32]; 1] = [&mut out];
            rm.process(&ctx, &[&[0.5; 8]], &mut outs);
        }
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
