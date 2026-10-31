//! Biquad filter node (RBJ cookbook): low-pass, high-pass, band-pass and notch.

use crate::biquad::{BiquadCoeffs, BiquadState};
use seclab_core::{ProcessContext, Processor};

/// The response shape of a [`Biquad`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    /// Passes frequencies below the cutoff.
    LowPass,
    /// Passes frequencies above the cutoff.
    HighPass,
    /// Passes a band around the cutoff.
    BandPass,
    /// Rejects a band around the cutoff.
    Notch,
}

/// A second-order (biquad) filter with one audio input and one audio output.
///
/// Coefficients follow the RBJ audio EQ cookbook and are recomputed per block from the
/// context's sample rate, so sample-rate changes are handled automatically. The cutoff
/// is clamped below Nyquist to stay stable. The shared [`BiquadState`] runs Direct Form I.
#[derive(Debug, Clone)]
pub struct Biquad {
    kind: FilterKind,
    cutoff_hz: f32,
    q: f32,
    state: BiquadState,
}

impl Biquad {
    /// Creates a filter of `kind` at `cutoff_hz` with resonance `q`.
    #[must_use]
    pub fn new(kind: FilterKind, cutoff_hz: f32, q: f32) -> Self {
        Self {
            kind,
            cutoff_hz,
            q,
            state: BiquadState::default(),
        }
    }

    /// The filter's response kind.
    #[must_use]
    pub fn kind(&self) -> FilterKind {
        self.kind
    }

    /// Retunes the cutoff (Hz) without disturbing the filter state — the hook a control
    /// input drives to sweep the filter (see [`crate::Vcf`]). Coefficients are recomputed
    /// on the next processed block.
    pub fn set_cutoff(&mut self, cutoff_hz: f32) {
        self.cutoff_hz = cutoff_hz;
    }

    /// The current cutoff in Hz.
    #[must_use]
    pub fn cutoff_hz(&self) -> f32 {
        self.cutoff_hz
    }

    fn coeffs(&self, sample_rate: f32) -> BiquadCoeffs {
        let f0 = self.cutoff_hz.clamp(1.0, 0.49 * sample_rate);
        let w0 = core::f32::consts::TAU * (f0 / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * self.q.max(1e-4));

        let (b0, b1, b2, a0, a1, a2) = match self.kind {
            FilterKind::LowPass => {
                let k = 1.0 - cos;
                (k * 0.5, k, k * 0.5, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterKind::HighPass => {
                let k = 1.0 + cos;
                (k * 0.5, -k, k * 0.5, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterKind::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        };

        BiquadCoeffs::normalized(b0, b1, b2, a0, a1, a2)
    }
}

impl Processor for Biquad {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        if ctx.sample_rate <= 0.0 {
            for (o, x) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        let c = self.coeffs(ctx.sample_rate);
        for (o, x) in outputs[0].iter_mut().zip(inputs[0].iter()).take(ctx.frames) {
            *o = self.state.step(&c, *x);
        }
    }

    fn reset(&mut self) {
        self.state.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::{Biquad, FilterKind};
    use seclab_core::{ProcessContext, Processor};

    /// Feeds `input` through the filter at 48 kHz and returns the output.
    fn run(filter: &mut Biquad, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        let ins: [&[f32]; 1] = [input];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        filter.process(&ctx, &ins, &mut outs);
        out
    }

    #[test]
    fn lowpass_passes_dc() {
        let mut f = Biquad::new(FilterKind::LowPass, 1_000.0, 0.707);
        let out = run(&mut f, &vec![1.0; 2048]);
        assert!(
            (out[2047] - 1.0).abs() < 0.02,
            "LP DC gain ~1, got {}",
            out[2047]
        );
    }

    #[test]
    fn highpass_blocks_dc() {
        let mut f = Biquad::new(FilterKind::HighPass, 1_000.0, 0.707);
        let out = run(&mut f, &vec![1.0; 2048]);
        assert!(out[2047].abs() < 0.02, "HP DC gain ~0, got {}", out[2047]);
    }

    #[test]
    fn bandpass_blocks_dc() {
        let mut f = Biquad::new(FilterKind::BandPass, 1_000.0, 0.707);
        let out = run(&mut f, &vec![1.0; 2048]);
        assert!(out[2047].abs() < 0.02, "BP DC gain ~0, got {}", out[2047]);
    }

    #[test]
    fn notch_passes_dc() {
        let mut f = Biquad::new(FilterKind::Notch, 1_000.0, 0.707);
        let out = run(&mut f, &vec![1.0; 2048]);
        assert!(
            (out[2047] - 1.0).abs() < 0.02,
            "notch DC gain ~1, got {}",
            out[2047]
        );
    }

    #[test]
    fn output_is_finite_and_stable() {
        let mut f = Biquad::new(FilterKind::LowPass, 5_000.0, 4.0);
        // Alternating full-scale input is a stress test for stability.
        let input: Vec<f32> = (0..1024)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let out = run(&mut f, &input);
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn reset_clears_state() {
        let mut f = Biquad::new(FilterKind::LowPass, 1_000.0, 0.707);
        let _ = run(&mut f, &vec![1.0; 512]);
        f.reset();
        // With cleared state the first output equals b0 * first input.
        let out = run(&mut f, &[1.0]);
        assert!(
            out[0] > 0.0 && out[0] < 0.01,
            "post-reset first sample = b0, got {}",
            out[0]
        );
    }

    #[test]
    fn zero_sample_rate_passes_through() {
        let mut f = Biquad::new(FilterKind::LowPass, 1_000.0, 0.707);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 3,
        };
        let input = [0.1_f32, 0.2, 0.3];
        let mut out = vec![0.0_f32; 3];
        let ins: [&[f32]; 1] = [&input];
        let mut outs: [&mut [f32]; 1] = [&mut out];
        f.process(&ctx, &ins, &mut outs);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }
}
