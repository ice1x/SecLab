//! FM operator: a sine oscillator whose phase is modulated by an input signal.
//!
//! The building block of FM synthesis. A classic operator is a pure sine carrier plus a
//! *phase-modulation* input: `out = sin(2π·(phase + index·mod))`. Feeding one operator's
//! output into another's `mod` input, at musically-related frequency ratios, is what
//! produces FM's characteristic metallic and bell-like timbres — far more harmonically
//! rich than the sine it is built from.
//!
//! Like [`crate::PitchOsc`] it tracks a `pitch_hz` control input so it plays from the
//! keyboard, and runs its carrier at `ratio × pitch` — the integer/fractional ratio that
//! places the carrier in a harmonic (or inharmonic) relationship with the note. With
//! `index = 0`, or nothing patched into `mod`, it is simply a sine at that frequency.

use seclab_core::{ProcessContext, Processor};

/// Lowest and highest carrier frequency (Hz).
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// A phase-modulated sine operator.
#[derive(Debug, Clone)]
pub struct FmOp {
    /// Resting carrier pitch when no note is held (Hz), before the ratio is applied.
    base_hz: f32,
    /// Carrier frequency as a multiple of the tracked pitch (harmonic ratio).
    ratio: f32,
    /// Modulation depth: how strongly the `mod` input displaces the phase.
    index: f32,
    /// Carrier phase in `[0, 1)`.
    phase: f32,
    /// Carrier frequency resolved from the pitch input at the block start (Hz).
    freq_hz: f32,
}

impl FmOp {
    /// Creates an operator resting at `base_hz`, running its carrier at `ratio × pitch`,
    /// with modulation depth `index`.
    #[must_use]
    pub fn new(base_hz: f32, ratio: f32, index: f32) -> Self {
        Self {
            base_hz,
            ratio,
            index,
            phase: 0.0,
            freq_hz: (base_hz * ratio).clamp(MIN_HZ, MAX_HZ),
        }
    }

    /// The carrier frequency currently in effect (Hz).
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.freq_hz
    }
}

impl Processor for FmOp {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        // Input 0 = pitch (control, read once per block); input 1 = modulator (audio-rate).
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let tracked = if pitch >= MIN_HZ { pitch } else { self.base_hz };
        self.freq_hz = (tracked * self.ratio).clamp(MIN_HZ, MAX_HZ);

