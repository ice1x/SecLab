//! Wavetable oscillator: a pitch-tracking oscillator that **morphs** between single-cycle
//! waveforms.
//!
//! The classic "wavetable synth" sound: a bank of single-cycle frames, played back at the
//! note's pitch, with a `position` that sweeps continuously across the bank — sweep it with
//! an envelope or LFO and the timbre evolves. This node ships a small built-in bank of four
//! frames morphing **sine → triangle → saw → square**, each synthesised **additively** (a
//! capped harmonic series), so the frames are band-limited by construction rather than
//! aliasing like a naive sampled saw/square.
//!
//! Pitch tracks a `pitch_hz` control input (as [`crate::PitchOsc`] does), falling back to
//! `base_hz` at rest. The `position` control input is **summed** with the `position`
//! parameter and clamped to `[0, 1]`, so a bare node sits at the parameter's morph point and
//! a connected envelope/LFO moves around it.

use seclab_core::{ProcessContext, Processor};

/// Samples per single-cycle frame.
const LEN: usize = 1024;
/// Number of morph frames in the built-in bank.
const FRAMES: usize = 4;
/// Harmonics summed per additive frame — caps the brightness so the frames stay
/// band-limited (and cheap to build).
const HARMONICS: usize = 24;

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// A pitch-tracking wavetable oscillator morphing across a built-in four-frame bank.
#[derive(Debug, Clone)]
pub struct Wavetable {
    base_hz: f32,
    /// Morph position parameter in `[0, 1]` (0 = frame 0 … 1 = last frame).
    position: f32,
    freq_hz: f32,
    phase: f32,
    /// The built-in bank: `FRAMES` single-cycle waveforms of `LEN` samples.
    bank: Box<[[f32; LEN]; FRAMES]>,
}

impl Wavetable {
    /// Creates a wavetable oscillator resting at `base_hz` with the given morph `position`.
    #[must_use]
    pub fn new(base_hz: f32, position: f32) -> Self {
        Self {
            base_hz,
            position: position.clamp(0.0, 1.0),
            freq_hz: base_hz,
            phase: 0.0,
            bank: build_bank(),
        }
    }

    /// The frequency the oscillator is currently tuned to (Hz).
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.freq_hz
    }

    /// Reads the bank at normalised `phase` (`[0, 1)`) and morph `pos` (`[0, 1]`), linearly
    /// interpolating both the phase within a frame and between the two straddled frames.
    fn read(&self, phase: f32, pos: f32) -> f32 {
        let f = pos.clamp(0.0, 1.0) * (FRAMES - 1) as f32;
        let a = f.floor() as usize;
        let b = (a + 1).min(FRAMES - 1);
        let morph = f - a as f32;

        let x = phase * LEN as f32;
        let i0 = (x as usize) % LEN;
        let i1 = (i0 + 1) % LEN;
        let frac = x - x.floor();

        let sa = lerp(self.bank[a][i0], self.bank[a][i1], frac);
        let sb = lerp(self.bank[b][i0], self.bank[b][i1], frac);
        lerp(sa, sb, morph)
    }
}

impl Processor for Wavetable {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        // Control-rate: retune from the pitch input (Hz), holding base_hz when no note plays.
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let tracked = if pitch >= MIN_HZ { pitch } else { self.base_hz };
        self.freq_hz = tracked.clamp(MIN_HZ, MAX_HZ);

        // Morph = parameter + CV input (unconnected reads 0 → sits at the parameter).
        let pos_cv = inputs
            .get(1)
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let pos = (self.position + pos_cv).clamp(0.0, 1.0);

