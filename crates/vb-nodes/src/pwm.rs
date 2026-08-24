//! Pulse-width oscillator: a band-limited pulse with an adjustable duty cycle.
//!
//! A square wave is a pulse at 50% duty; sweeping the duty away from that — the pulse
//! *width* — thins the waveform from full square toward a narrow spike, moving its harmonic
//! balance and its character from hollow/reedy to bright/nasal. At 50% it is a square (odd
//! harmonics only). Both edges are PolyBLEP-corrected so the pulse stays alias-free, and it
//! tracks a `pitch_hz` control input like [`crate::PitchOsc`].

use vb_core::{ProcessContext, Processor};

/// Lowest and highest oscillator frequency (Hz).
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// A pitch-tracking, band-limited pulse-width oscillator.
///
/// Input 0 is a `pitch_hz` control (falls back to `base_hz`). Output 0 is a pulse of duty
/// `width` in `[0, 1]`. Params: `base_hz`, `width`.
#[derive(Debug, Clone)]
pub struct Pwm {
    base_hz: f32,
    width: f32,
    freq_hz: f32,
    phase: f32,
}

impl Pwm {
    /// Creates a pulse oscillator resting at `base_hz` with duty `width` (clamped 0.01..=0.99).
    #[must_use]
    pub fn new(base_hz: f32, width: f32) -> Self {
        Self {
            base_hz,
            width: width.clamp(0.01, 0.99),
            freq_hz: base_hz,
            phase: 0.0,
        }
    }

    /// The frequency currently in effect (Hz).
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.freq_hz
    }
}

impl Processor for Pwm {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let tracked = if pitch >= MIN_HZ { pitch } else { self.base_hz };
        self.freq_hz = tracked.clamp(MIN_HZ, MAX_HZ);
        let dt = self.freq_hz / sr;

        for out in outputs[0].iter_mut().take(ctx.frames) {
            // Naive pulse plus PolyBLEP residuals at both discontinuities (phase 0 and width).
            let naive = if self.phase < self.width { 1.0 } else { -1.0 };
            let edge = (self.phase - self.width + 1.0) % 1.0;
            *out = naive + poly_blep(self.phase, dt) - poly_blep(edge, dt);
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

/// PolyBLEP residual for a discontinuity at phase `t` with per-sample increment `dt`.
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
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(osc: &mut Pwm, pitch: Option<f32>, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        let pitch_buf = pitch.map(|p| vec![p; frames]);
        let ins: Vec<&[f32]> = pitch_buf
            .as_ref()
            .map(|b| vec![b.as_slice()])
            .unwrap_or_default();
        {
            let mut slice = [&mut out[..]];
            osc.process(&ctx, &ins, &mut slice);
        }
        out
    }

    fn zero_crossings(xs: &[f32]) -> usize {
        xs.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    }

    #[test]
    fn a_50_percent_pulse_is_a_symmetric_square() {
        // width 0.5 → equal time high and low → mean ≈ 0.
        let out = render(&mut Pwm::new(220.0, 0.5), Some(220.0), 4_800);
        let mean: f32 = out.iter().sum::<f32>() / out.len() as f32;
        assert!(mean.abs() < 0.05, "square is balanced, mean {mean}");
        assert!(
            out.iter().all(|s| s.abs() <= 1.2),
            "band-limited pulse stays bounded"
        );
    }

    #[test]
    fn a_narrow_pulse_is_asymmetric() {
        // width 0.1 → high only 10% of the time → strongly negative mean.
        let out = render(&mut Pwm::new(220.0, 0.1), Some(220.0), 4_800);
        let mean: f32 = out.iter().sum::<f32>() / out.len() as f32;
        assert!(
            mean < -0.5,
            "narrow pulse spends most time low, mean {mean}"
        );
    }

    #[test]
    fn it_tracks_the_pitch_input() {
        let low = zero_crossings(&render(&mut Pwm::new(100.0, 0.5), Some(200.0), 4_800));
        let high = zero_crossings(&render(&mut Pwm::new(100.0, 0.5), Some(800.0), 4_800));
        assert!(high > low * 2, "4× pitch → more crossings: {low} vs {high}");
        let mut osc = Pwm::new(330.0, 0.5);
        render(&mut osc, Some(0.0), 64);
        assert!((osc.freq_hz() - 330.0).abs() < 1e-3, "falls back to base");
    }

    #[test]
    fn width_is_clamped_and_zero_sr_is_safe() {
        assert!((Pwm::new(220.0, 5.0).width - 0.99).abs() < 1e-6);
        let mut osc = Pwm::new(220.0, 0.5);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 8,
        };
        let mut out = [0.0_f32; 8];
        {
            let mut slice = [&mut out[..]];
            osc.process(&ctx, &[&[220.0; 8]], &mut slice);
        }
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