        let modulator = inputs.get(1).copied().unwrap_or(&[]);
        let dt = if ctx.sample_rate > 0.0 {
            self.freq_hz / ctx.sample_rate
        } else {
            0.0
        };

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let m = modulator.get(i).copied().unwrap_or(0.0);
            *out = (core::f32::consts::TAU * (self.phase + self.index * m)).sin();
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

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Renders a block with an optional constant pitch and an optional modulator buffer.
    fn render(
        op: &mut FmOp,
        pitch: Option<f32>,
        modulator: Option<&[f32]>,
        frames: usize,
    ) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        let pitch_buf = pitch.map(|p| vec![p; frames]);
        let mut ins: Vec<&[f32]> = Vec::new();
        if let Some(p) = &pitch_buf {
            ins.push(p.as_slice());
            if let Some(m) = modulator {
                ins.push(m);
            }
        }
        {
            let mut slice = [&mut out[..]];
            op.process(&ctx, &ins, &mut slice);
        }
        out
    }

    fn zero_crossings(xs: &[f32]) -> usize {
        xs.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    }
    fn rms(xs: &[f32]) -> f32 {
        (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt()
    }

    #[test]
    fn with_no_modulator_it_is_a_pure_sine_at_ratio_times_pitch() {
        // ratio 1, pitch 480 Hz over 4800 samples ≈ 48 cycles ≈ 96 crossings.
        let mut op = FmOp::new(100.0, 1.0, 4.0);
        let out = render(&mut op, Some(480.0), None, 4_800);
        assert!((op.freq_hz() - 480.0).abs() < 1e-3);
        let c = zero_crossings(&out);
        assert!((94..=98).contains(&c), "expected ~96 crossings, got {c}");
        assert!(
            out.iter().all(|s| s.abs() <= 1.0 + 1e-6),
            "a bare sine stays within ±1"
        );
    }

    #[test]
    fn the_ratio_multiplies_the_carrier_frequency() {
        // ratio 2 → carrier at twice the pitch → about twice the crossings.
        let base = zero_crossings(&render(
            &mut FmOp::new(100.0, 1.0, 0.0),
            Some(300.0),
            None,
            4_800,
        ));
        let doubled = zero_crossings(&render(
            &mut FmOp::new(100.0, 2.0, 0.0),
            Some(300.0),
            None,
            4_800,
        ));
        assert!(
            doubled >= base * 2 - 4 && doubled <= base * 2 + 4,
            "ratio 2 ≈ 2× crossings: {base} vs {doubled}"
        );
    }

    #[test]
    fn modulation_enriches_the_signal_beyond_a_pure_sine() {
        // A pure sine has exactly two zero crossings per cycle; phase modulation by a
        // higher-frequency operator adds sidebands, changing the waveform. We drive `mod`
        // with a fast sine and require the modulated output to differ from the unmodulated.
        let frames = 4_800;
        let carrier_pitch = 200.0;
        let clean = render(
            &mut FmOp::new(100.0, 1.0, 0.0),
            Some(carrier_pitch),
            None,
            frames,
        );
        // Modulator: a 1 kHz sine (well above the carrier) at unit amplitude.
        let modulator: Vec<f32> = (0..frames)
            .map(|i| (core::f32::consts::TAU * 1_000.0 * i as f32 / SR).sin())
            .collect();
        let mut op = FmOp::new(100.0, 1.0, 3.0);
        let dirty = render(&mut op, Some(carrier_pitch), Some(&modulator), frames);
        assert!(
            dirty.iter().all(|s| s.is_finite() && s.abs() <= 1.0 + 1e-6),
            "stays bounded"
        );
        let diff = clean
            .iter()
            .zip(&dirty)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff > 0.1,
            "modulation must change the waveform (max diff {diff})"
        );
        assert!(
            zero_crossings(&dirty) > zero_crossings(&clean),
            "FM sidebands add crossings"
        );
        assert!(rms(&dirty) > 0.0);
    }

    #[test]
    fn falls_back_to_base_when_no_note_is_held() {
        let mut op = FmOp::new(330.0, 1.0, 0.0);
        render(&mut op, Some(0.0), None, 64);
        assert!((op.freq_hz() - 330.0).abs() < 1e-3);
        // And with no pitch input connected at all.
        let mut op2 = FmOp::new(261.63, 1.0, 0.0);
        render(&mut op2, None, None, 64);
        assert!((op2.freq_hz() - 261.63).abs() < 1e-2);
    }

    #[test]
    fn carrier_is_clamped_to_the_audio_range() {
        let mut op = FmOp::new(440.0, 8.0, 0.0);
        render(&mut op, Some(10_000.0), None, 64); // 8 × 10 kHz → clamped
        assert!((op.freq_hz() - MAX_HZ).abs() < 1e-3);
    }

    #[test]
    fn reset_returns_to_start_phase() {
        let mut op = FmOp::new(100.0, 1.0, 0.0);
        render(&mut op, Some(480.0), None, 3);
        op.reset();
        let out = render(&mut op, Some(480.0), None, 1);
        assert!(out[0].abs() < 1e-5, "phase 0 → sin(0) = 0");
    }

    #[test]
    fn zero_sample_rate_is_safe() {
        let mut op = FmOp::new(440.0, 1.0, 2.0);
        let ctx = ProcessContext {
            sample_rate: 0.0,
            frames: 8,
        };
        let mut out = [0.0_f32; 8];
        {
            let mut slice = [&mut out[..]];
            op.process(&ctx, &[&[440.0; 8], &[0.5; 8]], &mut slice);
        }
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
