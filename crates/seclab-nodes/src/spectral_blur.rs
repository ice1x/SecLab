//! Spectral blur: a one-pole temporal smoothing of the magnitude spectrum — the frequency-domain
//! equivalent of blurring an image along the time axis.
//!
//! Each short-time frame the analysed magnitude of every bin is passed through a one-pole
//! low-pass **across frames**: `M'[k] = a·M'[k] + (1-a)·|X[k]|`, where `a` (the `amount`) is the
//! per-hop retention. The phase is propagated by the phase vocoder (each bin's synthesis phase
//! accumulates its measured per-hop advance), exactly as [`crate::SpectralFreeze`] does, so the
//! smeared magnitudes keep ringing at the source's pitch. With `amount = 0` the smoothing is a
//! no-op and the engine is a faithful analysis/resynthesis round-trip; as `amount → 1` the
//! spectrum changes ever more slowly, so transients are smeared and a note blooms into a long
//! decaying tail after the input stops — a soft, reverb-like "spectral smear".
//!
//! Clean tails: when the input goes silent the measured phase advance is meaningless (the bin is
//! numerical noise), so the advance is only *updated* from frames whose magnitude is audible and
//! otherwise *held* at its last reliable value. The still-audible smoothed magnitude therefore
//! keeps rotating at the last real pitch instead of at garbage, so the decay is a clean tone.
//!
//! Input 0 is audio. Output 0 is the blurred (and dry-mixed) signal, delayed by
//! [`SpectralBlur::latency_frames`]. Built on the shared [`crate::stft`] overlap-add engine.

use crate::stft::{Stft, HALF, LATENCY};
use seclab_core::{ProcessContext, Processor};

/// Below this analysed magnitude a bin is treated as silent: its phase advance is held, not
/// updated, so a decaying tail keeps the last real pitch instead of rotating at numerical noise.
const ADV_EPS: f32 = 1e-2;

/// A spectral-blur effect (temporal magnitude smoothing + phase-vocoder resynthesis).
pub struct SpectralBlur {
    amount: f32,
    mix: f32,

    stft: Stft,
    last_phase: Vec<f32>,
    sum_phase: Vec<f32>,
    smoothed_magn: Vec<f32>,
    held_adv: Vec<f32>,
}

impl SpectralBlur {
    /// Creates a spectral blur with per-hop magnitude retention `amount` (0 = no blur, →1 = heavy
    /// smear / long tail) and dry/wet `mix`.
    #[must_use]
    pub fn new(amount: f32, mix: f32) -> Self {
        Self {
            amount: amount.clamp(0.0, 0.999),
            mix: mix.clamp(0.0, 1.0),
            stft: Stft::new(),
            last_phase: vec![0.0; HALF + 1],
            sum_phase: vec![0.0; HALF + 1],
            smoothed_magn: vec![0.0; HALF + 1],
            held_adv: vec![0.0; HALF + 1],
        }
    }

    /// One analysed frame: one-pole smooth each bin's magnitude across frames, propagate the phase
    /// by its (held-when-silent) per-hop advance, and write the smoothed magnitude at the
    /// accumulated phase. Reads and writes the analysed half-spectrum in `self.stft.fft_work`.
    fn transform(&mut self) {
        let a = self.amount;
        for k in 0..=HALF {
            let re = self.stft.fft_work[2 * k];
            let im = self.stft.fft_work[2 * k + 1];
            let magn = 2.0 * (re * re + im * im).sqrt();
            let phase = im.atan2(re);
            let adv = phase - self.last_phase[k];
            self.last_phase[k] = phase;

            self.smoothed_magn[k] = a * self.smoothed_magn[k] + (1.0 - a) * magn;
            if magn > ADV_EPS {
                self.held_adv[k] = adv;
            }
            self.sum_phase[k] += self.held_adv[k];
            let p = self.sum_phase[k];
            self.stft.fft_work[2 * k] = self.smoothed_magn[k] * p.cos();
            self.stft.fft_work[2 * k + 1] = self.smoothed_magn[k] * p.sin();
        }
    }
}

