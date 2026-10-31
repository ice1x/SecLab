//! State-variable filter: one input, lowpass/bandpass/highpass outputs at once.
//!
//! Andrew Simper's trapezoidal-integration SVF (the Cytomic formulation) — the
//! modern standard for musical filters: stable under cutoff modulation and cheap
//! enough for the RT path. All three responses fall out of one topology, so the node
//! exposes them as three simultaneous audio outputs; patch whichever you need.
//!
//! A zero sample rate (no context yet) passes the input through to the lowpass
//! output, matching the biquad's behaviour.

use seclab_core::{ProcessContext, Processor};

/// Simper-style state-variable filter.
pub struct Svf {
    cutoff_hz: f32,
    q: f32,
    /// Integrator states.
    ic1: f32,
    ic2: f32,
    /// Coefficients, recomputed when the sample rate changes.
    g: f32,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    coeff_sr: f32,
}

impl Svf {
    /// Creates a filter with the given cutoff and resonance (`q`).
    #[must_use]
    pub fn new(cutoff_hz: f32, q: f32) -> Self {
        Self {
            cutoff_hz,
            q: q.max(1e-3),
            ic1: 0.0,
            ic2: 0.0,
            g: 0.0,
            k: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            coeff_sr: 0.0,
        }
    }

    /// Recomputes coefficients for `sample_rate` if it changed.
    fn update_coeffs(&mut self, sample_rate: f32) {
        if (sample_rate - self.coeff_sr).abs() < f32::EPSILON {
            return;
        }
        self.coeff_sr = sample_rate;
        // Clamp cutoff below Nyquist to keep tan() finite.
        let cutoff = self.cutoff_hz.clamp(1.0, sample_rate * 0.49);
        self.g = (std::f32::consts::PI * cutoff / sample_rate).tan();
        self.k = 1.0 / self.q;
        self.a1 = 1.0 / self.g.mul_add(self.g + self.k, 1.0);
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
    }
}

impl Processor for Svf {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(input) = inputs.first() else {
            return;
        };
        // No sample rate yet: pass through on the lowpass output.
        if ctx.sample_rate <= 0.0 {
            if let Some(low) = outputs.first_mut() {
                low[..ctx.frames].copy_from_slice(&input[..ctx.frames]);
            }
            return;
        }
        self.update_coeffs(ctx.sample_rate);

        let [low_out, rest @ ..] = outputs else {
            return;
        };
        let (mut band_out, mut high_out) = match rest {
            [band, high, ..] => (Some(band), Some(high)),
            [band] => (Some(band), None),
            [] => (None, None),
        };

