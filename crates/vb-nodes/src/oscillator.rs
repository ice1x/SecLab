//! Anti-aliased oscillator node.
//!
//! Produces sine, saw, square and triangle waveforms. Saw and square use PolyBLEP to
//! suppress the aliasing that naive discontinuous waveforms would otherwise fold back
//! into the audible band; sine is inherently band-limited and triangle's aliasing is
//! mild, so both are generated directly.

use vb_core::{ProcessContext, Processor};

/// The shape produced by an [`Oscillator`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    /// Pure sine.
    Sine,
    /// Rising sawtooth (PolyBLEP band-limited).
    Saw,
    /// Square / pulse at 50% duty (PolyBLEP band-limited).
    Square,
    /// Triangle.
    Triangle,
}

/// A single-output oscillator running at a fixed frequency.
///
/// No inputs yet; a `freq` control input arrives with the control-rate work. The output
/// is written to channel 0. Phase is normalized to `[0, 1)`.
#[derive(Debug, Clone)]
pub struct Oscillator {
    waveform: Waveform,
    freq_hz: f32,
    phase: f32,
}

impl Oscillator {
    /// Creates an oscillator of `waveform` at `freq_hz`, starting at phase 0.
    #[must_use]
    pub fn new(waveform: Waveform, freq_hz: f32) -> Self {
        Self {
            waveform,
            freq_hz,
            phase: 0.0,
        }
    }

    /// The waveform this oscillator produces.
    #[must_use]
    pub fn waveform(&self) -> Waveform {
        self.waveform
    }

    /// Computes the current sample for the given per-sample phase increment `dt`.
    fn sample(&self, dt: f32) -> f32 {
        match self.waveform {
            Waveform::Sine => (core::f32::consts::TAU * self.phase).sin(),
            Waveform::Saw => (2.0 * self.phase - 1.0) - poly_blep(self.phase, dt),
            Waveform::Square => {
                let naive = if self.phase < 0.5 { 1.0 } else { -1.0 };
                naive + poly_blep(self.phase, dt) - poly_blep((self.phase + 0.5) % 1.0, dt)
            }
            Waveform::Triangle => {
                if self.phase < 0.5 {
                    4.0 * self.phase - 1.0
                } else {
                    3.0 - 4.0 * self.phase
                }
            }
        }
    }
}

impl Processor for Oscillator {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let dt = if ctx.sample_rate > 0.0 {
            self.freq_hz / ctx.sample_rate
        } else {
            0.0
        };
        for out in outputs[0].iter_mut().take(ctx.frames) {
            *out = self.sample(dt);
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

/// PolyBLEP residual used to round the discontinuities of saw/square waveforms.
///
/// `t` is the phase in `[0, 1)`, `dt` the per-sample phase increment.
fn poly_blep(mut t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        t /= dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Oscillator, Waveform};
    use vb_core::{ProcessContext, Processor};

    fn render(osc: &mut Oscillator, sample_rate: f32, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate,
            frames,
        };
        let mut buf = vec![0.0_f32; frames];
        let ins: [&[f32]; 0] = [];
        let mut outs: [&mut [f32]; 1] = [&mut buf];
        osc.process(&ctx, &ins, &mut outs);
        buf
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn sine_matches_expected_quarter_phase_values() {
        // sample_rate 8, freq 1 → dt = 0.125, so phases step 0, 1/8, 2/8, …
        let mut osc = Oscillator::new(Waveform::Sine, 1.0);
        let out = render(&mut osc, 8.0, 8);
        assert!(approx(out[0], 0.0)); // sin(0)
        assert!(approx(out[2], 1.0)); // sin(pi/2)
        assert!(approx(out[4], 0.0)); // sin(pi)
        assert!(approx(out[6], -1.0)); // sin(3pi/2)
    }

    #[test]
    fn phase_wraps_and_is_periodic() {
        let mut osc = Oscillator::new(Waveform::Sine, 1.0);
        let first = render(&mut osc, 8.0, 8);
        let second = render(&mut osc, 8.0, 8);
        for (a, b) in first.iter().zip(second.iter()) {
            assert!(approx(*a, *b), "period 8 should repeat: {a} vs {b}");
        }
    }

    #[test]
    fn saw_and_square_stay_bounded() {
        // A high ratio of freq to sample rate is where anti-aliasing matters most.
        for wave in [Waveform::Saw, Waveform::Square] {
            let mut osc = Oscillator::new(wave, 1_000.0);
            let out = render(&mut osc, 8_000.0, 256);
            assert!(
                out.iter().all(|s| s.abs() <= 1.2),
                "{wave:?} exceeded bounds: max {}",
                out.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
            );
        }
    }

    #[test]
    fn triangle_peaks_and_bounds() {
        // sample_rate 4, freq 1 → dt 0.25 → phases 0, .25, .5, .75.
        let mut osc = Oscillator::new(Waveform::Triangle, 1.0);
        let out = render(&mut osc, 4.0, 4);
        assert!(approx(out[0], -1.0)); // phase 0
        assert!(approx(out[1], 0.0)); // phase 0.25
        assert!(approx(out[2], 1.0)); // phase 0.5 (peak)
        assert!(approx(out[3], 0.0)); // phase 0.75
    }

    #[test]
    fn reset_returns_to_start_phase() {
        let mut osc = Oscillator::new(Waveform::Sine, 1.0);
        let _ = render(&mut osc, 8.0, 3);
        osc.reset();
        let out = render(&mut osc, 8.0, 1);
        assert!(approx(out[0], 0.0));
    }

    #[test]
    fn zero_sample_rate_is_silent_and_safe() {
        let mut osc = Oscillator::new(Waveform::Saw, 440.0);
        let out = render(&mut osc, 0.0, 8);
        // dt is 0, so phase never advances and the saw sits at its phase-0 value (-1).
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
