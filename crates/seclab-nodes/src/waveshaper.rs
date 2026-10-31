//! Waveshaper: drive the signal into a nonlinearity — from warm saturation to
//! aggressive foldback.
//!
//! Stateless per sample, so it is trivially RT-safe. `drive` is the pre-gain into
//! the curve, `level` the post-gain out of it.

use seclab_core::{ProcessContext, Processor};

/// The transfer curve applied after the drive stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Smooth `tanh` saturation — musical, compresses peaks.
    Saturate,
    /// Hard clip at ±1 — buzzy, aggressive.
    HardClip,
    /// Triangle foldback — the signal reflects off the rails instead of flattening.
    Foldback,
}

/// Applies `shape` to `x` (already driven).
fn transfer(shape: Shape, x: f32) -> f32 {
    match shape {
        Shape::Saturate => x.tanh(),
        Shape::HardClip => x.clamp(-1.0, 1.0),
        Shape::Foldback => {
            // Reflect x into [-1, 1]: period-4 triangle through the origin.
            let t = (x - 1.0).rem_euclid(4.0);
            (t - 2.0).abs() - 1.0
        }
    }
}

/// Drive → nonlinearity → level.
pub struct Waveshaper {
    shape: Shape,
    drive: f32,
    level: f32,
}

impl Waveshaper {
    /// Creates a shaper with the given curve, pre-gain and post-gain.
    #[must_use]
    pub fn new(shape: Shape, drive: f32, level: f32) -> Self {
        Self {
            shape,
            drive: drive.max(0.0),
            level,
        }
    }
}

impl Processor for Waveshaper {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (Some(input), Some(out)) = (inputs.first(), outputs.first_mut()) else {
            return;
        };
        for (o, x) in out.iter_mut().zip(input.iter()).take(ctx.frames) {
            *o = transfer(self.shape, self.drive * x) * self.level;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: ProcessContext = ProcessContext {
        sample_rate: 48_000.0,
        frames: 8,
    };

    fn shape_one(shaper: &mut Waveshaper, x: f32) -> f32 {
        let input = [x; 8];
        let mut out = [0.0_f32; 8];
        shaper.process(&CTX, &[&input], &mut [&mut out]);
        out[0]
    }

    #[test]
    fn saturation_is_linear_when_quiet_and_compresses_when_loud() {
        let mut s = Waveshaper::new(Shape::Saturate, 1.0, 1.0);
        let quiet = shape_one(&mut s, 0.1);
        assert!((quiet - 0.1).abs() < 0.001, "≈linear at low level: {quiet}");

        let loud = shape_one(&mut s, 5.0);
        assert!(loud < 1.0 && loud > 0.99, "compressed toward 1: {loud}");

        // Odd symmetry: no DC offset introduced.
        assert!((shape_one(&mut s, -0.6) + shape_one(&mut s, 0.6)).abs() < 1e-6);
    }

    #[test]
    fn hard_clip_flattens_the_rails() {
        let mut s = Waveshaper::new(Shape::HardClip, 4.0, 1.0);
        assert_eq!(shape_one(&mut s, 0.5), 1.0, "0.5 * drive 4 clips");
        assert_eq!(shape_one(&mut s, -2.0), -1.0);
        let inside = shape_one(&mut s, 0.1);
        assert!(
            (inside - 0.4).abs() < 1e-6,
            "inside the rails drive is linear"
        );
    }

    #[test]
    fn foldback_reflects_instead_of_flattening() {
        let mut s = Waveshaper::new(Shape::Foldback, 1.0, 1.0);
        // Identity inside [-1, 1].
        assert!((shape_one(&mut s, 0.5) - 0.5).abs() < 1e-6);
        // 1.5 folds back down to 0.5; 2.5 folds through to -0.5.
        assert!((shape_one(&mut s, 1.5) - 0.5).abs() < 1e-6);
        assert!((shape_one(&mut s, 2.5) + 0.5).abs() < 1e-6);
        // Bounded for wild inputs.
        for x in [-100.0, -7.3, 13.9, 512.0] {
            let y = shape_one(&mut s, x);
            assert!((-1.0..=1.0).contains(&y), "fold({x}) = {y}");
        }
    }

    #[test]
    fn drive_adds_harmonics_and_level_scales_the_result() {
        // A driven sine through tanh gets closer to a square: its RMS rises.
        let frames = 1_024;
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
        };
        let input: Vec<f32> = (0..frames)
            .map(|i| (std::f32::consts::TAU * 100.0 * i as f32 / 48_000.0).sin())
            .collect();
        let rms = |xs: &[f32]| (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt();

        let mut clean = Waveshaper::new(Shape::Saturate, 1.0, 1.0);
        let mut driven = Waveshaper::new(Shape::Saturate, 10.0, 1.0);
        let mut out_clean = vec![0.0; frames];
        let mut out_driven = vec![0.0; frames];
        clean.process(&ctx, &[&input], &mut [&mut out_clean]);
        driven.process(&ctx, &[&input], &mut [&mut out_driven]);
        assert!(rms(&out_driven) > rms(&out_clean) * 1.2);

        let mut half = Waveshaper::new(Shape::Saturate, 10.0, 0.5);
        let mut out_half = vec![0.0; frames];
        half.process(&ctx, &[&input], &mut [&mut out_half]);
        for (h, d) in out_half.iter().zip(&out_driven) {
            assert!((h - d * 0.5).abs() < 1e-6);
        }
    }
}
