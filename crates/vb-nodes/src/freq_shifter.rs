//! Frequency shifter: single-sideband (Bode) shift of the whole spectrum by a fixed number
//! of hertz.
//!
//! Unlike a pitch shifter (which multiplies every frequency by a ratio, preserving harmonic
//! relationships) or a ring modulator (which produces *both* sum and difference sidebands), a
//! frequency shifter adds a constant `shift_hz` to every partial. Harmonics `f, 2f, 3f…`
//! become `f+s, 2f+s, 3f+s…` — no longer integer multiples, so the sound turns metallic and
//! inharmonic, and feedback patches drift endlessly. This is the classic Bode/Haas effect.
//!
//! Implementation: form the analytic signal with an FIR Hilbert transformer (a 90° phase
//! network with flat magnitude), rotate it by a complex exponential at `shift_hz`, and take
//! the real part — `y = x·cos(ωₛt) − H{x}·sin(ωₛt)` for an up-shift. A positive `shift_hz`
//! shifts up, a negative one down. The dry path is delayed by the Hilbert filter's latency so
//! the `mix` blend and the `shift_hz = 0` case stay phase-aligned.
//!
//! The Hilbert FIR is the ideal `h[n] = 2·sin²(πn/2)/(πn)` (i.e. `2/(πn)` for odd `n`, zero
//! for even `n`), windowed (Blackman) and delayed by `(N−1)/2`.
//!
//! Primary sources: the ideal Hilbert-transformer impulse response and the windowed/delayed
//! FIR design (Scilab `hilb`; Reilly, Frazer & Boashash, "Analytic signal generation: Tips
//! and traps", IEEE Trans. Signal Processing 42, 1994).

use vb_core::{ProcessContext, Processor};

/// FIR Hilbert transformer length (odd, so the delay is an integer). 65 taps give a usable
/// band of roughly 1–23 kHz at 48 kHz.
const N: usize = 65;
/// Integer group delay of the linear-phase Hilbert FIR, `(N − 1) / 2`.
const DELAY: usize = (N - 1) / 2;

/// A single-sideband frequency shifter.
///
/// Input 0 is audio. Output 0 is the spectrum shifted by `shift_hz`, blended with the
/// (latency-matched) dry signal by `mix`.
#[derive(Debug, Clone)]
pub struct FreqShifter {
    shift_hz: f32,
    mix: f32,
    /// Windowed Hilbert FIR taps (length `N`).
    h: Vec<f32>,
    /// Ring buffer of the last `N` input samples.
    buf: Vec<f32>,
    write: usize,
    /// Phase of the shifting oscillator, in cycles `[0, 1)`.
    phase: f32,
}

impl FreqShifter {
    /// Creates a frequency shifter of `shift_hz` (positive = up, negative = down) and dry/wet
    /// `mix` (clamped to `0..=1`).
    #[must_use]
    pub fn new(shift_hz: f32, mix: f32) -> Self {
        let mut h = vec![0.0_f32; N];
        for (j, hj) in h.iter_mut().enumerate() {
            let n = j as i32 - DELAY as i32;
            // Ideal Hilbert impulse response: 2/(πn) for odd n, 0 for even n (and n = 0).
            let ideal = if n != 0 && n % 2 != 0 {
                2.0 / (core::f32::consts::PI * n as f32)
            } else {
                0.0
            };
            // Blackman window over the N taps.
            let x = core::f32::consts::TAU * j as f32 / (N - 1) as f32;
            let window = 0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos();
            *hj = ideal * window;
        }
        Self {
            shift_hz,
            mix: mix.clamp(0.0, 1.0),
            h,
            buf: vec![0.0; N],
            write: 0,
            phase: 0.0,
        }
    }
}

impl Processor for FreqShifter {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let dt = self.shift_hz / sr; // cycles per sample (may be negative)
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            self.buf[self.write] = x;

            // Quadrature path: convolve with the Hilbert FIR. h[j] weights x[n-j].
            let mut quad = 0.0;
            for (j, &hj) in self.h.iter().enumerate() {
                if hj != 0.0 {
                    let idx = (self.write + N - j) % N;
                    quad += hj * self.buf[idx];
                }
            }
            // In-phase path: the dry input delayed by the filter's group delay, so it lines up
            // with the quadrature path.
            let real = self.buf[(self.write + N - DELAY) % N];

