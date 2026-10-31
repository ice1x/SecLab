//! Phase-vocoder pitch shifter: transposes the input up or down **without** changing its speed.
//!
//! Unlike [`crate::FreqShifter`] (which adds a constant number of Hz to every partial, turning
//! a harmonic tone inharmonic) and [`crate::RingMod`] (sum/difference sidebands), a pitch shifter
//! *multiplies* every partial's frequency by a ratio, so harmonics stay integer multiples and the
//! sound keeps its character an octave up or down. `semitones` sets the interval (±12 = ±1
//! octave); `mix` blends the dry signal (delayed to match the vocoder latency) with the shifted
//! one.
//!
//! The method is Stephan M. Bernsee's `smbPitchShift` routine (the canonical STFT phase-vocoder
//! pitch shifter): overlap-added windowed frames are FFT'd, each bin's *true* frequency is
//! recovered from the phase advance between frames, the magnitudes and true frequencies are moved
//! to bins scaled by the pitch ratio, the per-bin phase is re-accumulated, and an inverse FFT and
//! overlap-add resynthesise the signal. Anti-aliasing is automatic: bins that would scale past
//! Nyquist are dropped.
//!
//! Primary source: Stephan M. Bernsee, "Pitch Shifting Using the Fourier Transform"
//! (<http://blogs.zynaptiq.com/bernsee/pitch-shifting-using-the-ft/>) and the accompanying
//! `smbPitchShift.cpp` (Wide Open License). Exact setup constants used here:
//! `stepSize = fftFrameSize / osamp`, `freqPerBin = sampleRate / fftFrameSize`,
//! `expct = 2π·stepSize / fftFrameSize`.

use crate::stft::{Stft, FFT_SIZE, HALF, LATENCY, OSAMP, STEP_SIZE};
use seclab_core::{ProcessContext, Processor};

/// A phase-vocoder pitch shifter.
///
/// Input 0 is audio. Output 0 is the pitch-shifted (and dry-mixed) signal, delayed by
/// [`PitchShifter::latency_frames`].
pub struct PitchShifter {
    semitones: f32,
    mix: f32,

    /// Shared STFT overlap-add engine (window, FIFOs, FFT workspace, dry delay).
    stft: Stft,
    /// Last analysis phase per bin, for the inter-frame phase difference.
    last_phase: Vec<f32>,
    /// Accumulated synthesis phase per bin.
    sum_phase: Vec<f32>,
    ana_magn: Vec<f32>,
    ana_freq: Vec<f32>,
    syn_magn: Vec<f32>,
    syn_freq: Vec<f32>,
}

impl PitchShifter {
    /// Creates a pitch shifter transposing by `semitones` (clamped to ±12, i.e. ±1 octave) with
    /// a dry/wet `mix` (0 = dry, 1 = fully shifted).
    #[must_use]
    pub fn new(semitones: f32, mix: f32) -> Self {
        Self {
            semitones: semitones.clamp(-12.0, 12.0),
            mix: mix.clamp(0.0, 1.0),
            stft: Stft::new(),
            last_phase: vec![0.0; HALF + 1],
            sum_phase: vec![0.0; HALF + 1],
            ana_magn: vec![0.0; HALF + 1],
            ana_freq: vec![0.0; HALF + 1],
            syn_magn: vec![0.0; HALF + 1],
            syn_freq: vec![0.0; HALF + 1],
        }
    }

    /// The pitch ratio: `2^(semitones/12)`, clamped to Bernsee's valid `0.5 … 2.0`.
    fn ratio(&self) -> f32 {
        2.0_f32.powf(self.semitones / 12.0).clamp(0.5, 2.0)
    }

