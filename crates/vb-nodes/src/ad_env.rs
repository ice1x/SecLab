//! AD (attack–decay) one-shot envelope, driven by a gate/trigger control input.
//!
//! Where the [`crate::Adsr`] holds a sustain for as long as the gate is on, an AD envelope
//! fires a single attack→decay contour on each rising gate edge and then returns to zero,
//! **ignoring the gate release entirely**. That "trigger and forget" shape is what
//! percussion (kick/snare/tom bodies, plucks, blips) is built from: a note just fires the
//! contour, its length set by the attack and decay times rather than how long a key is held.
//! A new rising edge re-triggers from the current level.

use vb_core::{ProcessContext, Processor};

/// Which segment of the one-shot is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
}

/// A linear attack–decay one-shot envelope.
///
/// Input 0 is a gate (control): a rising edge (`< 0.5` → `>= 0.5`) fires the contour. Output
/// 0 is the level in `[0, 1]`. The gate falling has no effect — the contour always runs to
/// completion.
#[derive(Debug, Clone)]
pub struct AdEnv {
    attack_s: f32,
    decay_s: f32,
    stage: Stage,
    level: f32,
    gate_prev: bool,
}

impl AdEnv {
    /// Creates an AD envelope from attack and decay times (seconds).
    #[must_use]
    pub fn new(attack_s: f32, decay_s: f32) -> Self {
        Self {
            attack_s,
            decay_s,
            stage: Stage::Idle,
            level: 0.0,
            gate_prev: false,
        }
    }

    /// The current envelope level.
    #[must_use]
    pub fn level(&self) -> f32 {
        self.level
    }
}

impl Processor for AdEnv {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let attack_inc = if self.attack_s > 0.0 {
            1.0 / (self.attack_s * sr)
        } else {
            1.0
        };
        let decay_dec = if self.decay_s > 0.0 {
            1.0 / (self.decay_s * sr)
        } else {
            1.0
        };

        for (out, gate_in) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            let gate = *gate_in >= 0.5;
            // A rising edge (re)fires the contour from the current level.
            if gate && !self.gate_prev {
                self.stage = Stage::Attack;
            }
            self.gate_prev = gate;

            match self.stage {
                Stage::Idle => self.level = 0.0,
                Stage::Attack => {
                    self.level += attack_inc;
                    if self.level >= 1.0 {
                        self.level = 1.0;
                        self.stage = Stage::Decay;
                    }
                }
                Stage::Decay => {
                    self.level -= decay_dec;
                    if self.level <= 0.0 {
                        self.level = 0.0;
                        self.stage = Stage::Idle;
                    }
                }
            }
            *out = self.level;
        }
    }

    fn reset(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
        self.gate_prev = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(env: &mut AdEnv, sample_rate: f32, gate: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate,
            frames: gate.len(),
        };
        let mut out = vec![0.0_f32; gate.len()];
        let ins: [&[f32]; 1] = [gate];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        env.process(&ctx, &ins, &mut outs);
        out
    }

    #[test]
    fn a_trigger_rises_then_falls_back_to_zero() {
        // 1 kHz, attack/decay 10 samples each. One rising edge, gate stays high.
        let mut env = AdEnv::new(0.01, 0.01);
        let out = run(&mut env, 1_000.0, &[1.0; 40]);
        let (peak_i, peak) =
            out.iter()
                .copied()
                .enumerate()
                .fold(
                    (0, 0.0_f32),
                    |(bi, bv), (i, v)| if v > bv { (i, v) } else { (bi, bv) },
                );
        assert!((peak - 1.0).abs() < 1e-3, "reaches full scale, peak {peak}");
        assert!(
            peak_i > 0 && peak_i < 25,
            "peak is early (after attack), at {peak_i}"
        );
        assert!(
            out[out.len() - 1].abs() < 1e-3,
            "returns to 0 after decay, got {}",
            out[out.len() - 1]
        );
        assert!(out.iter().all(|&s| (0.0..=1.0).contains(&s)));
    }

    #[test]
    fn the_gate_release_is_ignored_one_shot() {
        // Gate on for only 2 samples, but the full ~20-sample contour still completes: the
        // level keeps rising past the release and peaks well after the gate has fallen.
        let mut env = AdEnv::new(0.01, 0.01);
        let mut gate = vec![1.0_f32; 2];
        gate.extend(std::iter::repeat_n(0.0, 38));
        let out = run(&mut env, 1_000.0, &gate);
        let peak = out.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            (peak - 1.0).abs() < 1e-3,
            "one-shot ignores the short gate and still peaks, got {peak}"
        );
    }

    #[test]
    fn a_new_edge_retriggers() {
        let mut env = AdEnv::new(0.01, 0.01);
        // First trigger, let it fully decay, then a gap, then a second trigger.
        let mut gate = vec![1.0_f32; 1];
        gate.extend(std::iter::repeat_n(0.0, 40)); // decay to 0
        gate.push(1.0); // second rising edge
        gate.extend(std::iter::repeat_n(0.0, 20));
        let out = run(&mut env, 1_000.0, &gate);
        // There should be two separate humps: a peak in the first 25 samples AND after the retrigger.
        let second_hump = out[42..].iter().copied().fold(0.0_f32, f32::max);
        assert!(
            second_hump > 0.5,
            "the second trigger fires a fresh contour, got {second_hump}"
        );
    }

    #[test]
    fn stays_idle_at_zero_without_a_trigger() {
        let mut env = AdEnv::new(0.01, 0.01);
        let out = run(&mut env, 1_000.0, &[0.0; 16]);
        assert!(out.iter().all(|&s| s == 0.0));
    }
}
