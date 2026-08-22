//! Moog-style four-pole (24 dB/octave) resonant low-pass ladder filter.
//!
//! The characteristic Moog voice: four cascaded one-pole low-pass stages with a global
//! resonance feedback path, giving a steeper roll-off than the two-pole [`crate::Svf`] /
//! [`crate::Biquad`] and the singing resonance a ladder is known for. Like the
//! [`crate::Vcf`] it is control-swept — a control input retunes the cutoff once per block
//! (`cutoff = clamp(base_hz + amount_hz * cv)`) — so it drops straight into a synth voice
//! with an envelope or LFO on the cutoff. With the control input unconnected it sits at
//! `base_hz`.
//!
//! The DSP is a linear four-pole ladder built from topology-preserving-transform (TPT)
//! one-pole low-pass stages — the same `tan`-prewarped integrator the [`crate::Svf`] uses,
//! so the cutoff tunes accurately across the audio range — closed with a **zero-delay
//! feedback** (ZDF) resonance loop (Zavalishin's formulation): the instantaneous feedback
//! is solved algebraically each sample, so it is unconditionally stable for resonance in
//! `0..1` (feedback `k = 4·resonance`, self-oscillation at the top) instead of blowing up
//! the way a one-sample-delayed loop would. Allocation-free and cheap enough for the RT
//! path.

use vb_core::{ProcessContext, Processor};

/// Lowest and highest cutoff the modulation is clamped to (Hz).
const MIN_CUTOFF_HZ: f32 = 20.0;
const MAX_CUTOFF_HZ: f32 = 20_000.0;

/// A control-swept four-pole ladder low-pass: `cutoff = clamp(base_hz + amount_hz * cv)`,
/// retuned per block.
#[derive(Debug, Clone)]
pub struct Ladder {
    /// Cutoff (Hz) when the control signal is zero.
    base_hz: f32,
    /// Hz of cutoff swing per unit of control signal (may be negative to invert).
    amount_hz: f32,
    /// Resonance / feedback amount, `0..1` (higher approaches self-oscillation).
    resonance: f32,
    /// Cutoff the filter is currently tuned to (Hz).
    cutoff_hz: f32,
    /// TPT one-pole integrator states (one per stage).
    z: [f32; 4],
}

impl Ladder {
    /// Creates a ladder centred at `base_hz` with `resonance` (`0..1`), sweeping
    /// `amount_hz` Hz of cutoff per unit of control input.
    #[must_use]
    pub fn new(base_hz: f32, resonance: f32, amount_hz: f32) -> Self {
        Self {
            base_hz,
            amount_hz,
            resonance: resonance.clamp(0.0, 1.0),
            cutoff_hz: base_hz,
            z: [0.0; 4],
        }
    }

    /// The cutoff the filter is currently tuned to (Hz).
    #[must_use]
    pub fn cutoff_hz(&self) -> f32 {
        self.cutoff_hz
    }
}

impl Processor for Ladder {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(out) = outputs.first_mut() else {
            return;
        };
        let audio = inputs.first().copied().unwrap_or(&[]);

        // No sample rate yet: pass the audio through unchanged.
        if ctx.sample_rate <= 0.0 {
            for i in 0..ctx.frames {
                out[i] = audio.get(i).copied().unwrap_or(0.0);
            }
            return;
        }

