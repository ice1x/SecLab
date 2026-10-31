//! Stereo width: mid/side control over how wide a stereo signal sits.
//!
//! Takes a stereo pair (left, right) and rebalances it through its mid/side components:
//! `mid = (l + r)/2` is what both channels share, `side = (l - r)/2` is what differs. Scaling
//! only the side by `width` narrows or widens the image without touching the centre —
//! `width = 0` collapses to mono, `1` is unchanged, `> 1` pushes the sides wider. Pair it
//! after [`crate::Pan`] or [`crate::PingPong`] to tighten or exaggerate the stereo field.

use seclab_core::{ProcessContext, Processor};

/// A mid/side stereo-width processor.
///
/// Inputs 0 and 1 are the left/right audio channels; outputs 0 and 1 are the widened
/// left/right. `width` scales the side component: `0` = mono, `1` = unchanged, `2` = double
/// width.
#[derive(Debug, Clone)]
pub struct Width {
    width: f32,
}

impl Width {
    /// Creates a width processor. `width` is clamped to `[0, 2]`.
    #[must_use]
    pub fn new(width: f32) -> Self {
        Self {
            width: width.clamp(0.0, 2.0),
        }
    }
}

impl Processor for Width {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let in_l = inputs.first().copied().unwrap_or(&[]);
        let in_r = inputs.get(1).copied().unwrap_or(&[]);
        let (left, rest) = outputs.split_at_mut(1);
        let left = &mut left[0];
        let right = match rest.first_mut() {
            Some(r) => r,
            None => return,
        };
        for i in 0..ctx.frames {
            let l = in_l.get(i).copied().unwrap_or(0.0);
            let r = in_r.get(i).copied().unwrap_or(0.0);
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5 * self.width;
            if let Some(o) = left.get_mut(i) {
                *o = mid + side;
            }
            if let Some(o) = right.get_mut(i) {
                *o = mid - side;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(w: &mut Width, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: l.len(),
        };
        let mut ol = vec![0.0_f32; l.len()];
        let mut or = vec![0.0_f32; l.len()];
        {
            let ins: [&[f32]; 2] = [l, r];
            let mut outs: [&mut [f32]; 2] = [&mut ol, &mut or];
            w.process(&ctx, &ins, &mut outs);
        }
        (ol, or)
    }

    #[test]
    fn width_one_is_transparent() {
        let mut w = Width::new(1.0);
        let l = [1.0, 0.5, -0.3, 0.8];
        let r = [-0.4, 0.2, 0.9, -0.1];
        let (ol, or) = run(&mut w, &l, &r);
        for i in 0..l.len() {
            assert!((ol[i] - l[i]).abs() < 1e-6, "left unchanged at {i}");
            assert!((or[i] - r[i]).abs() < 1e-6, "right unchanged at {i}");
        }
    }

    #[test]
    fn width_zero_collapses_to_mono() {
        let mut w = Width::new(0.0);
        let l = [1.0, 0.5, -0.3];
        let r = [-0.4, 0.2, 0.9];
        let (ol, or) = run(&mut w, &l, &r);
        for i in 0..l.len() {
            let mid = (l[i] + r[i]) * 0.5;
            assert!(
                (ol[i] - mid).abs() < 1e-6 && (or[i] - mid).abs() < 1e-6,
                "both = mid"
            );
            assert!((ol[i] - or[i]).abs() < 1e-6, "L == R (mono) at {i}");
        }
    }

    #[test]
    fn width_two_doubles_the_side() {
        let mut w = Width::new(2.0);
        let l = [1.0];
        let r = [0.0];
        let (ol, or) = run(&mut w, &l, &r);
        // mid = 0.5, side = 0.5*2 = 1.0 → L = 1.5, R = -0.5.
        assert!((ol[0] - 1.5).abs() < 1e-6, "left widened: {}", ol[0]);
        assert!((or[0] + 0.5).abs() < 1e-6, "right widened: {}", or[0]);
    }

    #[test]
    fn a_centred_signal_is_untouched_by_width() {
        // Equal L/R (side = 0): width has nothing to scale.
        let mut w = Width::new(2.0);
        let l = [0.7, -0.2];
        let r = [0.7, -0.2];
        let (ol, or) = run(&mut w, &l, &r);
        for i in 0..l.len() {
            assert!((ol[i] - l[i]).abs() < 1e-6 && (or[i] - r[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn width_is_clamped() {
        assert_eq!(Width::new(5.0).width, 2.0);
        assert_eq!(Width::new(-1.0).width, 0.0);
    }
}
