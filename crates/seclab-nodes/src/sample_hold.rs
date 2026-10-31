//! Sample & hold: latch an input value on each trigger edge and hold it until the next.
//!
//! A cornerstone modular utility. On every rising edge of the `trig` input it samples the
//! current `in` value and outputs that constant until the next trigger. Fed noise and a
//! clock it produces the classic stepped-random modulation (random arpeggios, "computer"
//! sounds); fed a slow LFO it quantises that LFO into stairs locked to the clock.

use seclab_core::{ProcessContext, Processor};

/// A sample-and-hold latch.
///
/// Input 0 is the signal to sample (`in`); input 1 is the trigger (`trig`, control): a
/// rising edge (`< 0.5` → `>= 0.5`) latches `in`. Output 0 holds the last sampled value,
/// starting at 0 before the first trigger.
#[derive(Debug, Clone)]
pub struct SampleHold {
    held: f32,
    trig_prev: bool,
}

impl SampleHold {
    /// Creates a sample & hold holding 0 until its first trigger.
    #[must_use]
    pub fn new() -> Self {
        Self {
            held: 0.0,
            trig_prev: false,
        }
    }

    /// The value currently held.
    #[must_use]
    pub fn held(&self) -> f32 {
        self.held
    }
}

impl Default for SampleHold {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor for SampleHold {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let signal = inputs.first().copied().unwrap_or(&[]);
        let trig = inputs.get(1).copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let gate = trig.get(i).copied().unwrap_or(0.0) >= 0.5;
            if gate && !self.trig_prev {
                self.held = signal.get(i).copied().unwrap_or(self.held);
            }
            self.trig_prev = gate;
            *out = self.held;
        }
    }

    fn reset(&mut self) {
        self.held = 0.0;
        self.trig_prev = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(sh: &mut SampleHold, signal: &[f32], trig: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: signal.len(),
        };
        let mut out = vec![0.0_f32; signal.len()];
        {
            let ins: [&[f32]; 2] = [signal, trig];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            sh.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn holds_zero_until_the_first_trigger() {
        let out = run(
            &mut SampleHold::new(),
            &[0.1, 0.2, 0.3, 0.4],
            &[0.0, 0.0, 0.0, 0.0],
        );
        assert!(
            out.iter().all(|&s| s == 0.0),
            "no trigger → holds the initial 0"
        );
    }

    #[test]
    fn latches_the_input_on_a_rising_edge() {
        // Trigger at index 1: it samples signal[1] = 0.7 and holds it thereafter.
        let signal = [0.1, 0.7, 0.2, 0.9];
        let trig = [0.0, 1.0, 1.0, 0.0]; // rising edge only at index 1
        let out = run(&mut SampleHold::new(), &signal, &trig);
        assert_eq!(out[0], 0.0);
        assert!((out[1] - 0.7).abs() < 1e-6, "sampled 0.7 at the edge");
        assert!((out[2] - 0.7).abs() < 1e-6, "held (no new rising edge)");
        assert!(
            (out[3] - 0.7).abs() < 1e-6,
            "still held after the gate falls"
        );
    }

    #[test]
    fn each_new_edge_samples_again() {
        let signal = [0.3, 0.3, 0.8, 0.8, 0.2];
        let trig = [1.0, 0.0, 1.0, 0.0, 1.0]; // rising edges at 0, 2, 4
        let out = run(&mut SampleHold::new(), &signal, &trig);
        assert!((out[0] - 0.3).abs() < 1e-6);
        assert!((out[1] - 0.3).abs() < 1e-6);
        assert!((out[2] - 0.8).abs() < 1e-6, "re-sampled at the second edge");
        assert!((out[4] - 0.2).abs() < 1e-6, "re-sampled at the third edge");
    }

    #[test]
    fn reset_clears_the_held_value() {
        let mut sh = SampleHold::new();
        run(&mut sh, &[0.5], &[1.0]);
        assert!((sh.held() - 0.5).abs() < 1e-6);
        sh.reset();
        assert_eq!(sh.held(), 0.0);
    }
}
