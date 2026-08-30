//! Pan: place a mono signal in the stereo field with an equal-power law.
//!
//! Splits one audio input into a **left** and a **right** output, weighted so the signal
//! sits anywhere from hard-left (`-1`) through centre (`0`) to hard-right (`+1`). The
//! position is the `pan` parameter plus a control input, so an LFO or envelope can move the
//! source across the field (auto-pan, stereo widening on a doubled voice). The two gains
//! follow the constant-power law `cos`/`sin`, so the *total* level stays steady as the
//! signal pans rather than dipping through the middle. This is the first node with a real
//! stereo (two-channel) output — end a patch here to feed the host a stereo signal.

use std::f32::consts::FRAC_PI_2;
use vb_core::{ProcessContext, Processor};

/// An equal-power stereo panner.
///
/// Input 0 is audio, input 1 is the control pan offset. Output 0 is the **left** channel,
/// output 1 the **right**. The position is `pan + cv`, clamped to `[-1, 1]`: `-1` is hard
/// left, `0` centre (both gains `≈0.707`), `+1` hard right.
#[derive(Debug, Clone)]
pub struct Pan {
    /// Base pan position, clamped to `[-1, 1]`.
    pan: f32,
}

impl Pan {
    /// Creates a panner at base position `pan` (clamped to `[-1, 1]`).
    #[must_use]
    pub fn new(pan: f32) -> Self {
        Self {
            pan: pan.clamp(-1.0, 1.0),
        }
    }
}

impl Processor for Pan {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        let cv = inputs.get(1).copied().unwrap_or(&[]);
        // Split the output borrow into the two channels (left, right).
        let (left, rest) = outputs.split_at_mut(1);
        let left = &mut left[0];
        let right = match rest.first_mut() {
            Some(r) => r,
            None => return,
        };
        for i in 0..ctx.frames {
            let x = input.get(i).copied().unwrap_or(0.0);
            let pos = (self.pan + cv.get(i).copied().unwrap_or(0.0)).clamp(-1.0, 1.0);
            // Map [-1, 1] → [0, 1] and take the constant-power gains.
            let t = (pos + 1.0) * 0.5;
            let (rg, lg) = (t * FRAC_PI_2).sin_cos();
            if let Some(o) = left.get_mut(i) {
                *o = x * lg;
            }
            if let Some(o) = right.get_mut(i) {
                *o = x * rg;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn pan_frame(p: &mut Pan, x: f32, cv: Option<f32>) -> (f32, f32) {
        let input = [x; 4];
        let cvbuf = cv.map(|c| [c; 4]);
        let mut l = [0.0_f32; 4];
        let mut r = [0.0_f32; 4];
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 4,
        };
        {
            let ins: Vec<&[f32]> = match &cvbuf {
                Some(c) => vec![&input, c],
                None => vec![&input],
            };
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            p.process(&ctx, &ins, &mut outs);
        }
        (l[0], r[0])
    }

    #[test]
    fn centre_is_equal_and_constant_power() {
        let mut p = Pan::new(0.0);
        let (l, r) = pan_frame(&mut p, 1.0, None);
        assert!((l - r).abs() < 1e-6, "centre is equal: {l} vs {r}");
        let expected = std::f32::consts::FRAC_1_SQRT_2;
        assert!((l - expected).abs() < 1e-5, "≈0.707 each: {l}");
        assert!((l * l + r * r - 1.0).abs() < 1e-6, "power sums to 1");
    }

    #[test]
    fn hard_left_and_hard_right() {
        let mut left = Pan::new(-1.0);
        let (l, r) = pan_frame(&mut left, 1.0, None);
        assert!(
            (l - 1.0).abs() < 1e-5 && r.abs() < 1e-6,
            "hard left: {l}/{r}"
        );

        let mut right = Pan::new(1.0);
        let (l, r) = pan_frame(&mut right, 1.0, None);
        assert!(
            l.abs() < 1e-6 && (r - 1.0).abs() < 1e-5,
            "hard right: {l}/{r}"
        );
    }

    #[test]
    fn power_is_constant_across_the_whole_sweep() {
        for pos in [-1.0, -0.5, -0.1, 0.0, 0.25, 0.6, 1.0] {
            let mut p = Pan::new(pos);
            let (l, r) = pan_frame(&mut p, 1.0, None);
            assert!(
                (l * l + r * r - 1.0).abs() < 1e-5,
                "constant power at {pos}: {}",
                l * l + r * r
            );
        }
    }

    #[test]
    fn the_control_input_moves_the_position() {
        // Base centre, but a +1 control pushes hard right.
        let mut p = Pan::new(0.0);
        let (l, r) = pan_frame(&mut p, 1.0, Some(1.0));
        assert!(
            l.abs() < 1e-6 && (r - 1.0).abs() < 1e-5,
            "cv pans right: {l}/{r}"
        );
        // A -1 control pushes hard left.
        let (l, r) = pan_frame(&mut p, 1.0, Some(-1.0));
        assert!(
            (l - 1.0).abs() < 1e-5 && r.abs() < 1e-6,
            "cv pans left: {l}/{r}"
        );
    }

    #[test]
    fn position_is_clamped() {
        assert_eq!(Pan::new(5.0).pan, 1.0);
        assert_eq!(Pan::new(-5.0).pan, -1.0);
        // An out-of-range control is clamped too: base right + more right stays hard right.
        let mut p = Pan::new(1.0);
        let (l, r) = pan_frame(&mut p, 1.0, Some(5.0));
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-5);
    }
}
