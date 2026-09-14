//! ADSR envelope node, driven by a gate control input.

use vb_core::{ProcessContext, Processor};

/// Which segment of the envelope is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// A linear ADSR envelope.
///
/// Input 0 is a gate (control): a value `>= 0.5` is "on". A rising edge starts the
/// attack; a falling edge starts the release. Output 0 is the envelope level in
/// `[0, 1]`. Segments are linear, which is cheap and predictable; the release time is
/// referenced to a full-scale fall, so releasing from below full scale is proportionally
/// quicker.
#[derive(Debug, Clone)]
pub struct Adsr {
    attack_s: f32,
    decay_s: f32,
    sustain: f32,
    release_s: f32,
    stage: Stage,
    level: f32,
    gate_prev: bool,
}

impl Adsr {
    /// Creates an envelope from attack/decay/release times (seconds) and sustain level
    /// (`0..=1`, clamped).
    #[must_use]
    pub fn new(attack_s: f32, decay_s: f32, sustain: f32, release_s: f32) -> Self {
        Self {
            attack_s,
            decay_s,
            sustain: sustain.clamp(0.0, 1.0),
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

impl Processor for Adsr {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let attack_inc = if self.attack_s > 0.0 {
            1.0 / (self.attack_s * sr)
        } else {
            1.0
        };
        let decay_dec = if self.decay_s > 0.0 {
            (1.0 - self.sustain) / (self.decay_s * sr)
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
                        self.stage = Stage::Decay;
                    }
                }
                Stage::Decay => {
                    self.level -= decay_dec;
                    if self.level <= self.sustain {
                        self.level = self.sustain;
                        self.stage = Stage::Sustain;
                    }
                }
                Stage::Sustain => self.level = self.sustain,
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

    /// Live-updates `attack_s`, `decay_s`, `sustain`, `release_s` (registry order), preserving the
    /// current stage and level so editing the envelope during a held note reshapes it smoothly
    /// instead of restarting it from zero.
    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&v) = values.first() {
            self.attack_s = v;
        }
        if let Some(&v) = values.get(1) {
            self.decay_s = v;
        }
        if let Some(&v) = values.get(2) {
            self.sustain = v.clamp(0.0, 1.0);
        }
        if let Some(&v) = values.get(3) {
            self.release_s = v;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::Adsr;
    use vb_core::{ProcessContext, Processor};

    /// Runs the envelope over a gate signal and returns the level per sample.
    fn run(env: &mut Adsr, sample_rate: f32, gate: &[f32]) -> Vec<f32> {
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
    fn rises_to_peak_then_settles_to_sustain() {
        // 1 kHz: attack/decay 10 samples each, sustain 0.5.
        let mut env = Adsr::new(0.01, 0.01, 0.5, 0.01);
        let out = run(&mut env, 1_000.0, &vec![1.0; 60]);

        let peak = out.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            (peak - 1.0).abs() < 1e-3,
            "attack should reach 1.0, peak {peak}"
        );
        assert!(
            (out[59] - 0.5).abs() < 1e-3,
            "should hold sustain, got {}",
            out[59]
        );
        assert!(out.iter().all(|&s| (0.0..=1.0).contains(&s)));
    }

    #[test]
    fn releases_to_zero_after_gate_off() {
        let mut env = Adsr::new(0.01, 0.01, 0.5, 0.01);
        // Hold, then release.
        let mut gate = vec![1.0_f32; 40];
        gate.extend(std::iter::repeat_n(0.0, 40));
        let out = run(&mut env, 1_000.0, &gate);
        assert!(
            out[out.len() - 1].abs() < 1e-3,
            "release should reach 0, got {}",
            out[out.len() - 1]
        );
    }

    #[test]
    fn stays_idle_at_zero_without_a_gate() {
        let mut env = Adsr::new(0.01, 0.01, 0.5, 0.01);
        let out = run(&mut env, 1_000.0, &[0.0; 16]);
        assert!(out.iter().all(|&s| s == 0.0));
    }
}
