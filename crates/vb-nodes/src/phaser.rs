//! Phaser: a sweeping all-pass notch effect.
//!
//! A cascade of first-order all-pass sections shifts the signal's phase, and mixing that
//! phase-shifted copy back with the dry signal carves a series of moving notches into the
//! spectrum. An internal LFO sweeps the all-pass coefficient at `rate_hz`, `depth` sets how
//! far the notches travel, `feedback` sharpens their resonance, and `mix` blends wet against
//! dry. The classic swirling, jet-plane sweep. Self-contained — the LFO lives inside.

use vb_core::{ProcessContext, Processor};

/// Number of cascaded all-pass sections. Four gives two notches — the familiar phaser sound.
const STAGES: usize = 4;

/// A feedback phaser built from a cascade of first-order all-pass filters.
///
/// Input 0 is audio. Output 0 is `dry·(1 - mix) + wet·mix`, where `wet` is the input passed
/// through the swept all-pass cascade with `feedback` around it. `mix = 0` is transparent.
#[derive(Debug, Clone)]
pub struct Phaser {
    rate_hz: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    phase: f32,
    /// One state register per all-pass section (transposed direct-form II).
    stages: [f32; STAGES],
    /// Last cascade output, fed back into the input.
    fb: f32,
}

impl Phaser {
    /// Creates a phaser. `depth` and `mix` are clamped to `0..=1`, `feedback` to `0..=0.95`
    /// so the feedback loop stays stable.
    #[must_use]
    pub fn new(rate_hz: f32, depth: f32, feedback: f32, mix: f32) -> Self {
        Self {
            rate_hz,
            depth: depth.clamp(0.0, 1.0),
            feedback: feedback.clamp(0.0, 0.95),
            mix: mix.clamp(0.0, 1.0),
            phase: 0.0,
            stages: [0.0; STAGES],
            fb: 0.0,
        }
    }
}

impl Processor for Phaser {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let dt = self.rate_hz / sr;
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let dry = input.get(i).copied().unwrap_or(0.0);

            // Sweep the all-pass coefficient with the LFO; |a| < 0.7 keeps every section
            // (and the feedback loop around them) comfortably stable.
            let a = 0.7 * self.depth * (core::f32::consts::TAU * self.phase).sin();

            let mut v = dry + self.feedback * self.fb;
            for s in &mut self.stages {
                // First-order all-pass: y = a·v + s ; s = v − a·y.
                let y = a * v + *s;
                *s = v - a * y;
                v = y;
            }
            self.fb = v;

            *out = dry * (1.0 - self.mix) + v * self.mix;

            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.stages = [0.0; STAGES];
        self.fb = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(p: &mut Phaser, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            p.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn sine(n: usize, hz: f32) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * hz * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn mix_zero_is_transparent() {
        let input = sine(2_048, 440.0);
        let out = run(&mut Phaser::new(0.5, 0.8, 0.5, 0.0), &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "mix 0 passes the dry signal through");
        }
    }

    #[test]
    fn it_colours_the_signal_when_wet() {
        let input = sine(4_096, 440.0);
        let out = run(&mut Phaser::new(0.5, 0.8, 0.4, 0.5), &input);
        let diff: f32 = input.iter().zip(&out).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 1.0,
            "a wet phaser is not transparent, total diff {diff}"
        );
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn high_feedback_stays_bounded() {
        // Full-scale input, maximum feedback: the all-pass loop must not blow up.
        let out = run(&mut Phaser::new(1.0, 1.0, 0.95, 1.0), &[1.0; 48_000]);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            out.iter().all(|s| s.abs() < 50.0),
            "feedback loop stays bounded"
        );
    }

    #[test]
    fn parameters_are_clamped() {
        let p = Phaser::new(0.5, 3.0, 2.0, -1.0);
        assert!((p.depth - 1.0).abs() < 1e-6);
        assert!((p.feedback - 0.95).abs() < 1e-6);
        assert!((p.mix - 0.0).abs() < 1e-6);
    }
}
