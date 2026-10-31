//! AHR (attack–hold–release) one-shot envelope — a gated trapezoid.
//!
//! On each rising gate edge it ramps up over `attack_s`, **holds at full scale for a fixed
//! `hold_s`**, then ramps down over `release_s` and returns to zero — a trapezoid whose
//! length is set by its own times, not by how long the gate is held (the release is
//! ignored, like [`crate::AdEnv`]). Where AD is a triangle that starts falling the instant
//! it peaks, AHR parks at the top for a controllable plateau: useful for gated-reverb
//! swells, sustained blips, or any "on for exactly this long" modulation shape.

use seclab_core::{ProcessContext, Processor};

/// Which segment of the one-shot is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Hold,
    Release,
}

/// A linear attack–hold–release one-shot envelope.
///
/// Input 0 is a gate (control): a rising edge (`< 0.5` → `>= 0.5`) fires the trapezoid.
/// Output 0 is the level in `[0, 1]`. The gate falling has no effect — the contour, hold
/// included, always runs to completion.
#[derive(Debug, Clone)]
pub struct AhrEnv {
    attack_s: f32,
    hold_s: f32,
    release_s: f32,
    stage: Stage,
    level: f32,
    /// Samples remaining in the hold plateau.
    hold_left: u64,
    gate_prev: bool,
}

impl AhrEnv {
    /// Creates an AHR envelope from attack, hold and release times (seconds).
    #[must_use]
    pub fn new(attack_s: f32, hold_s: f32, release_s: f32) -> Self {
        Self {
            attack_s,
            hold_s,
            release_s,
            stage: Stage::Idle,
            level: 0.0,
            hold_left: 0,
            gate_prev: false,
        }
    }

    /// The current envelope level.
    #[must_use]
    pub fn level(&self) -> f32 {
        self.level
    }
}

impl Processor for AhrEnv {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let attack_inc = if self.attack_s > 0.0 {
            1.0 / (self.attack_s * sr)
        } else {
            1.0
        };
        let release_dec = if self.release_s > 0.0 {
            1.0 / (self.release_s * sr)
        } else {
            1.0
        };
        let hold_samples = (self.hold_s.max(0.0) * sr) as u64;

        for (out, gate_in) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            let gate = *gate_in >= 0.5;
            // A rising edge (re)fires the trapezoid from the current level.
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
                        self.stage = Stage::Hold;
                        self.hold_left = hold_samples;
                    }
                }
                Stage::Hold => {
                    self.level = 1.0;
                    if self.hold_left == 0 {
                        self.stage = Stage::Release;
                    } else {
                        self.hold_left -= 1;
                    }
                }
                Stage::Release => {
                    self.level -= release_dec;
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
        self.hold_left = 0;
        self.gate_prev = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(env: &mut AhrEnv, sample_rate: f32, gate: &[f32]) -> Vec<f32> {
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
    fn it_traces_a_trapezoid_rise_plateau_fall() {
        // 1 kHz: attack 10, hold 20, release 10 samples.
        let mut env = AhrEnv::new(0.01, 0.02, 0.01);
        let out = run(&mut env, 1_000.0, &[1.0; 60]);
        // Rises to 1 during attack.
        assert!((out[9] - 1.0).abs() < 1e-3, "full by end of attack");
        // Plateau: a stretch of samples pinned at 1 (the hold).
        let held = out.iter().filter(|&&s| (s - 1.0).abs() < 1e-6).count();
        assert!(held >= 20, "holds at full for ~hold_s: {held} samples");
        // Falls back to 0 by the end.
        assert!(out.last().unwrap().abs() < 1e-3, "released to 0");
    }

    #[test]
    fn the_gate_release_is_ignored_one_shot() {
        // Gate on for only 2 samples: the full attack+hold+release still completes.
        let mut gate = vec![1.0_f32; 2];
        gate.extend(std::iter::repeat_n(0.0, 58));
        let mut env = AhrEnv::new(0.01, 0.02, 0.01);
        let out = run(&mut env, 1_000.0, &gate);
        let held = out.iter().filter(|&&s| (s - 1.0).abs() < 1e-6).count();
        assert!(held >= 20, "hold plateau survives the short gate: {held}");
    }

    #[test]
    fn a_longer_hold_keeps_the_plateau_longer() {
        let plateau = |hold: f32| {
            let mut env = AhrEnv::new(0.005, hold, 0.005);
            let out = run(&mut env, 1_000.0, &[1.0; 200]);
            out.iter().filter(|&&s| (s - 1.0).abs() < 1e-6).count()
        };
        assert!(
            plateau(0.05) > plateau(0.01),
            "more hold time → longer plateau"
        );
    }

    #[test]
    fn a_new_edge_retriggers() {
        let mut env = AhrEnv::new(0.005, 0.005, 0.005);
        let mut gate = vec![1.0_f32; 1];
        gate.extend(std::iter::repeat_n(0.0, 40)); // full contour completes → idle
        gate.push(1.0); // second rising edge
        gate.extend(std::iter::repeat_n(0.0, 30));
        let out = run(&mut env, 1_000.0, &gate);
        let second = out[42..].iter().copied().fold(0.0_f32, f32::max);
        assert!(
            second > 0.9,
            "second trigger fires a fresh trapezoid: {second}"
        );
    }

    #[test]
    fn stays_idle_at_zero_without_a_trigger() {
        let mut env = AhrEnv::new(0.01, 0.01, 0.01);
        let out = run(&mut env, 1_000.0, &[0.0; 16]);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn reset_returns_to_idle() {
        let mut env = AhrEnv::new(0.01, 0.02, 0.01);
        let _ = run(&mut env, 1_000.0, &[1.0; 20]);
        env.reset();
        assert_eq!(env.level, 0.0);
        assert_eq!(env.hold_left, 0);
        assert!(!env.gate_prev);
    }
}