        for i in 0..ctx.frames {
            let v0 = input[i];
            let v3 = v0 - self.ic2;
            let v1 = self.a1.mul_add(self.ic1, self.a2 * v3);
            let v2 = self
                .ic2
                .mul_add(1.0, self.a2.mul_add(self.ic1, self.a3 * v3));
            self.ic1 = 2.0f32.mul_add(v1, -self.ic1);
            self.ic2 = 2.0f32.mul_add(v2, -self.ic2);

            low_out[i] = v2;
            if let Some(band) = band_out.as_deref_mut() {
                band[i] = v1;
            }
            if let Some(high) = high_out.as_deref_mut() {
                high[i] = self.k.mul_add(-v1, v0 - v2);
            }
        }
    }

    fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Runs a sine of `freq` through the filter and returns (low, band, high) RMS
    /// over the tail (after the transient settles).
    fn response(filter: &mut Svf, freq: f32) -> (f32, f32, f32) {
        let frames = 8_192;
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let input: Vec<f32> = (0..frames)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin())
            .collect();
        let (mut low, mut band, mut high) =
            (vec![0.0; frames], vec![0.0; frames], vec![0.0; frames]);
        filter.process(&ctx, &[&input], &mut [&mut low, &mut band, &mut high]);
        let rms = |xs: &[f32]| {
            let tail = &xs[frames / 2..];
            (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
        };
        (rms(&low), rms(&band), rms(&high))
    }

    #[test]
    fn lowpass_passes_low_and_rejects_high() {
        let (low_pass, _, _) = response(&mut Svf::new(500.0, 0.707), 100.0);
        let (low_reject, _, _) = response(&mut Svf::new(500.0, 0.707), 8_000.0);
        assert!(low_pass > 0.6, "100 Hz through a 500 Hz LP: {low_pass}");
        assert!(low_reject < 0.05, "8 kHz through a 500 Hz LP: {low_reject}");
    }

    #[test]
    fn highpass_mirrors_the_lowpass() {
        let (_, _, high_reject) = response(&mut Svf::new(500.0, 0.707), 100.0);
        let (_, _, high_pass) = response(&mut Svf::new(500.0, 0.707), 8_000.0);
        assert!(high_pass > 0.6, "8 kHz through a 500 Hz HP: {high_pass}");
        assert!(
            high_reject < 0.1,
            "100 Hz through a 500 Hz HP: {high_reject}"
        );
    }

    #[test]
    fn bandpass_peaks_at_the_cutoff() {
        let (_, at_cutoff, _) = response(&mut Svf::new(1_000.0, 2.0), 1_000.0);
        let (_, below, _) = response(&mut Svf::new(1_000.0, 2.0), 100.0);
        let (_, above, _) = response(&mut Svf::new(1_000.0, 2.0), 10_000.0);
        assert!(at_cutoff > 0.5, "on-cutoff bandpass: {at_cutoff}");
        assert!(below < at_cutoff * 0.3 && above < at_cutoff * 0.3);
    }

    #[test]
    fn outputs_reconstruct_the_input() {
        // SVF identity: v0 = high + k*band + low, per sample.
        let mut f = Svf::new(700.0, 1.3);
        let frames = 512;
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let input: Vec<f32> = (0..frames)
            .map(|i| (std::f32::consts::TAU * 330.0 * i as f32 / SR).sin() * 0.8)
            .collect();
        let (mut low, mut band, mut high) =
            (vec![0.0; frames], vec![0.0; frames], vec![0.0; frames]);
        f.process(&ctx, &[&input], &mut [&mut low, &mut band, &mut high]);
        let k = 1.0 / 1.3;
        for i in 0..frames {
            let rebuilt = high[i] + k * band[i] + low[i];
            assert!(
                (rebuilt - input[i]).abs() < 1e-4,
                "sample {i}: {rebuilt} vs {}",
                input[i]
            );
        }
    }

    #[test]
    fn stays_stable_at_high_resonance() {
        let mut f = Svf::new(2_000.0, 20.0);
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 256,
        };
        let input = vec![1.0_f32; 256]; // step excitation, repeated
        let mut low = vec![0.0; 256];
        for _ in 0..200 {
            f.process(&ctx, &[&input], &mut [&mut low]);
        }
        assert!(low.iter().all(|s| s.is_finite() && s.abs() < 100.0));
    }

    #[test]
    fn reset_clears_the_state() {
        let mut f = Svf::new(500.0, 0.707);
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 64,
        };
        let mut impulse = vec![0.0_f32; 64];
        impulse[0] = 1.0;
        let mut first = vec![0.0; 64];
        f.process(&ctx, &[&impulse], &mut [&mut first]);

        let mut second = vec![0.0; 64];
        f.reset();
        f.process(&ctx, &[&impulse], &mut [&mut second]);
        assert_eq!(first, second);
    }

    #[test]
    fn zero_sample_rate_passes_through() {
        let mut f = Svf::new(500.0, 0.707);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 8,
        };
        let input = [0.5_f32; 8];
        let mut low = [0.0_f32; 8];
        f.process(&ctx, &[&input], &mut [&mut low]);
        assert_eq!(low, input);
    }
}
