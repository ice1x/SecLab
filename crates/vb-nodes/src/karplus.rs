//! Karplus–Strong plucked-string oscillator: a gated physical-model of a struck/plucked
//! string.
//!
//! Where the other generators sustain a tone for as long as they run, a plucked string is
//! **excited once** and then rings down on its own. On each rising **gate** edge this node
//! fills a delay line of length `sample_rate / pitch` with a short noise burst — the "pluck"
//! — and thereafter feeds the line back through a two-point averaging low-pass. That
//! averaging is what makes the timbre darken and decay exactly as a real string does, and the
//! `damping` feedback loss sets how long the note sustains.
//!
//! The pitch is latched **at pluck time** from the `pitch_hz` control input (falling back to
//! `base_hz` at rest): a real string does not change length while it rings, so holding the
//! delay length for the life of the pluck is both simpler and more faithful than retuning
//! mid-decay. `tone` blends between the full averaging low-pass (dull, fast decay of the
//! highs) and a bright, barely-filtered string.

use vb_core::{ProcessContext, Processor};

/// Longest delay line we keep — sized for the lowest pitch ([`MIN_HZ`]) at a generous sample
/// rate, so even a 20 Hz pluck at 192 kHz fits.
const MAX_LEN: usize = 16_384;

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// A gated Karplus–Strong plucked-string voice.
#[derive(Debug, Clone)]
pub struct Karplus {
    base_hz: f32,
    /// Feedback loss per lap of the delay line in `[0, 1)`: higher rings longer.
    damping: f32,
    /// Brightness in `[0, 1]`: 0 = full averaging low-pass (dull), 1 = no averaging (bright).
    tone: f32,
    buf: Vec<f32>,
    /// Active delay-line length in samples (latched at pluck time).
    len: usize,
    pos: usize,
    gate_prev: bool,
    /// Whether a pluck has been excited yet (silent until first plucked).
    plucked: bool,
    /// Excitation RNG state (xorshift32), so a pluck's noise burst is deterministic.
    rng: u32,
}

impl Karplus {
    /// Creates a plucked string resting at `base_hz`, with feedback `damping` and `tone`
    /// brightness both clamped to their valid ranges.
    #[must_use]
    pub fn new(base_hz: f32, damping: f32, tone: f32) -> Self {
        Self {
            base_hz,
            damping: damping.clamp(0.0, 0.9999),
            tone: tone.clamp(0.0, 1.0),
            buf: vec![0.0; MAX_LEN],
            len: 2,
            pos: 0,
            gate_prev: false,
            plucked: false,
            rng: 0x2545_f491,
        }
    }

    /// The delay-line length (samples) the current pluck is ringing at.
    #[must_use]
    pub fn delay_len(&self) -> usize {
        self.len
    }

    fn next_noise(&mut self) -> f32 {
        // xorshift32 → a float in [-1, 1).
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Excites the string: latch the delay length from `freq` and fill the line with noise.
    fn pluck(&mut self, freq: f32, sample_rate: f32) {
        let f = freq.clamp(MIN_HZ, MAX_HZ);
        let raw = if sample_rate > 0.0 {
            (sample_rate / f).round() as usize
        } else {
            2
        };
        self.len = raw.clamp(2, MAX_LEN);
        for i in 0..self.len {
            self.buf[i] = self.next_noise();
        }
        self.pos = 0;
        self.plucked = true;
    }
}

impl Processor for Karplus {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let gate = inputs.first().copied().unwrap_or(&[]);
        let pitch = inputs.get(1).copied().unwrap_or(&[]);
        let Some(out) = outputs.first_mut() else {
            return;
        };

        for n in 0..ctx.frames {
            let g = gate.get(n).copied().unwrap_or(0.0) >= 0.5;
            if g && !self.gate_prev {
                // Pitch is latched at pluck time; unconnected/at-rest falls back to base_hz.
                let f = pitch.get(n).copied().unwrap_or(0.0);
                let f = if f >= MIN_HZ { f } else { self.base_hz };
                self.pluck(f, ctx.sample_rate);
            }
            self.gate_prev = g;

            let s = if self.plucked {
                let cur = self.buf[self.pos];
                let prev_idx = (self.pos + self.len - 1) % self.len;
                let prev = self.buf[prev_idx];
                // tone=1 → keep the sample (bright); tone=0 → the classic two-point average.
                let filtered = self.tone * cur + (1.0 - self.tone) * 0.5 * (cur + prev);
                self.buf[self.pos] = self.damping * filtered;
                self.pos = (self.pos + 1) % self.len;
                cur
            } else {
                0.0
            };
            if let Some(o) = out.get_mut(n) {
                *o = s;
            }
        }
    }

