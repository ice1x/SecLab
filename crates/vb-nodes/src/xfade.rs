//! Crossfade: a control-swept, equal-power blend between two audio inputs.
//!
//! Where [`crate::math::Mix`] blends by a fixed parameter, `xfade` takes its blend position
//! from a **control input**, so an LFO or envelope can sweep between two sources at audio
//! rate — morph oscillators, duck one layer under another, autopan-style A/B motion. The
//! blend uses an equal-power (constant-loudness) law — `a·cos(t·π/2) + b·sin(t·π/2)` — so
//! the perceived level stays steady across the sweep instead of dipping in the middle the
//! way a linear crossfade does.

use std::f32::consts::FRAC_PI_2;
use vb_core::{ProcessContext, Processor};

/// An equal-power crossfade driven by a control input.
///
/// Input 0 and 1 are audio (`a`, `b`); input 2 is the control blend position, clamped to
/// `[0, 1]` (0 = all `a`, 1 = all `b`). Output 0 is the blended audio.
#[derive(Debug, Clone, Default)]
pub struct Xfade;

impl Processor for Xfade {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let a = inputs.first().copied().unwrap_or(&[]);
        let b = inputs.get(1).copied().unwrap_or(&[]);
        let mix = inputs.get(2).copied().unwrap_or(&[]);
        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let t = mix.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let (gb, ga) = (t * FRAC_PI_2).sin_cos();
            let av = a.get(i).copied().unwrap_or(0.0);
            let bv = b.get(i).copied().unwrap_or(0.0);
            *out = av * ga + bv * gb;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn blend(a: &[f32], b: &[f32], mix: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: a.len(),
        };
        let mut out = vec![0.0_f32; a.len()];
        let ins: [&[f32]; 3] = [a, b, mix];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        Xfade.process(&ctx, &ins, &mut outs);
        out
    }

    #[test]
    fn mix_zero_is_all_a_and_mix_one_is_all_b() {
        let a = [1.0; 4];
        let b = [-1.0; 4];
        let all_a = blend(&a, &b, &[0.0; 4]);
        let all_b = blend(&a, &b, &[1.0; 4]);
        assert!(all_a.iter().all(|&s| (s - 1.0).abs() < 1e-6), "mix 0 → a");
        assert!(all_b.iter().all(|&s| (s + 1.0).abs() < 1e-6), "mix 1 → b");
    }

    #[test]
    fn the_blend_is_equal_power_at_the_midpoint() {
        // At t = 0.5 both gains are cos(π/4) = sin(π/4) ≈ 0.707, so two equal DC inputs
        // sum to ~2·0.707·x, and two uncorrelated equal-power sources keep constant power.
        let a = [1.0; 4];
        let b = [1.0; 4];
        let mid = blend(&a, &b, &[0.5; 4]);
        let expected = 2.0 * (std::f32::consts::FRAC_1_SQRT_2);
        assert!(
            (mid[0] - expected).abs() < 1e-5,
            "equal-power mid: {}",
            mid[0]
        );
        // Gains squared sum to 1 (constant power) at every position.
        for &t in &[0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            let (gb, ga) = (t * FRAC_PI_2).sin_cos();
            assert!(
                (ga * ga + gb * gb - 1.0).abs() < 1e-6,
                "power sums to 1 at {t}"
            );
        }
    }

    #[test]
    fn a_swept_control_moves_the_balance_from_a_to_b() {
        let a = [1.0; 5];
        let b = [-1.0; 5];
        let ramp = [0.0, 0.25, 0.5, 0.75, 1.0];
        let out = blend(&a, &b, &ramp);
        // Monotonically moves from +1 (a) toward -1 (b) as the control rises.
        for w in out.windows(2) {
            assert!(w[1] < w[0] + 1e-6, "balance shifts toward b: {w:?}");
        }
        assert!(out[0] > 0.9 && *out.last().unwrap() < -0.9);
    }

    #[test]
    fn the_control_is_clamped_out_of_range() {
        let a = [1.0; 3];
        let b = [-1.0; 3];
        // Below 0 behaves like 0 (all a), above 1 like 1 (all b).
        assert!((blend(&a, &b, &[-5.0; 3])[0] - 1.0).abs() < 1e-6);
        assert!((blend(&a, &b, &[9.0; 3])[0] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_missing_control_defaults_to_all_a() {
        // No control samples → t = 0 → output is input a untouched.
        let a = [0.3; 4];
        let b = [0.9; 4];
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 4,
        };
        let mut out = vec![0.0_f32; 4];
        let ins: [&[f32]; 2] = [&a, &b];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        Xfade.process(&ctx, &ins, &mut outs);
        assert!(out.iter().all(|&s| (s - 0.3).abs() < 1e-6), "defaults to a");
    }
}
