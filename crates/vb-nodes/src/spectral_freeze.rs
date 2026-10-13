//! Spectral freeze: captures the current short-time spectrum and sustains it indefinitely.
//!
//! An STFT effect that, while its `freeze` control is low, resynthesises its input faithfully
//! (a near-passthrough), and while `freeze` is high holds the **last analysed magnitude
//! spectrum** and keeps advancing each bin's phase by its captured per-hop increment — so a
//! single instant of sound becomes an endless, evolving pad. Feeding silence while frozen leaves
//! the drone ringing; releasing `freeze` lets it track the input again.
//!
//! Input 0 is audio; input 1 is the `freeze` control (`>= 0.5` freezes). Output 0 is the
//! resynthesised (and dry-mixed) signal, delayed by [`SpectralFreeze::latency_frames`].
//!
//! Mechanism: the same STFT overlap-add engine as [`crate::PitchShifter`] (Hann-windowed frames,
//! forward FFT, per-bin phase tracking, inverse FFT, overlap-add) on the shared [`crate::fft`]
//! radix-2 transform. Each hop the analysed magnitude and the raw inter-hop phase advance are
//! captured into `held_*` **unless** frozen; the synthesis phase always accumulates the held
//! advance, so unfrozen it telescopes back to the true phase (faithful reconstruction) and frozen
//! it keeps rotating the held magnitudes at their captured pitch. The 2π ambiguity in the raw
//! advance is irrelevant because only `e^{iφ}` is used.

use crate::fft::fft;
use vb_core::{ProcessContext, Processor};

/// FFT frame size (power of two).
const FFT_SIZE: usize = 1024;
/// Overlap factor (75 % overlap).
const OSAMP: usize = 4;
/// Hop between frames.
const STEP_SIZE: usize = FFT_SIZE / OSAMP;
/// Positive-frequency bins (DC … Nyquist).
const HALF: usize = FFT_SIZE / 2;
/// Latency = one frame minus a hop (also the FIFO prime).
const LATENCY: usize = FFT_SIZE - STEP_SIZE;

/// A spectral-freeze effect.
pub struct SpectralFreeze {
    mix: f32,
    frozen: bool,

    window: Vec<f32>,
    in_fifo: Vec<f32>,
    out_fifo: Vec<f32>,
    fft_work: Vec<f32>,
    out_accum: Vec<f32>,
    last_phase: Vec<f32>,
    sum_phase: Vec<f32>,
    held_magn: Vec<f32>,
    held_adv: Vec<f32>,
    rover: usize,

    dry: Vec<f32>,
    dry_idx: usize,
}

impl SpectralFreeze {
    /// Creates a spectral freeze with dry/wet `mix` (0 = dry input, 1 = fully resynthesised).
    #[must_use]
    pub fn new(mix: f32) -> Self {
        let window = (0..FFT_SIZE)
            .map(|k| 0.5 - 0.5 * (core::f32::consts::TAU * k as f32 / FFT_SIZE as f32).cos())
            .collect();
        Self {
            mix: mix.clamp(0.0, 1.0),
            frozen: false,
            window,
            in_fifo: vec![0.0; FFT_SIZE],
            out_fifo: vec![0.0; FFT_SIZE],
            fft_work: vec![0.0; 2 * FFT_SIZE],
            out_accum: vec![0.0; 2 * FFT_SIZE],
            last_phase: vec![0.0; HALF + 1],
            sum_phase: vec![0.0; HALF + 1],
            held_magn: vec![0.0; HALF + 1],
            held_adv: vec![0.0; HALF + 1],
            rover: LATENCY,
            dry: vec![0.0; LATENCY],
            dry_idx: 0,
        }
    }