            let (s, c) = (core::f32::consts::TAU * self.phase).sin_cos();
            let wet = real * c - quad * s;
            *out = real * (1.0 - self.mix) + wet * self.mix;

            self.write = (self.write + 1) % N;
            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            } else if self.phase < 0.0 {
                self.phase += 1.0;
            }
        }
    }

    fn reset(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.write = 0;
        self.phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn run(f: &mut FreqShifter, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            f.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * hz * i as f32 / SR).sin())
            .collect()
    }

    /// Magnitude of the DFT of `xs` at frequency `hz` (direct Goertzel-style sum).
    fn mag_at(xs: &[f32], hz: f32) -> f32 {
        let w = core::f32::consts::TAU * hz / SR;
        let (mut re, mut im) = (0.0_f32, 0.0_f32);
        for (n, &x) in xs.iter().enumerate() {
            re += x * (w * n as f32).cos();
            im -= x * (w * n as f32).sin();
        }
        (re * re + im * im).sqrt() / xs.len() as f32
    }

    #[test]
    fn zero_shift_is_a_latency_passthrough() {
        // shift 0, full wet: cos = 1, sin = 0, so the output is the dry input delayed by DELAY.
        let input = tone(2_000.0, 4_096);
        let out = run(&mut FreqShifter::new(0.0, 1.0), &input);
        for k in (DELAY + 100)..input.len() {
            assert!(
                (out[k] - input[k - DELAY]).abs() < 1e-5,
                "zero shift is a pure {DELAY}-sample delay"
            );
        }
    }

    #[test]
    fn a_positive_shift_moves_the_tone_up() {
        // A 2 kHz tone shifted by +500 Hz should concentrate its energy at 2.5 kHz, not 2 kHz.
        let input = tone(2_000.0, 8_192);
        let out = run(&mut FreqShifter::new(500.0, 1.0), &input);
        let tail = &out[1_024..];
        let at_shifted = mag_at(tail, 2_500.0);
        let at_original = mag_at(tail, 2_000.0);
        assert!(
            at_shifted > 10.0 * at_original,
            "energy moves to 2.5 kHz (shifted {at_shifted}, original {at_original})"
        );
    }

    #[test]
    fn a_negative_shift_moves_the_tone_down() {
        let input = tone(2_000.0, 8_192);
        let out = run(&mut FreqShifter::new(-500.0, 1.0), &input);
        let tail = &out[1_024..];
        let at_shifted = mag_at(tail, 1_500.0);
        let at_original = mag_at(tail, 2_000.0);
        assert!(
            at_shifted > 10.0 * at_original,
            "energy moves to 1.5 kHz (shifted {at_shifted}, original {at_original})"
        );
    }

    #[test]
    fn the_single_sideband_suppresses_the_mirror_image() {
        // A ring modulator would put equal energy at 2000±500; the SSB shifter keeps only the
        // upper sideband, so the lower one (1.5 kHz) is far weaker than the upper (2.5 kHz).
        let input = tone(2_000.0, 8_192);
        let out = run(&mut FreqShifter::new(500.0, 1.0), &input);
        let tail = &out[1_024..];
        assert!(
            mag_at(tail, 2_500.0) > 10.0 * mag_at(tail, 1_500.0),
            "the opposite sideband is suppressed"
        );
    }

    #[test]
    fn mix_zero_is_the_dry_signal() {
        // With mix 0 the shift is irrelevant: output is the latency-delayed dry input.
        let input = tone(2_000.0, 4_096);
        let out = run(&mut FreqShifter::new(500.0, 0.0), &input);
        for k in (DELAY + 100)..input.len() {
            assert!(
                (out[k] - input[k - DELAY]).abs() < 1e-5,
                "mix 0 is the dry (delayed) signal"
            );
        }
    }

    #[test]
    fn output_stays_bounded_and_mix_is_clamped() {
        let input = tone(1_000.0, 4_096);
        let out = run(&mut FreqShifter::new(300.0, 1.0), &input);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 4.0),
            "bounded"
        );
        assert!(
            (FreqShifter::new(0.0, 9.0).mix - 1.0).abs() < 1e-6,
            "mix clamps to 1"
        );
    }
}
