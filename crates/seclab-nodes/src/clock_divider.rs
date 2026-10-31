//! Clock divider: pass every Nth gate pulse.
//!
//! Counts the rising edges of an incoming gate and lets one through for every `division`
//! it sees — turning a steady clock into a slower one. `division = 2` halves the tempo,
//! `4` quarters it, `1` passes every pulse. The output reproduces the full width of the
//! qualifying input pulse, so downstream envelopes and triggers see a clean gate. Pair it
//! with the [`crate::clock::Clock`] node to build polyrhythms from a single beat.

use seclab_core::{ProcessContext, Processor};

/// A rising-edge gate divider.
///
/// Input 0 is a gate (control). Output 0 is a gate that mirrors every `division`-th input
/// pulse and is `0` otherwise. Edge detection uses a `0.5` threshold.
#[derive(Debug, Clone)]
pub struct ClockDivider {
    division: u32,
    /// Whether the previous sample was high (for rising-edge detection).
    prev_high: bool,
    /// Rising edges seen so far, counted modulo `division`.
    count: u32,
    /// Whether the current input pulse is being passed through.
    active: bool,
}

impl ClockDivider {
    /// Creates a divider. `division` is clamped to `>= 1`.
    #[must_use]
    pub fn new(division: u32) -> Self {
        Self {
            division: division.max(1),
            prev_high: false,
            count: 0,
            active: false,
        }
    }
}

impl Processor for ClockDivider {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let high = input.get(i).copied().unwrap_or(0.0) > 0.5;
            if high && !self.prev_high {
                // Rising edge: advance the counter and decide if this pulse passes.
                self.count += 1;
                if self.count >= self.division {
                    self.count = 0;
                    self.active = true;
                } else {
                    self.active = false;
                }
            }
            if !high {
                // Pulse ended — stop passing until the next qualifying edge.
                self.active = false;
            }
            self.prev_high = high;
            *out = if high && self.active { 1.0 } else { 0.0 };
        }
    }

    fn reset(&mut self) {
        self.prev_high = false;
        self.count = 0;
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(d: &mut ClockDivider, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            d.process(&ctx, &ins, &mut outs);
        }
        out
    }

    /// A gate stream of `n` pulses, each `[1, 1, 0, 0]` (one rising edge per group).
    fn pulses(n: usize) -> Vec<f32> {
        let mut v = Vec::with_capacity(n * 4);
        for _ in 0..n {
            v.extend_from_slice(&[1.0, 1.0, 0.0, 0.0]);
        }
        v
    }

    fn rising_edges(xs: &[f32]) -> usize {
        xs.windows(2).filter(|w| w[0] < 0.5 && w[1] >= 0.5).count()
    }

    #[test]
    fn divide_by_two_passes_every_second_pulse() {
        let out = run(&mut ClockDivider::new(2), &pulses(8));
        assert_eq!(rising_edges(&out), 4, "8 pulses / 2 → 4");
    }

    #[test]
    fn divide_by_one_is_transparent() {
        let input = pulses(6);
        let out = run(&mut ClockDivider::new(1), &input);
        assert_eq!(out, input, "division 1 passes every pulse unchanged");
    }

    #[test]
    fn divide_by_four_passes_every_fourth_pulse() {
        let out = run(&mut ClockDivider::new(4), &pulses(12));
        assert_eq!(rising_edges(&out), 3, "12 pulses / 4 → 3");
    }

    #[test]
    fn the_passed_pulse_keeps_the_input_width() {
        // Every 2nd pulse passes; the output pulse is as wide as the input's high portion.
        let out = run(&mut ClockDivider::new(2), &pulses(2));
        let high = out.iter().filter(|&&s| s >= 0.5).count();
        assert_eq!(high, 2, "one passed pulse, two samples wide");
    }

    #[test]
    fn reset_clears_the_count() {
        let mut d = ClockDivider::new(3);
        let _ = run(&mut d, &pulses(2));
        assert_eq!(d.count, 2);
        d.reset();
        assert_eq!(d.count, 0);
        assert!(!d.active && !d.prev_high);
    }

    #[test]
    fn division_is_clamped() {
        let d = ClockDivider::new(0);
        assert_eq!(d.division, 1);
    }
}
