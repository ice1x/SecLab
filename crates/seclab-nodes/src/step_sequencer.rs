//! Step sequencer: a clocked bank of CV values.
//!
//! Holds up to [`STEPS`] control values and steps through the first `steps` of them,
//! advancing one position on every rising edge of an incoming clock gate. Between edges
//! the output holds the current step (sample-and-hold), so it drives a pitch input, a
//! filter cutoff, or any other CV directly. Clock it from the [`crate::clock::Clock`]
//! node — optionally through a [`crate::clock_divider::ClockDivider`] — and quantize the
//! output with [`crate::quantizer::Quantizer`] to lock a melody to a scale.

use seclab_core::{ProcessContext, Processor};

/// Maximum number of steps the sequencer holds.
pub const STEPS: usize = 8;

/// A rising-edge-clocked CV step sequencer.
///
/// Input 0 is a clock gate (control). Output 0 is the current step's value (control),
/// held until the next rising edge. Edge detection uses a `0.5` threshold.
#[derive(Debug, Clone)]
pub struct StepSequencer {
    /// The value emitted at each step position.
    values: [f32; STEPS],
    /// How many of `values` are active in the cycle, clamped to `1..=STEPS`.
    steps: usize,
    /// The step currently being emitted.
    index: usize,
    /// Whether the previous sample was high (for rising-edge detection).
    prev_high: bool,
}

impl StepSequencer {
    /// Creates a sequencer over `values`, cycling through the first `steps` of them.
    ///
    /// `steps` is clamped to `1..=STEPS`.
    #[must_use]
    pub fn new(steps: u32, values: [f32; STEPS]) -> Self {
        Self {
            values,
            steps: (steps as usize).clamp(1, STEPS),
            index: 0,
            prev_high: false,
        }
    }
}

impl Processor for StepSequencer {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let clock = inputs.first().copied().unwrap_or(&[]);
        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let high = clock.get(i).copied().unwrap_or(0.0) > 0.5;
            if high && !self.prev_high {
                // Rising edge: advance to the next step, wrapping at the active length.
                self.index = (self.index + 1) % self.steps;
            }
            self.prev_high = high;
            *out = self.values[self.index];
        }
    }

    fn reset(&mut self) {
        self.index = 0;
        self.prev_high = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(seq: &mut StepSequencer, clock: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: clock.len(),
        };
        let mut out = vec![0.0_f32; clock.len()];
        {
            let ins: [&[f32]; 1] = [clock];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            seq.process(&ctx, &ins, &mut outs);
        }
        out
    }

    /// A clock stream of `n` pulses, each `[1, 1, 0, 0]` (one rising edge per group).
    fn pulses(n: usize) -> Vec<f32> {
        let mut v = Vec::with_capacity(n * 4);
        for _ in 0..n {
            v.extend_from_slice(&[1.0, 1.0, 0.0, 0.0]);
        }
        v
    }

    #[test]
    fn holds_the_first_step_before_any_edge() {
        let mut seq = StepSequencer::new(4, [0.1, 0.2, 0.3, 0.4, 0.0, 0.0, 0.0, 0.0]);
        let out = run(&mut seq, &[0.0, 0.0, 0.0]);
        assert!(out.iter().all(|&s| (s - 0.1).abs() < 1e-6), "step 0 held");
    }

    #[test]
    fn each_rising_edge_advances_one_step() {
        let mut seq = StepSequencer::new(4, [0.1, 0.2, 0.3, 0.4, 0.0, 0.0, 0.0, 0.0]);
        // Sample the held value just before each pulse's rising edge: the value in effect
        // is that of the step reached by the previous edges.
        let out = run(&mut seq, &pulses(3));
        // pulses(3): edges at samples 0(no prev), 4, 8. Output after each edge:
        // frame 0 rising -> index 1 (0.2); frames 4 rising -> index 2 (0.3); 8 -> index 3.
        assert!(
            (out[0] - 0.2).abs() < 1e-6,
            "first edge -> step 1: {}",
            out[0]
        );
        assert!(
            (out[4] - 0.3).abs() < 1e-6,
            "second edge -> step 2: {}",
            out[4]
        );
        assert!(
            (out[8] - 0.4).abs() < 1e-6,
            "third edge -> step 3: {}",
            out[8]
        );
    }

    #[test]
    fn wraps_at_the_active_length() {
        let mut seq = StepSequencer::new(3, [10.0, 20.0, 30.0, 99.0, 0.0, 0.0, 0.0, 0.0]);
        // 3 active steps: edges cycle 1,2,0,1,2,0...; the 4th value (99) is never emitted.
        let out = run(&mut seq, &pulses(6));
        let sampled: Vec<f32> = (0..6).map(|k| out[k * 4]).collect();
        assert_eq!(sampled, vec![20.0, 30.0, 10.0, 20.0, 30.0, 10.0]);
        assert!(!out.contains(&99.0), "inactive step is never reached");
    }

    #[test]
    fn a_single_active_step_is_a_constant() {
        let mut seq = StepSequencer::new(1, [7.0, 1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0]);
        let out = run(&mut seq, &pulses(5));
        assert!(out.iter().all(|&s| (s - 7.0).abs() < 1e-6), "always step 0");
    }

    #[test]
    fn steps_is_clamped_into_range() {
        assert_eq!(StepSequencer::new(0, [0.0; STEPS]).steps, 1);
        assert_eq!(StepSequencer::new(99, [0.0; STEPS]).steps, STEPS);
    }

    #[test]
    fn reset_returns_to_the_first_step() {
        let mut seq = StepSequencer::new(4, [0.1, 0.2, 0.3, 0.4, 0.0, 0.0, 0.0, 0.0]);
        let _ = run(&mut seq, &pulses(2));
        assert_ne!(seq.index, 0);
        seq.reset();
        assert_eq!(seq.index, 0);
        assert!(!seq.prev_high);
    }
}
