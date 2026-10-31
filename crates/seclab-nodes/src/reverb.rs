//! Basic reverb: a mono Freeverb — 8 damped feedback combs in parallel into 4
//! series allpasses.
//!
//! The classic Schroeder/Moorer topology with Freeverb's well-tried tunings
//! (delay lengths in samples at 44.1 kHz, scaled to the running sample rate).
//! All buffers are pre-allocated in the constructor for the highest supported
//! sample rate — `process()` never allocates.

use seclab_core::{ProcessContext, Processor};

/// Freeverb comb delays at 44.1 kHz.
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
/// Freeverb allpass delays at 44.1 kHz.
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
/// Highest sample rate the pre-allocated buffers cover.
const MAX_SR: f32 = 192_000.0;
const BASE_SR: f32 = 44_100.0;

/// Damped feedback comb filter.
struct Comb {
    buf: Vec<f32>,
    len: usize,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(tuning: usize) -> Self {
        let max_len = scaled(tuning, MAX_SR);
        Self {
            buf: vec![0.0; max_len],
            len: max_len,
            idx: 0,
            store: 0.0,
        }
    }

    fn tick(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out.mul_add(1.0 - damp, self.store * damp);
        self.buf[self.idx] = self.store.mul_add(feedback, input);
        self.idx = (self.idx + 1) % self.len;
        out
    }
}

/// Freeverb-style allpass diffuser.
struct Allpass {
    buf: Vec<f32>,
    len: usize,
    idx: usize,
}

impl Allpass {
    fn new(tuning: usize) -> Self {
        let max_len = scaled(tuning, MAX_SR);
        Self {
            buf: vec![0.0; max_len],
            len: max_len,
            idx: 0,
        }
    }

    fn tick(&mut self, input: f32) -> f32 {
        let bufout = self.buf[self.idx];
        self.buf[self.idx] = bufout.mul_add(0.5, input);
        self.idx = (self.idx + 1) % self.len;
        bufout - input
    }
}

/// Scales a 44.1 kHz tuning to `sample_rate` samples (at least 1).
fn scaled(tuning: usize, sample_rate: f32) -> usize {
    ((tuning as f32 * sample_rate / BASE_SR) as usize).max(1)
}

/// Mono Freeverb: room size, damping, dry/wet mix.
pub struct Reverb {
    room_size: f32,
    damping: f32,
    mix: f32,
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
    /// Sample rate the delay lengths are currently set for.
    tuned_sr: f32,
}

impl Reverb {
    /// Creates a reverb (`room_size`, `damping`, `mix` all 0..=1).
    #[must_use]
    pub fn new(room_size: f32, damping: f32, mix: f32) -> Self {
        Self {
            room_size: room_size.clamp(0.0, 1.0),
            damping: damping.clamp(0.0, 1.0),
            mix: mix.clamp(0.0, 1.0),
            combs: COMB_TUNING.iter().map(|&t| Comb::new(t)).collect(),
            allpasses: ALLPASS_TUNING.iter().map(|&t| Allpass::new(t)).collect(),
            tuned_sr: 0.0,
        }
    }

    /// Adjusts the active delay lengths to `sample_rate` (buffers already fit).
    fn retune(&mut self, sample_rate: f32) {
        if (sample_rate - self.tuned_sr).abs() < f32::EPSILON {
            return;
        }
        self.tuned_sr = sample_rate;
        for (comb, &tuning) in self.combs.iter_mut().zip(&COMB_TUNING) {
            comb.len = scaled(tuning, sample_rate).min(comb.buf.len());
            comb.idx %= comb.len;
        }
        for (ap, &tuning) in self.allpasses.iter_mut().zip(&ALLPASS_TUNING) {
            ap.len = scaled(tuning, sample_rate).min(ap.buf.len());
            ap.idx %= ap.len;
        }
    }
}

