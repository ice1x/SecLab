//! Shared Direct Form I biquad core used by the RBJ cookbook filter and EQ nodes.
//!
//! Every second-order cookbook node ([`crate::Biquad`], [`crate::ShelfEq`], [`crate::PeakEq`],
//! [`crate::Allpass`]) computes a different set of five normalised coefficients but then runs
//! the *same* Direct Form I difference equation and keeps the *same* two-input/two-output delay
//! state. That shared arithmetic lives here so each node only has to spell out its own
//! coefficient formula.
//!
//! Direct Form I: `y[n] = b0·x[n] + b1·x[n−1] + b2·x[n−2] − a1·y[n−1] − a2·y[n−2]`, with the
//! coefficients already divided by `a0`.

/// Normalised biquad coefficients (already divided by `a0`).
#[derive(Debug, Clone, Copy)]
pub struct BiquadCoeffs {
    /// Feed-forward coefficient on the current input.
    pub b0: f32,
    /// Feed-forward coefficient on `x[n−1]`.
    pub b1: f32,
    /// Feed-forward coefficient on `x[n−2]`.
    pub b2: f32,
    /// Feed-back coefficient on `y[n−1]`.
    pub a1: f32,
    /// Feed-back coefficient on `y[n−2]`.
    pub a2: f32,
}

impl BiquadCoeffs {
    /// Normalises a raw transfer function (`b` over `a`) by dividing every coefficient by `a0`.
    ///
    /// This is the one place the RBJ cookbook's `a0` division is written; each node hands in its
    /// un-normalised `b0,b1,b2,a0,a1,a2` and gets back a ready-to-run [`BiquadCoeffs`].
    #[must_use]
    pub fn normalized(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// Direct Form I delay state: the two past inputs and two past outputs.
#[derive(Debug, Clone, Copy, Default)]
pub struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadState {
    /// Advances one sample through `c`, returning `y[n]` and shifting the delay line.
    #[inline]
    #[must_use]
    pub fn step(&mut self, c: &BiquadCoeffs, x: f32) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Clears the delay line back to silence.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_divides_every_coefficient_by_a0() {
        let c = BiquadCoeffs::normalized(2.0, 4.0, 6.0, 2.0, 8.0, 10.0);
        assert!((c.b0 - 1.0).abs() < 1e-7);
        assert!((c.b1 - 2.0).abs() < 1e-7);
        assert!((c.b2 - 3.0).abs() < 1e-7);
        assert!((c.a1 - 4.0).abs() < 1e-7);
        assert!((c.a2 - 5.0).abs() < 1e-7);
    }

    #[test]
    fn identity_coeffs_pass_the_signal_through() {
        // b0 = 1, everything else 0 is H(z) = 1: the output is the input, unchanged.
        let c = BiquadCoeffs::normalized(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        let mut s = BiquadState::default();
        let input = [0.1_f32, -0.5, 0.9, 0.0, 0.3];
        for &x in &input {
            assert!((s.step(&c, x) - x).abs() < 1e-7);
        }
    }

    #[test]
    fn step_matches_the_direct_form_one_difference_equation() {
        // Hand-roll the recurrence next to the state machine and require bit-for-bit agreement
        // over a run of samples — this is the exact arithmetic the nodes used to inline.
        let c = BiquadCoeffs::normalized(0.5, -0.2, 0.1, 1.0, -0.3, 0.05);
        let input = [1.0_f32, 0.0, -1.0, 0.7, 0.2, -0.4, 0.9];

        let mut s = BiquadState::default();
        let (mut x1, mut x2, mut y1, mut y2) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        for &x in &input {
            let expected = c.b0 * x + c.b1 * x1 + c.b2 * x2 - c.a1 * y1 - c.a2 * y2;
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = expected;

            let got = s.step(&c, x);
            assert_eq!(
                got.to_bits(),
                expected.to_bits(),
                "step must be bit-identical"
            );
        }
    }

    #[test]
    fn reset_clears_the_delay_line() {
        let c = BiquadCoeffs::normalized(0.5, 0.25, 0.125, 1.0, -0.5, 0.25);
        let mut s = BiquadState::default();
        for &x in &[1.0_f32, 0.8, -0.3, 0.6] {
            let _ = s.step(&c, x);
        }
        s.reset();
        // With a cleared line the first output is exactly b0 · x (no history contributes).
        let x = 0.42_f32;
        assert_eq!(s.step(&c, x).to_bits(), (c.b0 * x).to_bits());
    }
}
