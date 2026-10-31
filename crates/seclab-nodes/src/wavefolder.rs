//! Wavefolder: a west-coast timbre generator that reflects the signal off the rails.
//!
//! A wavefolder drives the signal hard and folds whatever exceeds `±1` back into range,
//! again and again — each fold adds a burst of harmonics, so as `drive` rises a plain sine
//! blooms into a bright, metallic spectrum. Unlike the [`crate::Waveshaper`]'s single
//! `Foldback` reflection, this node adds a `symmetry` offset **before** folding: shifting
//! the fold points off-centre breaks the waveform's odd symmetry and introduces even
//! harmonics, the hollow/reedy character that defines Buchla-style folding. `level` trims
//! the (always bounded) output.

use seclab_core::{ProcessContext, Processor};

/// A multi-fold wavefolder with an adjustable fold symmetry.
///
/// Input 0 is audio. Output 0 is the folded signal, always within `±level`. `drive` is the
/// pre-gain into the folder (more drive → more folds → richer spectrum), `symmetry` biases
/// the signal before folding (0 = odd-symmetric, `±` introduces even harmonics), `level` is
/// the post-fold gain.
#[derive(Debug, Clone)]
pub struct Wavefolder {
    drive: f32,
    symmetry: f32,
    level: f32,
}

impl Wavefolder {
    /// Creates a wavefolder. `drive` is clamped to `>= 0`.
    #[must_use]
    pub fn new(drive: f32, symmetry: f32, level: f32) -> Self {
        Self {
            drive: drive.max(0.0),
            symmetry,
            level,
        }
    }

    /// Reflects `v` into `[-1, 1]` via a period-4 triangle through the origin.
    fn fold(v: f32) -> f32 {
        let t = (v - 1.0).rem_euclid(4.0);
        (t - 2.0).abs() - 1.0
    }
}

impl Processor for Wavefolder {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (Some(input), Some(out)) = (inputs.first(), outputs.first_mut()) else {
            return;
        };
        for (o, &x) in out.iter_mut().zip(input.iter()).take(ctx.frames) {
            *o = Self::fold(self.drive * x + self.symmetry) * self.level;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const SR: f32 = 48_000.0;

    fn fold_one(f: &mut Wavefolder, x: f32) -> f32 {
        let input = [x; 8];
        let mut out = [0.0_f32; 8];
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 8,
        };
        f.process(&ctx, &[&input], &mut [&mut out]);
        out[0]
    }

    fn render(f: &mut Wavefolder, xs: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: xs.len(),
        };
        let mut out = vec![0.0_f32; xs.len()];
        f.process(&ctx, &[xs], &mut [&mut out]);
        out
    }

    fn zero_crossings(xs: &[f32]) -> usize {
        xs.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    }

    #[test]
    fn it_is_the_identity_inside_the_rails_at_unity_drive() {
        let mut f = Wavefolder::new(1.0, 0.0, 1.0);
        for x in [-0.9, -0.3, 0.0, 0.25, 0.8] {
            assert!((fold_one(&mut f, x) - x).abs() < 1e-6, "identity at {x}");
        }
    }

    #[test]
    fn the_output_is_always_bounded() {
        let mut f = Wavefolder::new(8.0, 0.4, 0.9);
        for x in [-100.0, -7.3, -1.0, 0.0, 1.0, 13.9, 512.0] {
            let y = fold_one(&mut f, x);
            assert!(y.abs() <= 0.9 + 1e-6, "fold({x}) = {y} within ±level");
        }
    }

    #[test]
    fn more_drive_adds_more_folds() {
        // A sine folded harder crosses zero many more times (more harmonics).
        let sine: Vec<f32> = (0..2_048)
            .map(|i| (TAU * 50.0 * i as f32 / SR).sin())
            .collect();
        let gentle = zero_crossings(&render(&mut Wavefolder::new(1.0, 0.0, 1.0), &sine));
        let hard = zero_crossings(&render(&mut Wavefolder::new(8.0, 0.0, 1.0), &sine));
        assert!(
            hard > gentle * 2,
            "harder drive folds more: {gentle} vs {hard} crossings"
        );
    }

    #[test]
    fn symmetry_zero_is_odd_symmetric_but_a_bias_breaks_it() {
        // With no bias the fold is odd: fold(-x) == -fold(x) (only odd harmonics).
        let mut sym = Wavefolder::new(3.0, 0.0, 1.0);
        let pos = fold_one(&mut sym, 0.4);
        let neg = fold_one(&mut sym, -0.4);
        assert!((pos + neg).abs() < 1e-6, "odd-symmetric at symmetry 0");

        // A symmetry offset breaks that symmetry → even harmonics appear.
        let mut biased = Wavefolder::new(3.0, 0.5, 1.0);
        let bp = fold_one(&mut biased, 0.4);
        let bn = fold_one(&mut biased, -0.4);
        assert!(
            (bp + bn).abs() > 0.1,
            "bias breaks odd symmetry: {bp} vs {bn}"
        );
    }

    #[test]
    fn level_scales_the_result() {
        let mut full = Wavefolder::new(5.0, 0.2, 1.0);
        let mut half = Wavefolder::new(5.0, 0.2, 0.5);
        let a = fold_one(&mut full, 0.7);
        let b = fold_one(&mut half, 0.7);
        assert!((b - a * 0.5).abs() < 1e-6, "level halves the output");
    }

    #[test]
    fn drive_is_clamped_non_negative() {
        assert_eq!(Wavefolder::new(-2.0, 0.0, 1.0).drive, 0.0);
    }
}