    fn process_frame(&mut self) {
        // Window the frame into the complex workspace, forward FFT.
        for k in 0..FFT_SIZE {
            self.fft_work[2 * k] = self.in_fifo[k] * self.window[k];
            self.fft_work[2 * k + 1] = 0.0;
        }
        fft(&mut self.fft_work, FFT_SIZE, -1.0);

        for k in 0..=HALF {
            let re = self.fft_work[2 * k];
            let im = self.fft_work[2 * k + 1];
            let magn = 2.0 * (re * re + im * im).sqrt();
            let phase = im.atan2(re);
            // Raw inter-hop advance; last_phase is tracked even when frozen so unfreezing resumes.
            let adv = phase - self.last_phase[k];
            self.last_phase[k] = phase;
            if !self.frozen {
                self.held_magn[k] = magn;
                self.held_adv[k] = adv;
            }
            self.sum_phase[k] += self.held_adv[k];
            let p = self.sum_phase[k];
            self.fft_work[2 * k] = self.held_magn[k] * p.cos();
            self.fft_work[2 * k + 1] = self.held_magn[k] * p.sin();
        }
        for k in (HALF + 1)..FFT_SIZE {
            self.fft_work[2 * k] = 0.0;
            self.fft_work[2 * k + 1] = 0.0;
        }
        fft(&mut self.fft_work, FFT_SIZE, 1.0);

        // Hann² constant-overlap-add gain (×2/3) on top of Bernsee's 2/(HALF·osamp).
        let norm = 2.0 / (HALF as f32 * OSAMP as f32) * (2.0 / 3.0);
        for k in 0..FFT_SIZE {
            self.out_accum[k] += self.window[k] * self.fft_work[2 * k] * norm;
        }
        self.out_fifo[..STEP_SIZE].copy_from_slice(&self.out_accum[..STEP_SIZE]);
        self.out_accum
            .copy_within(STEP_SIZE..STEP_SIZE + FFT_SIZE, 0);
        self.in_fifo.copy_within(STEP_SIZE..FFT_SIZE, 0);
    }
}

