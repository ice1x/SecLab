//! Voltage-controlled filter: a biquad whose cutoff a control signal sweeps.
//!
//! This is the filter side of modulation (the [`crate::Vca`] is the amplitude side), and
//! the node the README's MVP scenario assumes — "an LFO on cutoff". It wraps a
//! [`crate::Biquad`] and, once per block, retunes its cutoff from a control input:
//! `cutoff = clamp(base_hz * 2^(amount_oct * cv))`. The audio then flows through the filter
//! as usual. With the control input unconnected (silence) the cutoff sits at `base_hz`, so
//! it behaves like a plain biquad until an LFO or envelope is wired in.
//!
//! Modulation is **exponential (per-octave)**, matching the [`crate::Ladder`] and
//! [`crate::Vco`] `amount_oct` laws: a unit of control shifts the cutoff by `amount_oct`
//! octaves (bi-polar — negative reverses the sweep), so a fixed amount moves the cutoff by a
//! fixed number of octaves and sounds even across the range, the way a real analog filter's
//! ≈1 V/octave modulation does — rather than the low-end-collapsing linear-in-Hz map.

use vb_core::{ProcessContext, Processor};

use crate::filter::{Biquad, FilterKind};

/// Lowest and highest cutoff the modulation is clamped to (Hz).
const MIN_CUTOFF_HZ: f32 = 20.0;
const MAX_CUTOFF_HZ: f32 = 20_000.0;

/// A control-swept biquad: `cutoff = clamp(base_hz * 2^(amount_oct * cv))`, updated per block.
#[derive(Debug, Clone)]
pub struct Vcf {
    filter: Biquad,
    /// Cutoff (Hz) when the control signal is zero.
    base_hz: f32,
    /// Octaves of cutoff swing per unit of control signal (bi-polar: negative reverses the
    /// sweep).
    amount_oct: f32,
}

impl Vcf {
    /// Creates a VCF of `kind` centred at `base_hz` with resonance `q`, sweeping
    /// `amount_oct` octaves per unit of control input.
    #[must_use]
    pub fn new(kind: FilterKind, base_hz: f32, q: f32, amount_oct: f32) -> Self {
        Self {
            filter: Biquad::new(kind, base_hz, q),
            base_hz,
            amount_oct,
        }
    }

    /// The cutoff the filter is currently tuned to (Hz).
    #[must_use]
    pub fn cutoff_hz(&self) -> f32 {
        self.filter.cutoff_hz()
    }
}

impl Processor for Vcf {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        // Control-rate: retune from the control input's value at the block start.
        let cv = inputs
            .get(1)
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        // Exponential (per-octave) modulation: a unit of control shifts the cutoff by
        // `amount_oct` octaves, so the sweep sounds even across the range (see module docs).
        let cutoff =
            (self.base_hz * 2.0_f32.powf(self.amount_oct * cv)).clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
        self.filter.set_cutoff(cutoff);

