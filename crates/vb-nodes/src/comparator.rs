//! Comparator: turn two control signals into a gate.
//!
//! Outputs `1` when input `a` rises above input `b`, `0` when it falls below — the CV
//! equivalent of a logic gate. `hysteresis` widens the switching band (a Schmitt trigger):
//! the output only flips high once `a` clears `b + hysteresis`, and only flips low once `a`
//! drops under `b - hysteresis`, so a noisy or slowly-crossing signal does not chatter. With
//! `hysteresis = 0` it is a plain threshold comparison. Drive an envelope, trigger a
//! sample-and-hold, or build rhythmic gates from an LFO against a fixed level.

use vb_core::{ProcessContext, Processor};

/// A control-rate comparator with optional hysteresis.
///
/// Input 0 is `a`, input 1 is `b` (both control). Output 0 is a gate: `1.0` while `a` is
/// above `b` (by more than the hysteresis band), `0.0` while below. The current state is
/// held inside the band, so the two thresholds create a Schmitt trigger.
#[derive(Debug, Clone)]
pub struct Comparator {
    hysteresis: f32,
    high: bool,
}

impl Comparator {
    /// Creates a comparator. `hysteresis` is clamped to `>= 0`.
    #[must_use]
    pub fn new(hysteresis: f32) -> Self {
        Self {
            hysteresis: hysteresis.max(0.0),
            high: false,
        }
    }
}

impl Processor for Comparator {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let a = inputs.first().copied().unwrap_or(&[]);
        let b = inputs.get(1).copied().unwrap_or(&[]);
        let h = self.hysteresis;

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let av = a.get(i).copied().unwrap_or(0.0);
            let bv = b.get(i).copied().unwrap_or(0.0);
            if av > bv + h {
                self.high = true;
            } else if av < bv - h {
                self.high = false;
            }
            *out = if self.high { 1.0 } else { 0.0 };
        }
    }

    fn reset(&mut self) {
        self.high = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(c: &mut Comparator, a: &[f32], b: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: a.len(),
        };
        let mut out = vec![0.0_f32; a.len()];
        {
            let ins: [&[f32]; 2] = [a, b];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            c.process(&ctx, &ins, &mut outs);
        }
        out
    }

    #[test]
    fn high_when_a_exceeds_b() {
        let out = run(&mut Comparator::new(0.0), &[0.8, 0.2], &[0.5, 0.5]);
        assert_eq!(out, vec![1.0, 0.0]);
    }

    #[test]
    fn equal_inputs_stay_low_without_hysteresis() {
        // a == b is neither above nor below → holds the initial (low) state.
        let out = run(&mut Comparator::new(0.0), &[0.5], &[0.5]);
        assert_eq!(out, vec![0.0]);
    }

    #[test]
    fn hysteresis_holds_state_inside_the_band() {
        // Band ±0.2 around b=0.5. Sequence: 0.8 (→high), 0.6 (inside → hold high),
        // 0.4 (inside → hold high), 0.2 (below 0.3 → low), 0.4 (inside → hold low).
        let a = [0.8, 0.6, 0.4, 0.2, 0.4];
        let b = [0.5; 5];
        let out = run(&mut Comparator::new(0.2), &a, &b);
        assert_eq!(out, vec![1.0, 1.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn reset_returns_to_low() {
        let mut c = Comparator::new(0.0);
        let _ = run(&mut c, &[1.0], &[0.0]);
        assert!(c.high);
        c.reset();
        assert!(!c.high);
    }

    #[test]
    fn hysteresis_is_clamped() {
        let c = Comparator::new(-1.0);
        assert!((c.hysteresis - 0.0).abs() < 1e-6);
    }
}
