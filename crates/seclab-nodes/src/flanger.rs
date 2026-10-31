//! Flanger: a short LFO-modulated delay with feedback, mixed with the dry signal.
//!
//! A flanger sweeps a very short delay (here around 1 ms, modulated by a few milliseconds)
//! against the dry signal. Summing the two produces a comb filter whose notches slide up and
//! down the spectrum as the delay moves — the classic jet-plane "whoosh". Feeding the delayed
//! output back into the line (`feedback`) sharpens those notches into a resonant, metallic
//! sweep; a negative `feedback` inverts the reflection and shifts the comb's character. This
//! is what sets a flanger apart from [`crate::Chorus`], whose longer (~12 ms) delay and absent
//! feedback give a gentle detune rather than a resonant sweep.
//!
//! `rate_hz` is the sweep speed, `depth_ms` the delay excursion, `feedback` the regeneration
//! amount (`-0.95..=0.95` for stability) and `mix` the dry/wet balance.
//!
//! Ranges follow common flanger implementations: a ~1–5 ms delay swept a few ms, feedback up
//! to roughly ±95 %, and a slow LFO (fractions of a hertz up to a few hertz).

use seclab_core::{ProcessContext, Processor};

/// Base delay the LFO modulates upward from (ms): short, so the comb notches land in the
/// audible range rather than the gentle detune of a chorus.
const BASE_MS: f32 = 1.0;
/// Delay-line length in samples: ample for BASE + max depth even at 192 kHz, power of two.
const BUF_LEN: usize = 8192;
/// Feedback is clamped to this magnitude so the feedback comb can never run away.
const MAX_FEEDBACK: f32 = 0.95;

/// A single-voice modulated-delay flanger with feedback.
///
/// Input 0 is audio. Output 0 blends the dry input with the modulated, regenerated delay tap
/// by `mix`.
#[derive(Debug, Clone)]
pub struct Flanger {
    rate_hz: f32,
    depth_ms: f32,
    feedback: f32,
    mix: f32,
    buf: Vec<f32>,
    write: usize,
    phase: f32,
}

impl Flanger {
    /// Creates a flanger with LFO `rate_hz`, modulation `depth_ms`, `feedback`
    /// (clamped to `-0.95..=0.95`) and dry/wet `mix` (clamped to `0..=1`).
    #[must_use]
    pub fn new(rate_hz: f32, depth_ms: f32, feedback: f32, mix: f32) -> Self {
        Self {
            rate_hz,
            depth_ms: depth_ms.max(0.0),
            feedback: feedback.clamp(-MAX_FEEDBACK, MAX_FEEDBACK),
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

impl Processor for Flanger {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let dt = self.rate_hz / sr;
        let input = inputs.first().copied().unwrap_or(&[]);

        for (i, out) in outputs[0].iter_mut().enumerate().take(ctx.frames) {
            let x = input.get(i).copied().unwrap_or(0.0);

            // LFO in [0, 1] sweeps the delay from BASE up to BASE + depth.
            let lfo = 0.5 + 0.5 * (core::f32::consts::TAU * self.phase).sin();
            let delay_ms = BASE_MS + self.depth_ms * lfo;
            let delay_samp = delay_ms * 0.001 * sr;
            let wet = self.read_delayed(delay_samp);

            // Regenerate: the delayed tap is fed back into the line, deepening the notches.
            self.buf[self.write] = x + self.feedback * wet;

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

    fn run(f: &mut Flanger, input: &[f32]) -> Vec<f32> {
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

    fn tone(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (core::f32::consts::TAU * 220.0 * i as f32 / SR).sin())
            .collect()
    }

    #[test]
    fn mix_zero_passes_the_input_through() {
        let input = tone(1_024);
        let out = run(&mut Flanger::new(0.5, 3.0, 0.5, 0.0), &input);
        for (a, b) in input.iter().zip(&out) {
            assert!((a - b).abs() < 1e-6, "dry passes through at mix 0");
        }
    }

    #[test]
    fn full_wet_starts_from_the_silent_delay_line() {
        // BASE_MS (~1 ms ≈ 48 samples at 48 kHz) of the wet tap is the still-silent line, so
        // the very first sample is near zero even at full wet.
        let input = tone(4_800);
        let out = run(&mut Flanger::new(0.3, 3.0, 0.5, 1.0), &input);
        assert!(out[0].abs() < 1e-6, "the delayed tap starts silent");
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() <= 2.0),
            "output stays bounded with feedback"
        );
    }

    #[test]
    fn wet_output_differs_from_the_dry_input() {
        let input = tone(4_800);
        let out = run(&mut Flanger::new(1.0, 4.0, 0.6, 1.0), &input);
        let diff = input[1_000..]
            .iter()
            .zip(&out[1_000..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff > 0.1,
            "the modulated feedback delay reshapes the signal (max diff {diff})"
        );
    }

    #[test]
    fn feedback_lengthens_the_decay_tail() {
        // Feed an impulse and let the flanger ring. With regeneration the delayed echoes
        // recirculate, so the tail energy is strictly greater than with no feedback.
        let mut impulse = vec![0.0_f32; 4_096];
        impulse[0] = 1.0;
        let tail = |fb: f32| -> f32 {
            let out = run(&mut Flanger::new(0.1, 3.0, fb, 1.0), &impulse);
            out[200..].iter().map(|s| s.abs()).sum::<f32>()
        };
        assert!(
            tail(0.8) > tail(0.0) + 1e-3,
            "feedback should recirculate energy into a longer tail"
        );
    }

    #[test]
    fn high_feedback_stays_bounded() {
        // Even pushed past the clamp with a hot input, the feedback comb must not blow up.
        let input: Vec<f32> = (0..8_000)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let out = run(&mut Flanger::new(2.0, 5.0, 5.0, 1.0), &input);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() < 50.0),
            "clamped feedback keeps the output finite and bounded"
        );
    }

    #[test]
    fn silent_input_stays_silent_and_params_are_clamped() {
        let out = run(&mut Flanger::new(1.0, 3.0, 0.5, 1.0), &[0.0; 1_024]);
        assert!(out.iter().all(|&s| s == 0.0));
        let f = Flanger::new(1.0, -4.0, 9.0, 9.0);
        assert!((f.mix - 1.0).abs() < 1e-6, "mix clamps to 1");
        assert!(
            (f.feedback - MAX_FEEDBACK).abs() < 1e-6,
            "feedback clamps to 0.95"
        );
        assert!(f.depth_ms >= 0.0, "depth never negative");
    }
}
