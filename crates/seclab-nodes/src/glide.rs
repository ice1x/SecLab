//! Glide (portamento): smooths a control signal so pitch slides between notes.
//!
//! A mono synth's "glide" or "portamento" makes a new note *slide* from the previous
//! pitch instead of jumping to it. This node slews its control input toward each new
//! value over a fixed time: wire `midi-in.pitch_hz → glide → pitch-osc.pitch` and a legato
//! run bends smoothly from note to note. With `glide_ms = 0` it passes the input straight
//! through, so it is transparent when portamento is off.
//!
//! The first value it sees snaps in immediately (a patch should sound at pitch on the very
//! first note, not slide up from zero); every change after that ramps over `glide_ms`.

use seclab_core::{ProcessContext, Processor};

use crate::smooth::{ramp_samples, Smoother};

/// Longest glide time offered (ms).
const MAX_GLIDE_MS: f32 = 5_000.0;

/// A control-rate portamento: ramps its input toward each new target over `glide_ms`.
#[derive(Debug, Clone)]
pub struct Glide {
    smoother: Smoother,
    /// Glide time in milliseconds.
    glide_ms: f32,
    /// The last target we aimed at, so an unchanged input does not restart the ramp.
    last_target: f32,
    /// Whether we have seen any input yet (the first value snaps in).
    primed: bool,
}

impl Glide {
    /// Creates a glide of `glide_ms` milliseconds (0 = pass-through).
    #[must_use]
    pub fn new(glide_ms: f32) -> Self {
        Self {
            smoother: Smoother::new(0.0),
            glide_ms: glide_ms.clamp(0.0, MAX_GLIDE_MS),
            last_target: 0.0,
            primed: false,
        }
    }

    /// The value the glide currently sits at.
    #[must_use]
    pub fn current(&self) -> f32 {
        self.smoother.current()
    }
}

impl Processor for Glide {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(out) = outputs.first_mut() else {
            return;
        };
        // Block-rate target: control signals are constant across a block.
        let target = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);

        if !self.primed {
            // First note sounds at pitch immediately — no slide up from zero.
            self.smoother.reset(target);
            self.last_target = target;
            self.primed = true;
        } else if target != self.last_target {
            self.smoother
                .set_target(target, ramp_samples(self.glide_ms, ctx.sample_rate));
            self.last_target = target;
        }

        for slot in out.iter_mut().take(ctx.frames) {
            *slot = self.smoother.next_sample();
        }
    }

    fn reset(&mut self) {
        self.smoother.reset(0.0);
        self.last_target = 0.0;
        self.primed = false;
    }

    /// Live-updates `glide_ms` (registry order), preserving the smoother's current value and
    /// target so the pitch keeps gliding from where it is instead of snapping.
    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&v) = values.first() {
            self.glide_ms = v.clamp(0.0, MAX_GLIDE_MS);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(g: &mut Glide, target: f32, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        {
            let mut slice = [&mut out[..]];
            g.process(&ctx, &[&vec![target; frames]], &mut slice);
        }
        out
    }

    #[test]
    fn the_first_value_snaps_in_without_gliding() {
        let mut g = Glide::new(100.0);
        let out = run(&mut g, 440.0, 16);
        assert!(
            out.iter().all(|&s| (s - 440.0).abs() < 1e-3),
            "first note is instant"
        );
    }

    #[test]
    fn a_new_target_glides_from_the_previous_value() {
        let mut g = Glide::new(50.0); // 50 ms = 2400 samples at 48 kHz
        run(&mut g, 220.0, 16); // settle at 220
        let out = run(&mut g, 440.0, 512); // glide toward 440
                                           // Mid-glide the value sits strictly between the old and new pitch.
        assert!(
            out[0] > 220.0 && out[0] < 440.0,
            "starts gliding: {}",
            out[0]
        );
        assert!(out[511] > out[0], "climbs toward the target");
        assert!(out.iter().all(|&s| (220.0..=440.0).contains(&s)));
    }

    #[test]
    fn the_glide_reaches_the_target() {
        let mut g = Glide::new(10.0); // 480 samples
        run(&mut g, 100.0, 4);
        let out = run(&mut g, 800.0, 2_048); // well past the ramp length
        assert!(
            (out[out.len() - 1] - 800.0).abs() < 1e-2,
            "settles on target"
        );
    }

    #[test]
    fn zero_glide_time_passes_through() {
        let mut g = Glide::new(0.0);
        run(&mut g, 100.0, 4);
        let out = run(&mut g, 900.0, 8);
        assert!(
            out.iter().all(|&s| (s - 900.0).abs() < 1e-3),
            "instant when off"
        );
    }

    #[test]
    fn an_unchanged_target_holds_steady() {
        let mut g = Glide::new(50.0);
        run(&mut g, 330.0, 16);
        let out = run(&mut g, 330.0, 16);
        assert!(
            out.iter().all(|&s| (s - 330.0).abs() < 1e-3),
            "no drift when held"
        );
    }

    #[test]
    fn reset_re_primes_the_next_value() {
        let mut g = Glide::new(50.0);
        run(&mut g, 200.0, 8);
        g.reset();
        let out = run(&mut g, 660.0, 8); // first value after reset snaps again
        assert!(out.iter().all(|&s| (s - 660.0).abs() < 1e-3));
    }
}
