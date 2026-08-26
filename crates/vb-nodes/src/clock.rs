//! Clock: an internal gate metronome.
//!
//! Emits a periodic gate — `1` for the first `pulse_width` fraction of each beat, `0` for
//! the rest — at a musical tempo of `bpm` quarter-notes per minute. It is the trigger source
//! for the rest of the modulation toolkit: drive an envelope's gate, clock a sample-and-hold,
//! or feed a clock-divider to build rhythms. Self-contained — no input, the beat lives inside.

use vb_core::{ProcessContext, Processor};

/// A control-rate gate clock.
///
/// No inputs. Output 0 is a gate (control): `1.0` during the pulse, `0.0` otherwise, cycling
/// at `bpm` quarter-notes per minute. `pulse_width` sets the duty cycle (fraction high).
#[derive(Debug, Clone)]
pub struct Clock {
    bpm: f32,
    pulse_width: f32,
    phase: f32,
}

impl Clock {
    /// Creates a clock. `bpm` is clamped to a small positive floor, `pulse_width` to
    /// `0.01..=0.99` so each beat has both an on and an off portion.
    #[must_use]
    pub fn new(bpm: f32, pulse_width: f32) -> Self {
        Self {
            bpm: bpm.max(1e-3),
            pulse_width: pulse_width.clamp(0.01, 0.99),
            phase: 0.0,
        }
    }
}

impl Processor for Clock {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        // Quarter-notes per second → phase increment per sample.
        let dt = (self.bpm / 60.0) / sr;
        for out in outputs[0].iter_mut().take(ctx.frames) {
            *out = if self.phase < self.pulse_width {
                1.0
            } else {
                0.0
            };
            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(c: &mut Clock, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0_f32; frames];
        {
            let ins: [&[f32]; 0] = [];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            c.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn rising_edges(xs: &[f32]) -> usize {
        xs.windows(2).filter(|w| w[0] < 0.5 && w[1] >= 0.5).count()
    }

    #[test]
    fn it_ticks_at_the_musical_tempo() {
        // 120 BPM = 2 beats/s. Two seconds of audio → 4 beats. The first beat starts high
        // (no rising edge for it), so expect 3 rising edges after it.
        let out = run(&mut Clock::new(120.0, 0.5), (SR * 2.0) as usize);
        assert_eq!(rising_edges(&out), 3, "120 BPM over 2 s → 4 beats");
        assert!(out[0] >= 0.5, "the clock starts on a pulse");
    }

    #[test]
    fn a_faster_tempo_ticks_more_often() {
        let slow = run(&mut Clock::new(60.0, 0.5), SR as usize);
        let fast = run(&mut Clock::new(240.0, 0.5), SR as usize);
        assert!(rising_edges(&fast) > rising_edges(&slow));
    }

    #[test]
    fn pulse_width_sets_the_duty_cycle() {
        // Fraction of samples high ≈ pulse_width over a whole number of beats.
        let out = run(&mut Clock::new(120.0, 0.25), (SR * 2.0) as usize);
        let duty = out.iter().filter(|&&s| s >= 0.5).count() as f32 / out.len() as f32;
        assert!((duty - 0.25).abs() < 0.02, "duty ≈ 0.25, got {duty}");
    }

    #[test]
    fn output_is_a_clean_gate() {
        let out = run(&mut Clock::new(90.0, 0.5), 10_000);
        assert!(out.iter().all(|&s| s == 0.0 || s == 1.0));
    }

    #[test]
    fn parameters_are_clamped() {
        let c = Clock::new(-5.0, 2.0);
        assert!(c.bpm > 0.0);
        assert!((c.pulse_width - 0.99).abs() < 1e-6);
    }
}