        // Control-rate: retune from the control input's value at the block start.
        let cv = inputs
            .get(1)
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        self.cutoff_hz = (self.base_hz + self.amount_hz * cv).clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);

        // TPT one-pole coefficient. The `tan` prewarp tunes accurately, and the ZDF
        // ladder's resonant peak lands right on `cutoff_hz`, so no fudge factor is needed.
        let cutoff = self.cutoff_hz.clamp(1.0, ctx.sample_rate * 0.49);
        let g = (std::f32::consts::PI * cutoff / ctx.sample_rate).tan();
        let big_g = g / (1.0 + g); // one-pole TPT feedforward gain
        let one_minus_g = 1.0 - big_g; // = 1/(1+g); each stage's state contribution scale
        let g2 = big_g * big_g;
        let g3 = g2 * big_g;
        let g4 = g3 * big_g;
        // Resonance feedback: k = 4 drives self-oscillation; scale to that range.
        let k = self.resonance * 4.0;
        let denom = 1.0 + k * g4;

        for i in 0..ctx.frames {
            let x = audio.get(i).copied().unwrap_or(0.0);

            // Each stage's zero-input output contribution S_i = z_i / (1 + g).
            let s0 = self.z[0] * one_minus_g;
            let s1 = self.z[1] * one_minus_g;
            let s2 = self.z[2] * one_minus_g;
            let s3 = self.z[3] * one_minus_g;
            // Output of the four-stage cascade for an input of 0 (the state feedthrough).
            let sigma = g3 * s0 + g2 * s1 + big_g * s2 + s3;

            // Solve the instantaneous feedback loop u = x - k·y4 with y4 = G⁴·u + Σ.
            let y4 = (g4 * x + sigma) / denom;
            let u = x - k * y4;

            // Propagate u through the four TPT one-poles, updating their states.
            let mut sig = u;
            for stage in &mut self.z {
                let v = (sig - *stage) * big_g;
                let y = v + *stage;
                *stage = y + v;
                sig = y;
            }

            out[i] = sig;
        }
    }

    fn reset(&mut self) {
        self.z = [0.0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Runs a sine of `freq` through `filter` and returns the tail RMS (after the
    /// transient settles). A unit sine has RMS ≈ 0.707 when passed unchanged.
    fn response(filter: &mut Ladder, freq: f32) -> f32 {
        let frames = 8_192;
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let input: Vec<f32> = (0..frames)
            .map(|i| (std::f32::consts::TAU * freq * i as f32 / SR).sin())
            .collect();
        let mut out = vec![0.0; frames];
        {
            let mut slice = [&mut out[..]];
            filter.process(&ctx, &[&input], &mut slice);
        }
        let tail = &out[frames / 2..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn passes_low_frequencies() {
        // A 100 Hz sine (RMS ≈ 0.707) well below a 4 kHz cutoff comes through largely
        // intact — the slight loss is the ladder's characteristic feedback bass-droop.
        let pass = response(&mut Ladder::new(4_000.0, 0.1, 0.0), 100.0);
        assert!(pass > 0.45, "100 Hz through a 4 kHz ladder LP: {pass}");
    }

    #[test]
    fn rejects_high_frequencies() {
        // An 8 kHz sine through a 400 Hz low-pass is far up the 24 dB/oct slope.
        let reject = response(&mut Ladder::new(400.0, 0.1, 0.0), 8_000.0);
        assert!(reject < 0.02, "8 kHz through a 400 Hz ladder LP: {reject}");
    }

    #[test]
    fn rolls_off_steeper_than_a_two_pole_filter() {
        // Two octaves above cutoff a 24 dB/oct ladder should be dramatically lower than
        // one octave above — far steeper than a 12 dB/oct two-pole would fall.
        let one_octave = response(&mut Ladder::new(500.0, 0.1, 0.0), 1_000.0);
        let two_octaves = response(&mut Ladder::new(500.0, 0.1, 0.0), 2_000.0);
        assert!(
            two_octaves < one_octave * 0.35,
            "steep roll-off: {one_octave} at +1oct vs {two_octaves} at +2oct"
        );
    }

    #[test]
    fn resonance_emphasises_the_cutoff() {
        // A sine at the cutoff passes with more energy as resonance rises.
        let calm = response(&mut Ladder::new(1_000.0, 0.1, 0.0), 1_000.0);
        let resonant = response(&mut Ladder::new(1_000.0, 0.9, 0.0), 1_000.0);
        assert!(
            resonant > calm * 2.0,
            "resonance lifts the cutoff band: {calm} -> {resonant}"
        );
    }

    #[test]
    fn a_control_signal_opens_the_filter() {
        let bright: Vec<f32> = (0..2_048)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let run = |cv: f32| {
            let mut f = Ladder::new(300.0, 0.2, 15_000.0);
            let ctx = ProcessContext {
                sample_rate: SR,
                frames: bright.len(),
            };
            let mut out = vec![0.0; bright.len()];
            {
                let mut slice = [&mut out[..]];
                f.process(&ctx, &[&bright, &vec![cv; bright.len()]], &mut slice);
            }
            (
                f.cutoff_hz(),
                (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt(),
            )
        };
        let (closed_hz, closed_rms) = run(0.0);
        let (open_hz, open_rms) = run(1.0);
        assert!(open_hz > closed_hz, "cv raised the cutoff");
        assert!(
            open_rms > closed_rms * 1.5,
            "opening the ladder passes more treble: {closed_rms} -> {open_rms}"
        );
    }

    #[test]
    fn cutoff_is_clamped_to_the_audio_range() {
        // Drive the cutoff via the control input (base + amount·cv), since the sweep
        // only applies when a control signal is present.
        let sweep = |amount: f32, cv: f32| {
            let mut f = Ladder::new(1_000.0, 0.2, amount);
            let ctx = ProcessContext {
                sample_rate: SR,
                frames: 8,
            };
            let audio = [0.0_f32; 8];
            let mut out = [0.0_f32; 8];
            {
                let mut slice = [&mut out[..]];
                f.process(&ctx, &[&audio, &[cv; 8]], &mut slice);
            }
            f.cutoff_hz()
        };
        assert!(
            (sweep(1_000_000.0, 1.0) - MAX_CUTOFF_HZ).abs() < 1e-3,
            "clamped to max"
        );
        assert!(
            (sweep(-1_000_000.0, 1.0) - MIN_CUTOFF_HZ).abs() < 1e-3,
            "clamped to min"
        );
    }

    #[test]
    fn stays_stable_at_maximum_resonance() {
        let mut f = Ladder::new(2_000.0, 1.0, 0.0);
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 256,
        };
        let input = vec![1.0_f32; 256];
        let mut out = vec![0.0; 256];
        for _ in 0..400 {
            let mut slice = [&mut out[..]];
            f.process(&ctx, &[&input], &mut slice);
        }
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 100.0),
            "ladder stays bounded at full resonance"
        );
    }

    #[test]
    fn zero_sample_rate_passes_through() {
        let mut f = Ladder::new(500.0, 0.5, 0.0);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 4,
        };
        let input = [0.2, -0.4, 0.6, -0.8];
        let mut out = [0.0_f32; 4];
        {
            let mut slice = [&mut out[..]];
            f.process(&ctx, &[&input], &mut slice);
        }
        assert_eq!(out, input);
    }
}
