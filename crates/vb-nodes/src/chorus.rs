//! Chorus: a short LFO-modulated delay mixed with the dry signal.
//!
//! One delayed copy whose delay time wobbles gently (an internal LFO sweeps it around ~12 ms
//! by a few milliseconds) detunes against the original through the Doppler pitch shift of the
//! moving tap. Blended back with the dry signal it thickens and widens a thin sound — the
//! shimmer of a chorus pedal or an ensemble string patch. `rate_hz` sets the wobble speed,
//! `depth_ms` its excursion, `mix` the dry/wet balance.

use vb_core::{ProcessContext, Processor};

/// Base delay the LFO modulates around (ms) and the buffer headroom.
const BASE_MS: f32 = 12.0;
/// Delay-line length in samples: enough for BASE + max depth at 192 kHz, power of two.
const BUF_LEN: usize = 8192;

/// A single-voice modulated-delay chorus.
///
/// Input 0 is audio. Output 0 blends the dry input with the modulated delay tap by `mix`.
#[derive(Debug, Clone)]
pub struct Chorus {
    rate_hz: f32,
    depth_ms: f32,
    mix: f32,
    buf: Vec<f32>,
    write: usize,
    phase: f32,
}

impl Chorus {
    /// Creates a chorus with LFO `rate_hz`, modulation `depth_ms` and dry/wet `mix`
    /// (clamped to `0..=1`).
    #[must_use]
    pub fn new(rate_hz: f32, depth_ms: f32, mix: f32) -> Self {
        Self {
            rate_hz,
            depth_ms: depth_ms.max(0.0),
            mix: mix.clamp(0.0, 1.0),
            buf: vec![0.0; BUF_LEN],
            write: 0,
            phase: 0.0,
        }
    }

    /// Reads the delay line `delay` samples back from the write head, linearly interpolated.
    fn read_delayed(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (BUF_LEN - 2) as f32);
        let read = self.write as f32 - d;
        let read = if read < 0.0 {
            read + BUF_LEN as f32
        } else {
            read
        };
        let i0 = read.floor() as usize % BUF_LEN;
        let i1 = (i0 + 1) % BUF_LEN;
        let frac = read - read.floor();
        self.buf[i0] * (1.0 - frac) + self.buf[i1] * frac
    }
}

impl Processor for Chorus {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let dt = self.rate_hz / sr;
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);
            self.buf[self.write] = x;

            // LFO in [0, 1] sweeps the delay from BASE to BASE + depth.
            let lfo = 0.5 + 0.5 * (core::f32::consts::TAU * self.phase).sin();
            let delay_ms = BASE_MS + self.depth_ms * lfo;
            let delay_samp = delay_ms * 0.001 * sr;
            let wet = self.read_delayed(delay_samp);

            *out = x * (1.0 - self.mix) + wet * self.mix;

            self.write = (self.write + 1) % BUF_LEN;
            self.phase += dt;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
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

    fn run(c: &mut Chorus, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: input.len(),
        };
        let mut out = vec![0.0_f32; input.len()];
        {
            let ins: [&[f32]; 1] = [input];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            c.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn tone(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * 220.0 * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn mix_zero_passes_the_input_through() {
        let input = tone(1_024);
        let out = run(&mut Chorus::new(1.0, 4.0, 0.0), &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "dry passes through");
        }
    }

    #[test]
    fn full_wet_delays_the_signal() {
        // With mix 1 and a delay of at least BASE_MS (~12 ms ≈ 576 samples at 48 kHz), the
        // early output is the (silent) delay line, not the immediate input.
        let input = tone(4_800);
        let out = run(&mut Chorus::new(0.5, 4.0, 1.0), &input);
        assert!(out[0].abs() < 1e-6, "the delayed tap starts silent");
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() <= 1.0 + 1e-6),
            "bounded"
        );
        // Later, the delayed signal has arrived and is non-silent.
        let tail_energy: f32 = out[2_000..].iter().map(|s| s.abs()).sum();
        assert!(
            tail_energy > 1.0,
            "the delayed signal eventually sounds (energy {tail_energy})"
        );
    }

    #[test]
    fn wet_output_differs_from_the_dry_input() {
        let input = tone(4_800);
        let out = run(&mut Chorus::new(2.0, 6.0, 1.0), &input);
        let diff = input[2_000..]
            .iter()
            .zip(&out[2_000..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff > 0.1,
            "the modulated delay reshapes the signal (max diff {diff})"
        );
    }

    #[test]
    fn silent_input_stays_silent_and_mix_is_clamped() {
        let out = run(&mut Chorus::new(1.0, 4.0, 1.0), &[0.0; 1_024]);
        assert!(out.iter().all(|&s| s == 0.0));
        assert!((Chorus::new(1.0, 4.0, 9.0).mix - 1.0).abs() < 1e-6);
    }
}
