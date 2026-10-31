//! Voltage-controlled oscillator: an oscillator whose pitch a control signal sweeps.
//!
//! This is the pitch side of modulation — the third leg alongside the [`crate::Vca`]
//! (amplitude) and [`crate::Vcf`] (cutoff). It wraps an [`crate::Oscillator`] and, once
//! per block, retunes its frequency from a control input using an **exponential** (1 V /
//! octave) law: `freq = base_hz * 2^(amount_oct * cv)`. Exponential mapping is what makes
//! pitch modulation musical — a constant control offset shifts by a constant interval, so
//! an LFO gives even vibrato and an envelope a clean pitch sweep. With the control input
//! unconnected (silence) the frequency sits at `base_hz`, so it behaves like a plain
//! oscillator until an LFO or envelope is wired in.

use seclab_core::{ProcessContext, Processor};

use crate::oscillator::{Oscillator, Waveform};

/// Lowest and highest frequency the modulation is clamped to (Hz). Zero is allowed at the
/// bottom (silent DC) so a deep downward sweep is safe; the top stays under a typical
/// Nyquist.
const MIN_FREQ_HZ: f32 = 0.0;
const MAX_FREQ_HZ: f32 = 20_000.0;

/// A control-swept oscillator: `freq = clamp(base_hz * 2^(amount_oct * cv))`, updated per
/// block.
#[derive(Debug, Clone)]
pub struct Vco {
    osc: Oscillator,
    /// Frequency (Hz) when the control signal is zero.
    base_hz: f32,
    /// Octaves of pitch swing per unit of control signal (may be negative to invert).
    amount_oct: f32,
}

impl Vco {
    /// Creates a VCO of `waveform` centred at `base_hz`, sweeping `amount_oct` octaves per
    /// unit of control input.
    #[must_use]
    pub fn new(waveform: Waveform, base_hz: f32, amount_oct: f32) -> Self {
        Self {
            osc: Oscillator::new(waveform, base_hz),
            base_hz,
            amount_oct,
        }
    }

    /// The frequency the oscillator is currently tuned to (Hz).
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.osc.freq_hz()
    }
}

impl Processor for Vco {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        // Control-rate: retune from the control input's value at the block start.
        let cv = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let freq =
            (self.base_hz * 2.0_f32.powf(self.amount_oct * cv)).clamp(MIN_FREQ_HZ, MAX_FREQ_HZ);
        self.osc.set_freq_hz(freq);

        // Delegate generation: the oscillator ignores inputs and writes outputs[0].
        self.osc.process(ctx, &[], outputs);
    }

    fn reset(&mut self) {
        self.osc.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(vco: &mut Vco, cv: &[f32], frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        {
            let mut slice = [&mut out[..]];
            vco.process(&ctx, &[cv], &mut slice);
        }
        out
    }

    #[test]
    fn with_zero_control_it_sits_at_the_base_frequency() {
        let mut vco = Vco::new(Waveform::Sine, 440.0, 1.0);
        run(&mut vco, &[0.0; 64], 64);
        assert!((vco.freq_hz() - 440.0).abs() < 1e-3, "cv = 0 → base");
    }

    #[test]
    fn a_unit_control_with_one_octave_amount_doubles_the_frequency() {
        let mut vco = Vco::new(Waveform::Sine, 220.0, 1.0);
        run(&mut vco, &[1.0; 64], 64);
        // 220 * 2^(1*1) = 440.
        assert!((vco.freq_hz() - 440.0).abs() < 1e-2, "one octave up");

        // A negative control drops an octave: 220 * 2^-1 = 110.
        let mut down = Vco::new(Waveform::Sine, 220.0, 1.0);
        run(&mut down, &[-1.0; 64], 64);
        assert!((down.freq_hz() - 110.0).abs() < 1e-2, "one octave down");
    }

    #[test]
    fn the_mapping_is_exponential_not_linear() {
        // Equal control steps give equal *ratios*, not equal Hz deltas.
        let mut a = Vco::new(Waveform::Sine, 100.0, 2.0);
        run(&mut a, &[0.5; 8], 8); // 100 * 2^(1.0) = 200
        let mut b = Vco::new(Waveform::Sine, 100.0, 2.0);
        run(&mut b, &[1.0; 8], 8); // 100 * 2^(2.0) = 400
        assert!((a.freq_hz() - 200.0).abs() < 1e-2);
        assert!((b.freq_hz() - 400.0).abs() < 1e-2);
        // Ratio b/a is 2, though the control only doubled — exponential, not linear.
        assert!((b.freq_hz() / a.freq_hz() - 2.0).abs() < 1e-3);
    }

    #[test]
    fn a_higher_control_produces_a_higher_pitch_more_zero_crossings() {
        // At a high pitch the sine crosses zero more often over a fixed block.
        fn zero_crossings(x: &[f32]) -> usize {
            x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
        }
        let mut low = Vco::new(Waveform::Sine, 200.0, 2.0);
        let low_out = run(&mut low, &[0.0; 4_800], 4_800); // 200 Hz
        let mut high = Vco::new(Waveform::Sine, 200.0, 2.0);
        let high_out = run(&mut high, &[1.0; 4_800], 4_800); // 800 Hz
        assert!(
            zero_crossings(&high_out) > zero_crossings(&low_out) * 2,
            "raising the pitch CV audibly raises the frequency: {} vs {}",
            zero_crossings(&high_out),
            zero_crossings(&low_out)
        );
    }

    #[test]
    fn the_frequency_is_clamped_to_the_audio_range() {
        // A huge upward push is capped at the max.
        let mut up = Vco::new(Waveform::Sine, 1_000.0, 8.0);
        run(&mut up, &[8.0; 8], 8); // 1000 * 2^64, enormous
        assert!((up.freq_hz() - MAX_FREQ_HZ).abs() < 1e-3, "clamped to max");
    }

    #[test]
    fn a_missing_control_input_reads_as_zero() {
        let mut vco = Vco::new(Waveform::Sine, 330.0, 2.0);
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 3,
        };
        let mut out = [0.0_f32; 3];
        {
            let mut slice = [&mut out[..]];
            vco.process(&ctx, &[], &mut slice); // no cv channel
        }
        assert!(
            (vco.freq_hz() - 330.0).abs() < 1e-3,
            "frequency stays at base"
        );
    }

    #[test]
    fn the_output_stays_finite() {
        let mut vco = Vco::new(Waveform::Saw, 440.0, 3.0);
        let out = run(&mut vco, &[0.5; 256], 256);
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
