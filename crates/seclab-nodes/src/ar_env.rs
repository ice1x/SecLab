//! AR (attack–release) gate envelope — the simplest sustaining contour.
//!
//! While the gate is held, the level ramps up over `attack_s` and then **sustains at full
//! scale** for as long as the gate stays on; when the gate falls it ramps back down over
//! `release_s`. Unlike the one-shot [`crate::AdEnv`] (which ignores the gate release), an AR
//! envelope follows the gate for its whole length — the classic organ / pad / string shape
//! (also known as ASR). It has no decay or sustain-level control: the plateau is always
//! unity, which is what makes it the go-to for simply "on while held, off when released".

use seclab_core::{ProcessContext, Processor};

/// Which segment of the contour is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Sustain,
    Release,
}

/// A linear attack–sustain–release gate envelope.
///
/// Input 0 is a gate (control): the level rises while it is high (`>= 0.5`) and falls once
/// it goes low. Output 0 is the level in `[0, 1]`.
#[derive(Debug, Clone)]
pub struct ArEnv {
    attack_s: f32,
    release_s: f32,
    stage: Stage,
    level: f32,
    gate_prev: bool,
}

impl ArEnv {
    /// Creates an AR envelope from attack and release times (seconds).
    #[must_use]
    pub fn new(attack_s: f32, release_s: f32) -> Self {
        Self {
            attack_s,
            release_s,
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

impl Processor for ArEnv {
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

        for (out, gate_in) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            let gate = *gate_in >= 0.5;
            // A rising edge starts (or restarts) the attack; a falling edge starts release.
            if gate && !self.gate_prev {
                self.stage = Stage::Attack;
            } else if !gate && self.gate_prev {
                self.stage = Stage::Release;
            }
            self.gate_prev = gate;

            match self.stage {
                Stage::Idle => self.level = 0.0,
                Stage::Attack => {
                    self.level += attack_inc;
                    if self.level >= 1.0 {
                        self.level = 1.0;
                        self.stage = Stage::Sustain;
                    }
                }
                Stage::Sustain => self.level = 1.0,
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
        self.gate_prev = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(env: &mut ArEnv, sample_rate: f32, gate: &[f32]) -> Vec<f32> {
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
    fn it_rises_to_full_and_sustains_while_the_gate_is_held() {
        // 1 kHz, attack 10 samples. Gate held for 40 samples.
        let mut env = ArEnv::new(0.01, 0.01);
        let out = run(&mut env, 1_000.0, &[1.0; 40]);
        assert!((out[9] - 1.0).abs() < 1e-3, "reaches full by end of attack");
        // Plateau: the tail stays pinned at unity while the gate is still on.
        for &s in &out[15..] {
            assert!((s - 1.0).abs() < 1e-6, "sustains at 1 while held: {s}");
        }
    }

    #[test]
    fn it_releases_to_zero_after_the_gate_falls() {
        // Gate on 20 samples, then off: the level must ramp back down to 0.
        let mut gate = vec![1.0_f32; 20];
        gate.extend(std::iter::repeat_n(0.0, 40));
        let mut env = ArEnv::new(0.005, 0.01);
        let out = run(&mut env, 1_000.0, &gate);
        assert!((out[19] - 1.0).abs() < 1e-3, "held at full at gate end");
        assert!(
            out.last().unwrap().abs() < 1e-3,
            "released to 0: {}",
            out.last().unwrap()
        );
    }

    #[test]
    fn the_attack_ramps_monotonically() {
        let mut env = ArEnv::new(0.02, 0.01); // 20-sample attack at 1 kHz
        let out = run(&mut env, 1_000.0, &[1.0; 20]);
        for w in out[..18].windows(2) {
            assert!(w[1] >= w[0], "attack is non-decreasing: {w:?}");
        }
        assert!(out[0] > 0.0 && out[0] < 1.0, "starts partway up");
    }

    #[test]
    fn a_new_gate_after_release_retriggers() {
        let mut env = ArEnv::new(0.005, 0.005);
        let mut gate = vec![1.0_f32; 10];
        gate.extend(std::iter::repeat_n(0.0, 20)); // release to 0
        gate.extend(std::iter::repeat_n(1.0, 10)); // second gate
        let out = run(&mut env, 1_000.0, &gate);
        let second = out[30..].iter().copied().fold(0.0_f32, f32::max);
        assert!(second > 0.9, "second gate rises again: {second}");
    }

    #[test]
    fn stays_idle_at_zero_without_a_gate() {
        let mut env = ArEnv::new(0.01, 0.01);
        let out = run(&mut env, 1_000.0, &[0.0; 16]);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn reset_returns_to_idle() {
        let mut env = ArEnv::new(0.01, 0.01);
        let _ = run(&mut env, 1_000.0, &[1.0; 20]);
        env.reset();
        assert_eq!(env.level, 0.0);
        assert!(!env.gate_prev);
    }
}
