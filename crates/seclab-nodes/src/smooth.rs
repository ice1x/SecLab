//! Parameter smoothing for the real-time path: a linear ramp that turns an abrupt
//! parameter change into a short glide, so a jumped gain or cutoff does not click.
//!
//! Stepping a control value straight to its new level puts a discontinuity into the
//! signal — an audible "zipper" or click. A [`Smoother`] instead walks from the current
//! value to the target over a fixed number of samples. It is allocation-free and
//! branch-light (one counter), so it is safe to advance per sample on the audio thread;
//! retargeting mid-ramp simply aims at the new value from wherever it is now.

/// A linear parameter ramp: glides `current` to `target` over a set number of samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Smoother {
    current: f32,
    target: f32,
    /// Per-sample increment while ramping (0 once settled).
    step: f32,
    /// Samples left in the ramp (0 once settled).
    remaining: u32,
}

impl Smoother {
    /// A settled smoother sitting at `initial`.
    #[must_use]
    pub fn new(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            step: 0.0,
            remaining: 0,
        }
    }

    /// Jumps immediately to `value` with no ramp (e.g. on reset or preset load).
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.step = 0.0;
        self.remaining = 0;
    }

    /// Aims at `target`, gliding there over `ramp_samples`. A zero-length ramp (or an
    /// unchanged target) settles immediately. Retargeting mid-ramp re-aims from the
    /// current value.
    pub fn set_target(&mut self, target: f32, ramp_samples: u32) {
        self.target = target;
        if ramp_samples == 0 || target == self.current {
            self.current = target;
            self.step = 0.0;
            self.remaining = 0;
        } else {
            self.step = (target - self.current) / ramp_samples as f32;
            self.remaining = ramp_samples;
        }
    }

    /// Advances one sample and returns the new current value. Snaps exactly onto the
    /// target on the final step so rounding never leaves a residue.
    pub fn next_sample(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            if self.remaining == 0 {
                self.current = self.target;
                self.step = 0.0;
            } else {
                self.current += self.step;
            }
        }
        self.current
    }

    /// The current value without advancing.
    #[must_use]
    pub fn current(&self) -> f32 {
        self.current
    }

    /// The value being ramped toward.
    #[must_use]
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether the ramp has reached its target.
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.remaining == 0
    }

    /// Fills `out` with successive ramp values, one per sample (allocation-free).
    pub fn process_block(&mut self, out: &mut [f32]) {
        for slot in out {
            *slot = self.next_sample();
        }
    }
}

/// Ramp length in samples for a `ms`-millisecond glide at `sample_rate`, at least one
/// sample for any positive time.
#[must_use]
pub fn ramp_samples(ms: f32, sample_rate: f32) -> u32 {
    if ms <= 0.0 || sample_rate <= 0.0 {
        return 0;
    }
    (ms * 0.001 * sample_rate).round().max(1.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_smoother_sits_settled_at_its_initial_value() {
        let mut s = Smoother::new(0.5);
        assert!(s.is_settled());
        assert_eq!(s.current(), 0.5);
        assert_eq!(s.next_sample(), 0.5, "settled: next does not move");
    }

    #[test]
    fn set_target_ramps_linearly_and_lands_exactly_on_target() {
        let mut s = Smoother::new(0.0);
        s.set_target(1.0, 4);
        assert!(!s.is_settled());
        let steps: Vec<f32> = (0..4).map(|_| s.next_sample()).collect();
        assert_eq!(steps, vec![0.25, 0.5, 0.75, 1.0]);
        assert!(s.is_settled(), "settled after exactly N samples");
        assert_eq!(s.next_sample(), 1.0, "stays on target afterwards");
        assert_eq!(s.current(), 1.0);
    }

    #[test]
    fn a_zero_length_ramp_jumps_immediately() {
        let mut s = Smoother::new(0.0);
        s.set_target(0.9, 0);
        assert!(s.is_settled());
        assert_eq!(s.current(), 0.9);
    }

    #[test]
    fn retargeting_mid_ramp_reaims_from_the_current_value() {
        let mut s = Smoother::new(0.0);
        s.set_target(1.0, 4);
        assert_eq!(s.next_sample(), 0.25);
        assert_eq!(s.next_sample(), 0.5);
        // Change the destination mid-glide: ramp from 0.5 to 0.0 over 2 samples.
        s.set_target(0.0, 2);
        assert_eq!(s.next_sample(), 0.25);
        assert_eq!(s.next_sample(), 0.0);
        assert!(s.is_settled());
    }

    #[test]
    fn reset_snaps_without_a_ramp() {
        let mut s = Smoother::new(0.0);
        s.set_target(1.0, 100);
        s.next_sample();
        s.reset(0.3);
        assert!(s.is_settled());
        assert_eq!(s.current(), 0.3);
        assert_eq!(s.target(), 0.3);
    }

    #[test]
    fn process_block_continues_a_ramp_across_calls() {
        let mut s = Smoother::new(0.0);
        s.set_target(1.0, 8);
        let mut a = [0.0_f32; 4];
        let mut b = [0.0_f32; 4];
        s.process_block(&mut a);
        s.process_block(&mut b);
        assert_eq!(a, [0.125, 0.25, 0.375, 0.5]);
        assert_eq!(b, [0.625, 0.75, 0.875, 1.0]);
        assert!(s.is_settled());
    }

    #[test]
    fn ramp_samples_converts_milliseconds() {
        // 5 ms at 48 kHz = 240 samples.
        assert_eq!(ramp_samples(5.0, 48_000.0), 240);
        // Non-positive time or rate → no ramp.
        assert_eq!(ramp_samples(0.0, 48_000.0), 0);
        assert_eq!(ramp_samples(5.0, 0.0), 0);
        // A tiny positive time still rounds up to at least one sample.
        assert_eq!(ramp_samples(0.001, 48_000.0), 1);
    }
}
