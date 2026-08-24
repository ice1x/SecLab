//! Slew limiter: cap how fast a control signal may rise and fall.
//!
//! Where [`crate::Glide`] ramps to a target over a single time, a slew limiter bounds the
//! per-sample *rate of change* with **separate rise and fall** limits. It rounds off steps
//! and edges — turning a stepped sample-and-hold or a square LFO into gliding portamento or
//! a lopsided ramp (snap up, ease down, or vice-versa), and taming abrupt CV jumps. With
//! both times at zero it passes the input through unchanged.

use vb_core::{ProcessContext, Processor};

/// An asymmetric slew-rate limiter for a control signal.
///
/// Input 0 is the control signal to follow. Output 0 tracks it but may move at most
/// `1/(rise·sr)` per sample upward and `1/(fall·sr)` per sample downward — i.e. `rise`/`fall`
/// are the seconds a full `0→1` (or `1→0`) transition takes at most.
#[derive(Debug, Clone)]
pub struct Slew {
    rise_s: f32,
    fall_s: f32,
    current: f32,
    primed: bool,
}

impl Slew {
    /// Creates a slew limiter with rise and fall times (seconds).
    #[must_use]
    pub fn new(rise_s: f32, fall_s: f32) -> Self {
        Self {
            rise_s,
            fall_s,
            current: 0.0,
            primed: false,
        }
    }

    /// The current output value.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.current
    }

    /// Maximum per-sample step for a full-scale transition time `t` at `sr`.
    fn max_step(t: f32, sr: f32) -> f32 {
        if t > 0.0 {
            1.0 / (t * sr)
        } else {
            f32::INFINITY
        }
    }
}

impl Processor for Slew {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let up = Self::max_step(self.rise_s, sr);
        let down = Self::max_step(self.fall_s, sr);
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let target = input.get(i).copied().unwrap_or(self.current);
            // Snap to the very first input so the output starts on the signal, not at 0.
            if !self.primed {
                self.current = target;
                self.primed = true;
            } else {
                let delta = target - self.current;
                if delta > up {
                    self.current += up;
                } else if delta < -down {
                    self.current -= down;
                } else {
                    self.current = target;
                }
            }
            *out = self.current;
        }
    }

    fn reset(&mut self) {
        self.current = 0.0;
        self.primed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(slew: &mut Slew, sr: f32, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: sr,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            slew.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn a_step_up_is_rate_limited() {
        // rise 0.01 s at 1 kHz → max step 1/(0.01*1000) = 0.1 per sample.
        // First sample primes to input[0]=0; then a step to 1.0 climbs by ≤0.1 each sample.
        let mut input = vec![0.0_f32; 1];
        input.extend(std::iter::repeat_n(1.0, 20));
        let out = run(&mut Slew::new(0.01, 0.01), 1_000.0, &input);
        assert!((out[0]).abs() < 1e-6, "primed to the initial 0");
        assert!(
            (out[1] - 0.1).abs() < 1e-4,
            "first step is +0.1, got {}",
            out[1]
        );
        assert!(
            (out[2] - 0.2).abs() < 1e-4,
            "then +0.1 again, got {}",
            out[2]
        );
        assert!(
            out.windows(2).all(|w| w[1] - w[0] <= 0.1 + 1e-4),
            "never exceeds the rise rate"
        );
        assert!(
            (out[out.len() - 1] - 1.0).abs() < 1e-4,
            "eventually reaches the target"
        );
    }

    #[test]
    fn rise_and_fall_rates_are_independent() {
        // Fast rise (0.001 s), slow fall (0.1 s): rising covers a step far quicker than falling.
        let mut up = vec![0.0_f32; 1];
        up.extend(std::iter::repeat_n(1.0, 2_000));
        let out_up = run(&mut Slew::new(0.001, 0.1), 1_000.0, &up);
        let rise_reach = out_up.iter().position(|&v| v >= 0.99).unwrap_or(usize::MAX);

        let mut down = vec![1.0_f32; 1]; // primes to 1.0
        down.extend(std::iter::repeat_n(0.0, 2_000));
        let out_down = run(&mut Slew::new(0.001, 0.1), 1_000.0, &down);
        let fall_reach = out_down
            .iter()
            .position(|&v| v <= 0.01)
            .unwrap_or(usize::MAX);

        assert!(
            rise_reach < fall_reach,
            "fast rise ({rise_reach}) beats slow fall ({fall_reach})"
        );
    }

    #[test]
    fn zero_times_pass_through_unchanged() {
        let mut slew = Slew::new(0.0, 0.0);
        let input = [0.0, 1.0, -1.0, 0.5, 0.5];
        let out = run(&mut slew, 1_000.0, &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "instant follow: {a} vs {b}");
        }
    }

    #[test]
    fn primes_to_the_first_input_value() {
        // Starting far from 0, the output does not crawl up from 0 — it begins on the signal.
        let out = run(&mut Slew::new(0.01, 0.01), 1_000.0, &[0.9, 0.9, 0.9]);
        assert!((out[0] - 0.9).abs() < 1e-6, "starts on the input, not 0");
    }
}