        let dt = if ctx.sample_rate > 0.0 {
            self.freq_hz / ctx.sample_rate
        } else {
            0.0
        };
        let Some(out) = outputs.first_mut() else {
            return;
        };
        for s in out.iter_mut().take(ctx.frames) {
            *s = self.read(self.phase, pos);
            self.phase = (self.phase + dt).fract();
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Builds the four-frame bank additively: sine, triangle, saw, square, each a capped
/// harmonic series (so band-limited), amplitude-normalised to about `[-1, 1]`.
fn build_bank() -> Box<[[f32; LEN]; FRAMES]> {
    use core::f32::consts::{PI, TAU};
    let mut bank = Box::new([[0.0f32; LEN]; FRAMES]);
    for i in 0..LEN {
        let t = TAU * (i as f32 / LEN as f32);

        // Frame 0 — sine (the single fundamental).
        bank[0][i] = t.sin();

        // Frame 1 — triangle: odd harmonics, amplitude 1/n², alternating sign.
        let mut tri = 0.0;
        let mut sign = 1.0;
        let mut n = 1;
        while n <= HARMONICS {
            tri += sign * (n as f32 * t).sin() / (n * n) as f32;
            sign = -sign;
            n += 2;
        }
        bank[1][i] = tri * (8.0 / (PI * PI));

        // Frame 2 — saw: all harmonics, amplitude 1/n.
        let mut saw = 0.0;
        for n in 1..=HARMONICS {
            saw += (n as f32 * t).sin() / n as f32;
        }
        bank[2][i] = saw * (2.0 / PI);

        // Frame 3 — square: odd harmonics, amplitude 1/n.
        let mut sq = 0.0;
        let mut n = 1;
        while n <= HARMONICS {
            sq += (n as f32 * t).sin() / n as f32;
            n += 2;
        }
        bank[3][i] = sq * (4.0 / PI);
    }
    bank
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(wt: &mut Wavetable, note_hz: f32, pos_cv: f32, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
        };
        let pitch = vec![note_hz; frames];
        let posv = vec![pos_cv; frames];
        let mut out = vec![0.0; frames];
        {
            let inputs: [&[f32]; 2] = [&pitch, &posv];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            wt.process(&ctx, &inputs, &mut outs);
        }
        out
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn produces_a_finite_audible_tone() {
        let mut wt = Wavetable::new(220.0, 0.0);
        let out = render(&mut wt, 220.0, 0.0, 1024);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            peak(&out) > 0.3,
            "a wavetable tone must be audible: {}",
            peak(&out)
        );
    }

    #[test]
    fn tracks_the_pitch_input() {
        // Higher note → more zero crossings over the same window.
        let crossings = |hz: f32| {
            let mut wt = Wavetable::new(110.0, 0.5);
            let out = render(&mut wt, hz, 0.0, 4096);
            out.windows(2)
                .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
                .count()
        };
        assert!(
            crossings(440.0) > crossings(110.0),
            "pitch must track the input"
        );
    }

    #[test]
    fn position_morphs_the_timbre() {
        // Frame 0 (sine) is far duller than frame 3 (square): compare high-frequency content
        // via the mean absolute sample-to-sample difference.
        let brightness = |pos: f32| {
            let mut wt = Wavetable::new(220.0, pos);
            let out = render(&mut wt, 220.0, 0.0, 2048);
            let d: f32 = out.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            d / out.len() as f32
        };
        assert!(
            brightness(1.0) > brightness(0.0) * 1.5,
            "morphing toward the square frame must add harmonics: sine {}, square {}",
            brightness(0.0),
            brightness(1.0)
        );
    }

    #[test]
    fn position_cv_adds_to_the_parameter() {
        // The position CV input pushes the morph: param 0 + cv 1.0 lands near the square frame.
        let mut at_param = Wavetable::new(220.0, 0.0);
        let dull = {
            let out = render(&mut at_param, 220.0, 0.0, 2048);
            let d: f32 = out.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            d / out.len() as f32
        };
        let mut swept = Wavetable::new(220.0, 0.0);
        let bright = {
            let out = render(&mut swept, 220.0, 1.0, 2048);
            let d: f32 = out.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            d / out.len() as f32
        };
        assert!(
            bright > dull * 1.5,
            "position CV must move the morph: {dull} vs {bright}"
        );
    }
}