    /// The spectral transform of one analysed frame: recover each bin's true frequency, move the
    /// magnitudes/frequencies to bins scaled by `ratio`, and re-accumulate the synthesis phase.
    /// Reads and writes the analysed half-spectrum in `self.stft.fft_work`.
    fn transform(&mut self, ratio: f32, freq_per_bin: f32) {
        let expct = core::f32::consts::TAU * STEP_SIZE as f32 / FFT_SIZE as f32;

        // Analysis: recover each bin's magnitude and true frequency (Hz).
        for k in 0..=HALF {
            let re = self.stft.fft_work[2 * k];
            let im = self.stft.fft_work[2 * k + 1];
            let magn = 2.0 * (re * re + im * im).sqrt();
            let phase = im.atan2(re);

            // Phase advance this hop, minus the advance expected for a bin-centred sinusoid.
            let mut tmp = phase - self.last_phase[k];
            self.last_phase[k] = phase;
            tmp -= k as f32 * expct;

            // Map the phase difference into the ±π interval (Bernsee's parity trick).
            let mut qpd = (tmp / core::f32::consts::PI) as i64;
            if qpd >= 0 {
                qpd += qpd & 1;
            } else {
                qpd -= qpd & 1;
            }
            tmp -= core::f32::consts::PI * qpd as f32;

            // Deviation (in bins) from the bin centre, then the true frequency in Hz.
            tmp = OSAMP as f32 * tmp / core::f32::consts::TAU;
            self.ana_magn[k] = magn;
            self.ana_freq[k] = k as f32 * freq_per_bin + tmp * freq_per_bin;
        }

        // Processing: move each partial to the bin scaled by the pitch ratio.
        for k in 0..=HALF {
            self.syn_magn[k] = 0.0;
            self.syn_freq[k] = 0.0;
        }
        for k in 0..=HALF {
            let index = (k as f32 * ratio) as usize;
            if index <= HALF {
                self.syn_magn[index] += self.ana_magn[k];
                self.syn_freq[index] = self.ana_freq[k] * ratio;
            }
        }

        // Synthesis: turn each bin's magnitude and true frequency back into an accumulated phase.
        for k in 0..=HALF {
            let magn = self.syn_magn[k];
            let mut tmp = self.syn_freq[k];
            tmp -= k as f32 * freq_per_bin; // deviation from bin centre, in Hz
            tmp /= freq_per_bin; // → bins
            tmp = core::f32::consts::TAU * tmp / OSAMP as f32; // → phase advance per hop
            tmp += k as f32 * expct; // add the bin-centre advance back in
            self.sum_phase[k] += tmp;
            let phase = self.sum_phase[k];
            self.stft.fft_work[2 * k] = magn * phase.cos();
            self.stft.fft_work[2 * k + 1] = magn * phase.sin();
        }
    }
}

impl Processor for PitchShifter {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        if ctx.sample_rate <= 0.0 {
            for (o, x) in outputs[0].iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        let ratio = self.ratio();
        let freq_per_bin = ctx.sample_rate / FFT_SIZE as f32;
        let mix = self.mix;