    fn reset(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.pos = 0;
        self.gate_prev = false;
        self.plucked = false;
        self.rng = 0x2545_f491;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(k: &mut Karplus, gate: &[f32], pitch_hz: f32, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
        };
        let pitch = vec![pitch_hz; frames];
        let mut out = vec![0.0; frames];
        {
            let inputs: [&[f32]; 2] = [gate, &pitch];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            k.process(&ctx, &inputs, &mut outs);
        }
        out
    }

    fn energy(v: &[f32]) -> f32 {
        v.iter().map(|s| s * s).sum()
    }

    #[test]
    fn silent_until_the_first_pluck() {
        let mut k = Karplus::new(220.0, 0.996, 0.5);
        let gate = vec![0.0; 512];
        let out = render(&mut k, &gate, 220.0, 512);
        assert_eq!(energy(&out), 0.0, "no gate → no sound");
    }

    #[test]
    fn plucks_on_a_rising_gate_edge() {
        let mut k = Karplus::new(220.0, 0.996, 0.5);
        // One-sample trigger, then release: the string should ring on afterwards.
        let mut gate = vec![0.0; 2048];
        gate[0] = 1.0;
        let out = render(&mut k, &gate, 220.0, 2048);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(energy(&out) > 1.0, "a pluck must excite an audible ring");
    }

    #[test]
    fn the_pluck_rings_down() {
        let mut k = Karplus::new(220.0, 0.99, 0.3);
        let mut gate = vec![0.0; 16_000];
        gate[0] = 1.0;
        let out = render(&mut k, &gate, 220.0, 16_000);
        let early = energy(&out[0..2000]);
        let late = energy(&out[12_000..14_000]);
        assert!(
            late < early * 0.5,
            "damping must let the string decay: early {early}, late {late}"
        );
    }

    #[test]
    fn tracks_the_pitch_at_pluck_time() {
        // Recover the ringing pitch by autocorrelation: the string repeats every `period`
        // samples, so the lag maximising the autocorrelation is the fundamental period.
        // (Zero-crossing counting is unreliable here — the residual excitation noise, which
        // smooths more slowly on the longer low-pitch delay line, dominates the count.)
        let period = |hz: f32| {
            let mut k = Karplus::new(110.0, 0.997, 0.4);
            let mut gate = vec![0.0; 6000];
            gate[0] = 1.0;
            let out = render(&mut k, &gate, hz, 6000);
            let w = &out[2000..6000];
            let mut best_lag = 0;
            let mut best = f32::MIN;
            for lag in 40..600 {
                let corr: f32 = w[..w.len() - lag]
                    .iter()
                    .zip(&w[lag..])
                    .map(|(a, b)| a * b)
                    .sum();
                if corr > best {
                    best = corr;
                    best_lag = lag;
                }
            }
            best_lag
        };
        let low = period(110.0);
        let high = period(440.0);
        assert!(
            low > high,
            "a lower-pitched pluck must ring at a longer period: 110Hz={low}, 440Hz={high}"
        );
    }

    #[test]
    fn latched_delay_length_follows_pitch() {
        let mut low = Karplus::new(110.0, 0.99, 0.5);
        let mut hi = Karplus::new(880.0, 0.99, 0.5);
        let mut gate = vec![0.0; 8];
        gate[0] = 1.0;
        render(&mut low, &gate, 110.0, 8);
        render(&mut hi, &gate, 880.0, 8);
        assert!(
            low.delay_len() > hi.delay_len(),
            "lower pitch → longer delay line: {} vs {}",
            low.delay_len(),
            hi.delay_len()
        );
    }

    #[test]
    fn retriggers_on_a_new_edge() {
        let mut k = Karplus::new(220.0, 0.99, 0.5);
        // Pluck, let it decay a long way, then pluck again.
        let mut gate = vec![0.0; 20_000];
        gate[0] = 1.0;
        gate[16_000] = 1.0;
        let out = render(&mut k, &gate, 220.0, 20_000);
        let decayed = energy(&out[14_000..16_000]);
        let after = energy(&out[16_000..18_000]);
        assert!(
            after > decayed * 2.0,
            "a new edge must re-energise the string: decayed {decayed}, after {after}"
        );
    }

    #[test]
    fn reset_silences_the_string() {
        let mut k = Karplus::new(220.0, 0.996, 0.5);
        let mut gate = vec![0.0; 512];
        gate[0] = 1.0;
        render(&mut k, &gate, 220.0, 512);
        k.reset();
        let silent = vec![0.0; 512];
        let out = render(&mut k, &silent, 220.0, 512);
        assert_eq!(energy(&out), 0.0, "reset must clear the delay line");
    }
}