impl Processor for SpectralBlur {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        if ctx.sample_rate <= 0.0 {
            for (o, x) in outputs[0].iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        let mix = self.mix;
        for (i, o) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            let dry = self.stft.dry_delay(x);
            let (wet, frame_ready) = self.stft.push(x);
            if frame_ready {
                self.stft.forward();
                self.transform();
                self.stft.synthesize();
            }
            *o = mix * wet + (1.0 - mix) * dry;
        }
    }

    fn reset(&mut self) {
        self.stft.reset();
        self.last_phase.iter_mut().for_each(|s| *s = 0.0);
        self.sum_phase.iter_mut().for_each(|s| *s = 0.0);
        self.smoothed_magn.iter_mut().for_each(|s| *s = 0.0);
        self.held_adv.iter_mut().for_each(|s| *s = 0.0);
    }

    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&a) = values.first() {
            self.amount = a.clamp(0.0, 0.999);
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

    fn run(p: &mut SpectralBlur, input: &[f32]) -> Vec<f32> {
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

    fn mag(xs: &[f32], hz: f32) -> f32 {
        let w = core::f32::consts::TAU * hz / SR;
        let (mut re, mut im) = (0.0_f32, 0.0_f32);
        for (n, &x) in xs.iter().enumerate() {
            re += x * (w * n as f32).cos();
            im -= x * (w * n as f32).sin();
        }
        (re * re + im * im).sqrt() / xs.len() as f32
    }

    fn rms(xs: &[f32]) -> f32 {
        (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt()
    }

    #[test]
    fn latency_is_the_fft_minus_one_hop() {
        assert_eq!(SpectralBlur::new(0.5, 1.0).latency_frames(), LATENCY);
    }

    #[test]
    fn no_blur_reconstructs_the_input_at_unit_gain() {
        // amount = 0: the magnitude smoothing is a no-op, so the engine is a faithful
        // analysis/resynthesis round-trip of the 440 Hz tone.
        let input = tone(440.0, 16_384);
        let out = run(&mut SpectralBlur::new(0.0, 1.0), &input);
        let tail = &out[8_192..];
        let m = mag(tail, 440.0);
        assert!(
            (m - 0.5).abs() < 0.08,
            "no-blur reconstructs 440 Hz, got {m}"
        );
        assert!(mag(tail, 880.0) < 0.05, "no spurious octave");
    }

    #[test]
    fn blur_blooms_a_decaying_tail_after_the_input_stops() {
        // A tone burst then silence. With no blur the output stops (modulo latency) when the input
        // does; with heavy blur the smoothed magnitude decays slowly, leaving a clean 440 Hz tail
        // ringing well after the input went silent.
        let n1 = 16_384;
        let n2 = 16_384;
        let mut input = tone(440.0, n1);
        input.extend(std::iter::repeat_n(0.0, n2));

        let dry = run(&mut SpectralBlur::new(0.0, 1.0), &input);
        let wet = run(&mut SpectralBlur::new(0.98, 1.0), &input);

        // Well past the offset (+4096 clears the STFT latency), measure the ringing tail.
        let tail = n1 + 4_096;
        let dry_tail = rms(&dry[tail..]);
        let wet_tail = rms(&wet[tail..]);
        assert!(
            dry_tail < 0.02,
            "with no blur the tail is essentially gone: {dry_tail}"
        );
        assert!(
            wet_tail > 5.0 * dry_tail && wet_tail > 0.03,
            "blur blooms a tail that outlasts the input: dry {dry_tail}, wet {wet_tail}"
        );
        assert!(
            mag(&wet[tail..], 440.0) > 4.0 * mag(&wet[tail..], 1_000.0),
            "the bloomed tail is a clean 440 Hz tone, not noise"
        );
    }

    #[test]
    fn more_blur_means_a_longer_tail() {
        // The tail energy grows monotonically with the blur amount.
        let n1 = 16_384;
        let n2 = 16_384;
        let mut input = tone(440.0, n1);
        input.extend(std::iter::repeat_n(0.0, n2));
        let tail = n1 + 6_144;

        let mild = rms(&run(&mut SpectralBlur::new(0.9, 1.0), &input)[tail..]);
        let heavy = rms(&run(&mut SpectralBlur::new(0.99, 1.0), &input)[tail..]);
        assert!(
            heavy > mild,
            "a higher blur amount leaves a longer tail: mild {mild}, heavy {heavy}"
        );
    }

    #[test]
    fn silence_in_silence_out() {
        let out = run(&mut SpectralBlur::new(0.9, 1.0), &vec![0.0; 8_192]);
        assert!(
            out.iter().all(|s| s.abs() < 1e-6),
            "silence in → silence out"
        );
    }

    #[test]
    fn a_dry_mix_passes_the_input_through() {
        let input = tone(440.0, 4_096);
        let out = run(&mut SpectralBlur::new(0.9, 0.0), &input);
        for i in LATENCY..4_096 {
            assert!(
                (out[i] - input[i - LATENCY]).abs() < 1e-6,
                "dry mix is the delayed input"
            );
        }
    }

    #[test]
    fn output_stays_finite_and_bounded() {
        let out = run(&mut SpectralBlur::new(0.95, 1.0), &tone(220.0, 16_384));
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 8.0),
            "blurred output stays finite and bounded"
        );
    }

    #[test]
    fn set_params_updates_and_clamps() {
        let mut b = SpectralBlur::new(0.5, 1.0);
        assert!(b.set_params(&[0.8, 0.25]));
        assert!((b.amount - 0.8).abs() < 1e-6);
        assert!((b.mix - 0.25).abs() < 1e-6);
        assert!(b.set_params(&[9.0, 9.0]));
        assert!((b.amount - 0.999).abs() < 1e-6, "amount clamps below 1");
        assert!((b.mix - 1.0).abs() < 1e-6, "mix clamps to 1");
    }
}
