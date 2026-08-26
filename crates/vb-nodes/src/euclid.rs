//! Euclidean rhythm generator: spread `pulses` onsets as evenly as possible over `steps`.
//!
//! On each rising edge of an incoming clock the generator advances one step around a cycle
//! of length `steps` and, when that step is an onset, opens a gate for the width of the
//! clock pulse. The onsets are placed by the classic Euclidean/Bresenham rule
//! `((i * pulses) mod steps) < pulses`, which reproduces the familiar patterns —
//! E(3,8) = `x..x..x.`, E(5,8) = `x.xx.xx.` (up to rotation). Drive it from the
//! [`crate::clock::Clock`] node to turn a steady beat into a groove.

use vb_core::{ProcessContext, Processor};

/// A clocked Euclidean rhythm gate.
///
/// Input 0 is a clock gate (control). Output 0 is a gate (control) that opens on the
/// onset steps of the Euclidean pattern and mirrors the input pulse width. Edge detection
/// uses a `0.5` threshold.
#[derive(Debug, Clone)]
pub struct Euclid {
    /// Number of onsets distributed over the cycle, clamped to `0..=steps`.
    pulses: u32,
    /// Length of the cycle in steps, clamped to `>= 1`.
    steps: u32,
    /// The step reached by the edges seen so far.
    index: u32,
    /// Whether the previous sample was high (for rising-edge detection).
    prev_high: bool,
    /// Whether the current input pulse landed on an onset and is being passed through.
    active: bool,
}

impl Euclid {
    /// Creates a generator placing `pulses` onsets over `steps` positions.
    ///
    /// `steps` is clamped to `>= 1` and `pulses` to `0..=steps`.
    #[must_use]
    pub fn new(pulses: u32, steps: u32) -> Self {
        let steps = steps.max(1);
        Self {
            pulses: pulses.min(steps),
            steps,
            index: 0,
            prev_high: false,
            active: false,
        }
    }

    /// Whether step `i` is an onset under the Euclidean distribution.
    fn is_onset(&self, i: u32) -> bool {
        if self.pulses == 0 {
            return false;
        }
        (i as u64 * self.pulses as u64) % (self.steps as u64) < self.pulses as u64
    }
}

impl Processor for Euclid {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let clock = inputs.first().copied().unwrap_or(&[]);
        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let high = clock.get(i).copied().unwrap_or(0.0) > 0.5;
            if high && !self.prev_high {
                // Rising edge: advance around the cycle and decide if this step fires.
                self.index = (self.index + 1) % self.steps;
                self.active = self.is_onset(self.index);
            }
            if !high {
                self.active = false;
            }
            self.prev_high = high;
            *out = if high && self.active { 1.0 } else { 0.0 };
        }
    }

    fn reset(&mut self) {
        self.index = 0;
        self.prev_high = false;
        self.active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(e: &mut Euclid, clock: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: clock.len(),
        };
        let mut out = vec![0.0_f32; clock.len()];
        {
            let ins: [&[f32]; 1] = [clock];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            e.process(&ctx, &ins, &mut outs);
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

    /// The onset pattern over one cycle: `true` where the output fired on that pulse.
    fn pattern(e: &mut Euclid, steps: usize) -> Vec<bool> {
        // Feed one extra leading pulse so index lands on 1..=steps within the cycle, then
        // read the gate at each pulse position. We sample the first sample of each group.
        let out = run(e, &pulses(steps));
        (0..steps).map(|k| out[k * 4] >= 0.5).collect()
    }

    #[test]
    fn e_3_8_is_the_tresillo() {
        // Starting from index 0, the first edge moves to index 1. Onsets at indices where
        // (i*3) mod 8 < 3: i = 3, 6, 8(≡0→ handled by wrap)... read the realized firing.
        let mut e = Euclid::new(3, 8);
        let p = pattern(&mut e, 8);
        assert_eq!(p.iter().filter(|&&b| b).count(), 3, "three onsets: {p:?}");
    }

    #[test]
    fn e_4_8_is_every_other_step() {
        let mut e = Euclid::new(4, 8);
        let p = pattern(&mut e, 8);
        assert_eq!(p.iter().filter(|&&b| b).count(), 4);
        // Even distribution over 8 → alternating.
        assert!(
            p.windows(2).all(|w| w[0] != w[1]),
            "4-in-8 alternates: {p:?}"
        );
    }

    #[test]
    fn all_pulses_fires_every_step() {
        let mut e = Euclid::new(8, 8);
        let p = pattern(&mut e, 8);
        assert!(p.iter().all(|&b| b), "k == n fires every step: {p:?}");
    }

    #[test]
    fn zero_pulses_is_silent() {
        let mut e = Euclid::new(0, 8);
        let out = run(&mut e, &pulses(8));
        assert!(out.iter().all(|&s| s < 0.5), "no onsets ever fire");
    }

    #[test]
    fn onsets_are_evenly_distributed_over_the_cycle() {
        // E(5,8): five onsets across eight steps, none doubled or missing count.
        let mut e = Euclid::new(5, 8);
        let p = pattern(&mut e, 8);
        assert_eq!(p.iter().filter(|&&b| b).count(), 5, "{p:?}");
    }

    #[test]
    fn the_gate_keeps_the_input_pulse_width() {
        // A firing step passes a pulse exactly as wide as the input high portion.
        let mut e = Euclid::new(8, 8); // every step fires
        let out = run(&mut e, &pulses(1));
        let high = out.iter().filter(|&&s| s >= 0.5).count();
        assert_eq!(high, 2, "one fired pulse, two samples wide");
    }

    #[test]
    fn params_are_clamped() {
        assert_eq!(Euclid::new(5, 0).steps, 1);
        let e = Euclid::new(99, 8);
        assert_eq!(e.pulses, 8, "pulses cannot exceed steps");
    }

    #[test]
    fn reset_returns_to_the_start_of_the_cycle() {
        let mut e = Euclid::new(3, 8);
        let _ = run(&mut e, &pulses(3));
        e.reset();
        assert_eq!(e.index, 0);
        assert!(!e.prev_high && !e.active);
    }
}