impl Processor for Reverb {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (Some(input), Some(out)) = (inputs.first(), outputs.first_mut()) else {
            return;
        };
        self.retune(ctx.sample_rate.max(1.0));
        // Freeverb ranges: feedback 0.7..0.98, damp scaled by 0.4.
        let feedback = self.room_size.mul_add(0.28, 0.7);
        let damp = self.damping * 0.4;

        for i in 0..ctx.frames {
            let dry = input[i];
            // Attenuate into the tank so eight combs don't overload.
            let tank_in = dry * 0.015;
            let mut wet = 0.0;
            for comb in &mut self.combs {
                wet += comb.tick(tank_in, feedback, damp);
            }
            for ap in &mut self.allpasses {
                wet = ap.tick(wet);
            }
            out[i] = wet.mul_add(self.mix * 3.0, dry * (1.0 - self.mix));
        }
    }

    fn reset(&mut self) {
        for comb in &mut self.combs {
            comb.buf.fill(0.0);
            comb.idx = 0;
            comb.store = 0.0;
        }
        for ap in &mut self.allpasses {
            ap.buf.fill(0.0);
            ap.idx = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Feeds an impulse and renders `seconds` of output.
    fn impulse_response(reverb: &mut Reverb, seconds: f32) -> Vec<f32> {
        let frames = (SR * seconds) as usize;
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut input = vec![0.0_f32; frames];
        input[0] = 1.0;
        let mut out = vec![0.0; frames];
        reverb.process(&ctx, &[&input], &mut [&mut out]);
        out
    }

    fn energy(xs: &[f32]) -> f32 {
        xs.iter().map(|s| s * s).sum()
    }

    #[test]
    fn an_impulse_grows_a_tail() {
        let mut r = Reverb::new(0.7, 0.3, 1.0); // fully wet
        let out = impulse_response(&mut r, 1.0);
        // Late energy exists (a tail), and it decays over time.
        let early = energy(&out[..24_000]);
        let late = energy(&out[24_000..]);
        assert!(early > 0.0 && late > 0.0, "tail present");
        assert!(late < early, "tail decays");
    }

    #[test]
    fn bigger_rooms_ring_longer() {
        let mut small = Reverb::new(0.1, 0.3, 1.0);
        let mut large = Reverb::new(0.95, 0.3, 1.0);
        let small_out = impulse_response(&mut small, 1.0);
        let large_out = impulse_response(&mut large, 1.0);
        let late = |xs: &[f32]| energy(&xs[24_000..]);
        assert!(
            late(&large_out) > late(&small_out) * 5.0,
            "large {} vs small {}",
            late(&large_out),
            late(&small_out)
        );
    }

    #[test]
    fn mix_zero_is_a_dry_passthrough() {
        let mut r = Reverb::new(0.8, 0.2, 0.0);
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 64,
        };
        let input: Vec<f32> = (0..64).map(|i| (i as f32 * 0.1).sin()).collect();
        let mut out = vec![0.0; 64];
        r.process(&ctx, &[&input], &mut [&mut out]);
        assert_eq!(out, input);
    }

    #[test]
    fn output_stays_bounded_over_a_long_run() {
        let mut r = Reverb::new(1.0, 0.0, 1.0); // worst case: max room, no damping
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 256,
        };
        let input = vec![0.5_f32; 256];
        let mut out = vec![0.0; 256];
        for _ in 0..500 {
            r.process(&ctx, &[&input], &mut [&mut out]);
        }
        assert!(out.iter().all(|s| s.is_finite() && s.abs() < 50.0));
    }

    #[test]
    fn reset_silences_the_tail() {
        let mut r = Reverb::new(0.9, 0.1, 1.0);
        impulse_response(&mut r, 0.5);
        r.reset();
        let ctx = ProcessContext {
            sample_rate: SR,
            frames: 4_800,
        };
        let silence = vec![0.0_f32; 4_800];
        let mut out = vec![9.0; 4_800];
        r.process(&ctx, &[&silence], &mut [&mut out]);
        assert!(out.iter().all(|&s| s == 0.0), "no stale tail after reset");
    }
}
