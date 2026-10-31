//! Envelope follower: turns an audio signal's loudness into a control signal.
//!
//! It rectifies the input and smooths it with separate attack and release time constants,
//! producing a CV that rides the signal's amplitude envelope. Patch it from an audio source
//! into a modulation destination and the source's own dynamics drive the modulation: the
//! classic auto-wah (envelope → filter cutoff), dynamic tremolo, or a side-chain-style
//! ducking control. Attack faster than release is the usual setting — snap up to transients,
//! ease back down.

use seclab_core::{ProcessContext, Processor};

/// A rectify-and-smooth amplitude follower.
///
/// Input 0 is audio; output 0 is a control signal in `[0, 1)`-ish (it follows `|input|`, so
/// it stays within the input's amplitude range). Attack governs how fast it rises toward a
/// louder input, release how fast it falls toward a quieter one.
#[derive(Debug, Clone)]
pub struct EnvFollower {
    attack_s: f32,
    release_s: f32,
    env: f32,
}

impl EnvFollower {
    /// Creates a follower with attack and release times (seconds).
    #[must_use]
    pub fn new(attack_s: f32, release_s: f32) -> Self {
        Self {
            attack_s,
            release_s,
            env: 0.0,
        }
    }

    /// The current follower value.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.env
    }

    /// One-pole smoothing coefficient for a time constant `t` (seconds) at `sr`.
    /// `coef = 1 - e^(-1/(t·sr))`; a zero (or negative) time means "instant" (coef 1).
    fn coef(t: f32, sr: f32) -> f32 {
        if t > 0.0 {
            1.0 - (-1.0 / (t * sr)).exp()
        } else {
            1.0
        }
    }
}

impl Processor for EnvFollower {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let attack_coef = Self::coef(self.attack_s, sr);
        let release_coef = Self::coef(self.release_s, sr);
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let target = input.get(i).copied().unwrap_or(0.0).abs();
            // Rising toward a louder signal uses the attack rate; falling uses release.
            let coef = if target > self.env {
                attack_coef
            } else {
                release_coef
            };
            self.env += coef * (target - self.env);
            *out = self.env;
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(f: &mut EnvFollower, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            f.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn rises_toward_the_input_amplitude() {
        // A constant 0.8-amplitude tone: the follower climbs toward 0.8.
        let tone: Vec<f32> = (0..4_800)
            .map(|i| 0.8 * (core::f32::consts::TAU * 200.0 * i as f32 / SR).sin())
            .collect();
        let mut f = EnvFollower::new(0.005, 0.05);
        let out = run(&mut f, &tone);
        let tail = out[out.len() - 1];
        assert!(
            tail > 0.3,
            "follows the tone's amplitude, settled at {tail}"
        );
        assert!(out.iter().all(|s| s.is_finite() && *s >= 0.0));
    }

    #[test]
    fn a_louder_input_yields_a_higher_envelope() {
        let quiet: Vec<f32> = (0..4_800)
            .map(|i| 0.2 * (core::f32::consts::TAU * 200.0 * i as f32 / SR).sin())
            .collect();
        let loud: Vec<f32> = (0..4_800)
            .map(|i| 0.9 * (core::f32::consts::TAU * 200.0 * i as f32 / SR).sin())
            .collect();
        let q = *run(&mut EnvFollower::new(0.005, 0.05), &quiet)
            .last()
            .unwrap();
        let l = *run(&mut EnvFollower::new(0.005, 0.05), &loud)
            .last()
            .unwrap();
        assert!(l > q * 2.0, "louder → higher envelope: {q} vs {l}");
    }

    #[test]
    fn decays_toward_silence_when_the_input_stops() {
        // Ramp up on a tone, then feed silence: the envelope falls back down.
        let mut f = EnvFollower::new(0.005, 0.02);
        let tone: Vec<f32> = (0..2_400)
            .map(|i| 0.9 * (core::f32::consts::TAU * 200.0 * i as f32 / SR).sin())
            .collect();
        let peak = *run(&mut f, &tone).last().unwrap();
        let after = run(&mut f, &[0.0; 4_800]);
        assert!(
            after[after.len() - 1] < peak * 0.1,
            "decays after the input stops: {peak} → {}",
            after[after.len() - 1]
        );
    }

    #[test]
    fn attack_is_faster_than_release() {
        // Rising to a step reaches the target sooner than falling from it returns to zero,
        // for attack < release. Compare samples-to-halfway on the way up vs down.
        let mut f = EnvFollower::new(0.001, 0.1);
        let up = run(&mut f, &[1.0; 4_800]);
        let rise_half = up.iter().position(|&v| v >= 0.5).unwrap_or(usize::MAX);
        let down = run(&mut f, &[0.0; 4_800]);
        let fall_half = down.iter().position(|&v| v <= 0.5).unwrap_or(usize::MAX);
        assert!(
            rise_half < fall_half,
            "attack ({rise_half}) faster than release ({fall_half})"
        );
    }

    #[test]
    fn silent_input_stays_at_zero() {
        let out = run(&mut EnvFollower::new(0.01, 0.01), &[0.0; 64]);
        assert!(out.iter().all(|&s| s == 0.0));
    }
}