impl Processor for SpectralFreeze {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        let freeze = inputs.get(1).copied().unwrap_or(&[]);
        if ctx.sample_rate <= 0.0 {
            for (o, x) in outputs[0].iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        let mix = self.mix;
        for (i, o) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            self.frozen = freeze.get(i).copied().unwrap_or(0.0) >= 0.5;

            let dry = self.dry[self.dry_idx];
            self.dry[self.dry_idx] = x;
            self.dry_idx = (self.dry_idx + 1) % LATENCY;

            self.in_fifo[self.rover] = x;
            let wet = self.out_fifo[self.rover - LATENCY];
            self.rover += 1;
            if self.rover >= FFT_SIZE {
                self.rover = LATENCY;
                self.process_frame();
            }

            *o = mix * wet + (1.0 - mix) * dry;
        }
    }

    fn reset(&mut self) {
        self.in_fifo.iter_mut().for_each(|s| *s = 0.0);
        self.out_fifo.iter_mut().for_each(|s| *s = 0.0);
        self.out_accum.iter_mut().for_each(|s| *s = 0.0);
        self.last_phase.iter_mut().for_each(|s| *s = 0.0);
        self.sum_phase.iter_mut().for_each(|s| *s = 0.0);
        self.held_magn.iter_mut().for_each(|s| *s = 0.0);
        self.held_adv.iter_mut().for_each(|s| *s = 0.0);
        self.dry.iter_mut().for_each(|s| *s = 0.0);
        self.frozen = false;
        self.rover = LATENCY;
        self.dry_idx = 0;
    }

    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&m) = values.first() {
            self.mix = m.clamp(0.0, 1.0);
        }
        true
    }

    fn latency_frames(&self) -> usize {
        LATENCY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(p: &mut SpectralFreeze, input: &[f32], freeze: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 2] = [input, freeze];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            p.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * hz * i as f32 / SR).sin())
            .collect()
    }

    fn mag(xs: &[f32], hz: f32) -> f32 {
        let w = core::f32::consts::TAU * hz / SR;
        let (mut re, mut im) = (0.0_f32, 0.0_f32);
        for (n, &x) in xs.iter().enumerate() {
            re += x * (w * n as f32).cos();
            im -= x * (w * n as f32).sin();
        }
        (re * re + im * im).sqrt() / xs.len() as f32
    }

    #[test]
    fn latency_is_the_fft_minus_one_hop() {
        assert_eq!(
            SpectralFreeze::new(1.0).latency_frames(),
            FFT_SIZE - STEP_SIZE
        );
    }

    #[test]
    fn unfrozen_reconstructs_the_input_at_unit_gain() {
        // freeze held low: the held spectrum tracks the input every hop, so the output is the
        // input faithfully reconstructed at unit gain.
        let input = tone(440.0, 16_384);
        let out = run(&mut SpectralFreeze::new(1.0), &input, &vec![0.0; 16_384]);
        let tail = &out[8_192..];
        let m = mag(tail, 440.0);
        assert!(
            (m - 0.5).abs() < 0.08,
            "unfrozen reconstructs 440 Hz (≈0.5), got {m}"
        );
        assert!(mag(tail, 880.0) < 0.05, "no spurious octave");
    }

    #[test]
    fn a_frozen_spectrum_sustains_after_the_input_goes_silent() {
        // Play a tone with freeze low, flip freeze high, then feed SILENCE: the captured 440 Hz
        // spectrum must keep ringing.
        let n1 = 16_384;
        let n2 = 16_384;
        let mut input = tone(440.0, n1);
        input.extend(std::iter::repeat_n(0.0, n2));
        let mut freeze = vec![0.0; n1];
        freeze.extend(std::iter::repeat_n(1.0, n2));

        let out = run(&mut SpectralFreeze::new(1.0), &input, &freeze);
        // Look well into the frozen-and-silent region.
        let tail = &out[n1 + 4_096..];
        let m = mag(tail, 440.0);
        assert!(
            m > 0.1,
            "the frozen spectrum sustains the 440 Hz tone through silence, got {m}"
        );
    }

    #[test]
    fn a_freeze_ignores_the_later_input() {
        // Freeze on a 440 Hz tone, then feed a 1 kHz tone while still frozen: the output keeps the
        // frozen 440 Hz, not the new 1 kHz.
        let n1 = 16_384;
        let n2 = 16_384;
        let mut input = tone(440.0, n1);
        input.extend(tone(1_000.0, n2));
        let mut freeze = vec![0.0; n1];
        freeze.extend(std::iter::repeat_n(1.0, n2));

        let out = run(&mut SpectralFreeze::new(1.0), &input, &freeze);
        let tail = &out[n1 + 4_096..];
        assert!(
            mag(tail, 440.0) > 4.0 * mag(tail, 1_000.0),
            "the freeze holds 440 Hz and ignores the later 1 kHz input"
        );
    }

    #[test]
    fn silence_unfrozen_is_silent() {
        let out = run(
            &mut SpectralFreeze::new(1.0),
            &vec![0.0; 8_192],
            &vec![0.0; 8_192],
        );
        assert!(
            out.iter().all(|s| s.abs() < 1e-6),
            "silence in → silence out"
        );
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let n = 16_384;
        let mut freeze = vec![0.0; n / 2];
        freeze.extend(std::iter::repeat_n(1.0, n / 2));
        let out = run(&mut SpectralFreeze::new(1.0), &tone(330.0, n), &freeze);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 8.0),
            "frozen output stays finite and bounded"
        );
    }

    #[test]
    fn a_dry_mix_passes_the_input_through() {
        let input = tone(440.0, 4_096);
        let out = run(&mut SpectralFreeze::new(0.0), &input, &vec![1.0; 4_096]);
        let lat = LATENCY;
        // mix = 0 is the input delayed by the latency, verbatim.
        for i in lat..4_096 {
            assert!(
                (out[i] - input[i - lat]).abs() < 1e-6,
                "dry mix is the delayed input"
            );
        }
    }

    #[test]
    fn set_params_updates_mix_and_clamps() {
        let mut f = SpectralFreeze::new(1.0);
        assert!(f.set_params(&[0.25]));
        assert!((f.mix - 0.25).abs() < 1e-6);
        assert!(f.set_params(&[9.0]));
        assert!((f.mix - 1.0).abs() < 1e-6);
    }
}