        for (i, o) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let dry = self.stft.dry_delay(x);
            let (wet, frame_ready) = self.stft.push(x);
            if frame_ready {
                self.stft.forward();
                self.transform(ratio, freq_per_bin);
                self.stft.synthesize();
            }
            *o = mix * wet + (1.0 - mix) * dry;
        }
    }

    fn reset(&mut self) {
        self.stft.reset();
        self.last_phase.iter_mut().for_each(|s| *s = 0.0);
        self.sum_phase.iter_mut().for_each(|s| *s = 0.0);
    }

    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&s) = values.first() {
            self.semitones = s.clamp(-12.0, 12.0);
        }
        if let Some(&m) = values.get(1) {
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

    fn run(p: &mut PitchShifter, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
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

    /// Single-frequency magnitude via a Goertzel-style projection, normalised so a unit-amplitude
    /// sine reads ≈ 0.5. Robust for pure tones where autocorrelation ties between octaves.
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
        let p = PitchShifter::new(0.0, 1.0);
        assert_eq!(p.latency_frames(), FFT_SIZE - STEP_SIZE);
    }

    #[test]
    fn silence_in_silence_out() {
        let out = run(&mut PitchShifter::new(12.0, 1.0), &vec![0.0; 8_192]);
        assert!(out.iter().all(|s| s.abs() < 1e-6), "silence stays silent");
    }

    #[test]
    fn unity_ratio_reconstructs_the_tone_at_unity_gain() {
        // At 0 semitones every partial maps to its own bin: the output is the same tone, at the
        // same amplitude (the Hann² overlap-add normalisation is correct), with the energy still
        // concentrated at the input frequency and little elsewhere.
        let input = tone(440.0, 16_384);
        let out = run(&mut PitchShifter::new(0.0, 1.0), &input);
        let tail = &out[8_192..];
        let m440 = mag(tail, 440.0);
        assert!(
            (m440 - 0.5).abs() < 0.08,
            "unity reconstructs 440 Hz at unit gain (≈0.5), got {m440}"
        );
        assert!(mag(tail, 220.0) < 0.05, "no spurious sub-octave");
        assert!(mag(tail, 880.0) < 0.05, "no spurious upper octave");
    }

    #[test]
    fn an_octave_up_doubles_the_frequency() {
        // 300 Hz shifted +12 semitones concentrates at 600 Hz, not at the original 300 Hz.
        let input = tone(300.0, 32_768);
        let out = run(&mut PitchShifter::new(12.0, 1.0), &input);
        let tail = &out[16_384..];
        let (m300, m600) = (mag(tail, 300.0), mag(tail, 600.0));
        assert!(
            m600 > 8.0 * m300,
            "energy moves to 600 Hz: m600 {m600}, m300 {m300}"
        );
        assert!(
            m600 > 0.2,
            "the shifted tone stays clearly audible, got {m600}"
        );
    }

    #[test]
    fn an_octave_down_halves_the_frequency() {
        let input = tone(600.0, 32_768);
        let out = run(&mut PitchShifter::new(-12.0, 1.0), &input);
        let tail = &out[16_384..];
        let (m300, m600) = (mag(tail, 300.0), mag(tail, 600.0));
        assert!(
            m300 > 8.0 * m600,
            "energy moves to 300 Hz: m300 {m300}, m600 {m600}"
        );
        assert!(
            m300 > 0.2,
            "the shifted tone stays clearly audible, got {m300}"
        );
    }

    #[test]
    fn a_dry_mix_leaves_the_pitch_untouched() {
        // mix = 0 is the delayed dry signal regardless of the semitone setting: pitch unchanged.
        let input = tone(440.0, 16_384);
        let out = run(&mut PitchShifter::new(12.0, 0.0), &input);
        let tail = &out[8_192..];
        assert!(
            mag(tail, 440.0) > 8.0 * mag(tail, 880.0),
            "a dry mix keeps the original 440 Hz"
        );
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let input = tone(220.0, 16_384);
        let out = run(&mut PitchShifter::new(7.0, 1.0), &input);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 4.0),
            "a shifted tone stays finite and bounded"
        );
    }

    #[test]
    fn set_params_updates_in_place() {
        let mut p = PitchShifter::new(0.0, 1.0);
        assert!(p.set_params(&[12.0, 0.5]));
        assert!((p.semitones - 12.0).abs() < 1e-6);
        assert!((p.mix - 0.5).abs() < 1e-6);
        // Out-of-range values are clamped.
        assert!(p.set_params(&[99.0, 9.0]));
        assert!((p.semitones - 12.0).abs() < 1e-6);
        assert!((p.mix - 1.0).abs() < 1e-6);
    }

    #[test]
    fn semitone_setting_is_clamped_to_one_octave() {
        assert!((PitchShifter::new(24.0, 1.0).ratio() - 2.0).abs() < 1e-6);
        assert!((PitchShifter::new(-24.0, 1.0).ratio() - 0.5).abs() < 1e-6);
        assert!((PitchShifter::new(0.0, 1.0).ratio() - 1.0).abs() < 1e-6);
    }
}