        // Delegate the actual filtering: the biquad reads inputs[0] → outputs[0].
        let audio = inputs.first().copied().unwrap_or(&[]);
        self.filter.process(ctx, &[audio], outputs);
    }

    fn reset(&mut self) {
        self.filter.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// A bright test signal: alternating +1/-1 (maximum high-frequency content).
    fn bright(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect()
    }

    fn run(vcf: &mut Vcf, audio: &[f32], cv: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: audio.len(),
        };
        let mut out = vec![0.0; audio.len()];
        {
            let mut slice = [&mut out[..]];
            vcf.process(&ctx, &[audio, cv], &mut slice);
        }
        out
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn with_zero_control_it_sits_at_the_base_cutoff() {
        let mut vcf = Vcf::new(FilterKind::LowPass, 500.0, 0.707, 3.0);
        let audio = bright(256);
        run(&mut vcf, &audio, &[0.0; 256]);
        // cv = 0 → 2^0 = 1 → cutoff = base, amount ignored.
        assert!((vcf.cutoff_hz() - 500.0).abs() < 1e-3);
    }

    #[test]
    fn amount_oct_sweeps_the_cutoff_exponentially() {
        // A unit of control moves the cutoff by `amount_oct` octaves (base · 2^(amount·cv)),
        // and a negative amount reverses the sweep — matching the ladder / vco amount_oct law.
        let cutoff = |amount: f32, cv: f32| {
            let mut vcf = Vcf::new(FilterKind::LowPass, 1_000.0, 0.707, amount);
            run(&mut vcf, &bright(8), &[cv; 8]);
            vcf.cutoff_hz()
        };
        assert!((cutoff(1.0, 1.0) - 2_000.0).abs() < 1.0, "one octave up");
        assert!((cutoff(1.0, -1.0) - 500.0).abs() < 1.0, "one octave down");
        assert!((cutoff(2.0, 1.0) - 4_000.0).abs() < 1.0, "two octaves up");
        assert!(
            (cutoff(-1.0, 1.0) - 500.0).abs() < 1.0,
            "negative amount reverses the sweep"
        );
    }

    #[test]
    fn zero_amount_ignores_the_control_signal() {
        // amount 0 → 2^0 = 1 regardless of cv, so the cutoff rests at base_hz.
        let mut vcf = Vcf::new(FilterKind::LowPass, 800.0, 0.707, 0.0);
        run(&mut vcf, &bright(8), &[1.0; 8]);
        assert!((vcf.cutoff_hz() - 800.0).abs() < 1e-3);
    }

    #[test]
    fn a_control_signal_opens_the_lowpass_and_passes_more_treble() {
        let audio = bright(2_048);

        // Closed: low cutoff, no modulation → heavy attenuation of the bright signal.
        let mut closed = Vcf::new(FilterKind::LowPass, 300.0, 0.707, 3.0);
        let closed_out = run(&mut closed, &audio, &[0.0; 2_048]);

        // Open: the same filter with cv = 1 pushes the cutoff up 3 octaves → more passes.
        let mut open = Vcf::new(FilterKind::LowPass, 300.0, 0.707, 3.0);
        let open_out = run(&mut open, &audio, &[1.0; 2_048]);

        assert!(
            open.cutoff_hz() > closed.cutoff_hz(),
            "cv raised the cutoff"
        );
        assert!(
            rms(&open_out) > rms(&closed_out) * 1.5,
            "opening the filter passes noticeably more energy: {} vs {}",
            rms(&open_out),
            rms(&closed_out)
        );
    }

    #[test]
    fn the_cutoff_is_clamped_to_the_audio_range() {
        let mut vcf = Vcf::new(FilterKind::LowPass, 1_000.0, 0.707, 20.0);
        run(&mut vcf, &bright(64), &[1.0; 64]); // base · 2^20 hugely over Nyquist
        assert!(
            (vcf.cutoff_hz() - MAX_CUTOFF_HZ).abs() < 1e-3,
            "clamped to max"
        );

        let mut vcf = Vcf::new(FilterKind::LowPass, 1_000.0, 0.707, -20.0);
        run(&mut vcf, &bright(64), &[1.0; 64]); // driven far below 20 Hz
        assert!(
            (vcf.cutoff_hz() - MIN_CUTOFF_HZ).abs() < 1e-3,
            "clamped to min"
        );
    }

    #[test]
    fn a_missing_control_input_reads_as_zero() {
        let mut vcf = Vcf::new(FilterKind::LowPass, 700.0, 0.707, 3.0);
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 3,
        };
        let audio = [1.0, -1.0, 1.0];
        let mut out = [0.0_f32; 3];
        {
            let mut slice = [&mut out[..]];
            vcf.process(&ctx, &[&audio], &mut slice); // no cv channel
        }
        assert!(
            (vcf.cutoff_hz() - 700.0).abs() < 1e-3,
            "cutoff stays at base"
        );
    }
}
