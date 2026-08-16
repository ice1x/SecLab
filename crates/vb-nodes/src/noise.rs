//! Noise generator: white and pink, deterministic and allocation-free.
//!
//! White noise comes from a xorshift32 PRNG (seeded constant, so a patch renders the
//! same way twice and `reset()` replays the exact sequence). Pink noise is white run
//! through Paul Kellet's economy 3-pole filter — the classic −3 dB/octave
//! approximation, cheap enough for the audio thread.

use vb_core::{ProcessContext, Processor};

/// Which spectrum the generator produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseKind {
    /// Flat spectrum, uniform amplitude in [-1, 1].
    White,
    /// −3 dB/octave tilt (Kellet economy filter over white).
    Pink,
}

/// The seed every generator starts from — determinism is part of the contract.
const SEED: u32 = 0x9e37_79b9;

/// White/pink noise source with a `level` output scaler.
pub struct Noise {
    kind: NoiseKind,
    level: f32,
    state: u32,
    /// Kellet filter memory (pink only).
    b: [f32; 3],
}

impl Noise {
    /// Creates a generator of `kind` scaled by `level` (0..=1).
    #[must_use]
    pub fn new(kind: NoiseKind, level: f32) -> Self {
        Self {
            kind,
            level,
            state: SEED,
            b: [0.0; 3],
        }
    }

    /// Next white sample in [-1, 1] (xorshift32).
    fn white(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        // Map the full u32 range onto [-1, 1].
        (x as f32 / u32::MAX as f32).mul_add(2.0, -1.0)
    }

    /// One pink sample via Kellet's economy filter (already roughly unit-scaled).
    fn pink(&mut self) -> f32 {
        let w = self.white();
        self.b[0] = 0.997_65f32.mul_add(self.b[0], w * 0.099_046);
        self.b[1] = 0.963f32.mul_add(self.b[1], w * 0.296_516_4);
        self.b[2] = 0.57f32.mul_add(self.b[2], w * 1.052_691_3);
        (self.b[0] + self.b[1] + self.b[2] + w * 0.184_8) * 0.2
    }
}

impl Processor for Noise {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(out) = outputs.first_mut() else {
            return;
        };
        for o in out.iter_mut().take(ctx.frames) {
            let s = match self.kind {
                NoiseKind::White => self.white(),
                NoiseKind::Pink => self.pink(),
            };
            *o = s * self.level;
        }
    }

    fn reset(&mut self) {
        self.state = SEED;
        self.b = [0.0; 3];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(noise: &mut Noise, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
        };
        let mut out = vec![0.0; frames];
        noise.process(&ctx, &[], &mut [&mut out]);
        out
    }

    /// Sum of squared sample-to-sample differences — a cheap high-frequency-energy
    /// proxy: white noise jumps wildly, pink moves smoothly.
    fn diff_energy(xs: &[f32]) -> f32 {
        xs.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum()
    }

    fn energy(xs: &[f32]) -> f32 {
        xs.iter().map(|s| s * s).sum()
    }

    #[test]
    fn white_noise_is_bounded_and_actually_noisy() {
        let mut n = Noise::new(NoiseKind::White, 1.0);
        let out = render(&mut n, 4_096);
        assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
        let distinct = out.windows(2).filter(|w| w[0] != w[1]).count();
        assert!(distinct > 4_000, "not a constant or short cycle");
        let mean = out.iter().sum::<f32>() / out.len() as f32;
        assert!(mean.abs() < 0.05, "roughly zero-centred, got {mean}");
    }

    #[test]
    fn generation_is_deterministic_and_reset_replays_it() {
        let mut a = Noise::new(NoiseKind::Pink, 1.0);
        let first = render(&mut a, 512);

        let mut b = Noise::new(NoiseKind::Pink, 1.0);
        assert_eq!(render(&mut b, 512), first, "same seed, same sequence");

        let _ = render(&mut a, 512); // advance past the first block
        a.reset();
        assert_eq!(render(&mut a, 512), first, "reset replays from the seed");
    }

    #[test]
    fn pink_has_less_high_frequency_energy_than_white() {
        let mut white = Noise::new(NoiseKind::White, 1.0);
        let mut pink = Noise::new(NoiseKind::Pink, 1.0);
        let w = render(&mut white, 8_192);
        let p = render(&mut pink, 8_192);

        // Normalize the HF proxy by total energy so overall loudness cancels out.
        let white_tilt = diff_energy(&w) / energy(&w);
        let pink_tilt = diff_energy(&p) / energy(&p);
        assert!(
            pink_tilt < white_tilt * 0.5,
            "pink {pink_tilt} vs white {white_tilt}"
        );
    }

    #[test]
    fn level_scales_the_output() {
        let mut loud = Noise::new(NoiseKind::White, 1.0);
        let mut quiet = Noise::new(NoiseKind::White, 0.25);
        let l = render(&mut loud, 256);
        let q = render(&mut quiet, 256);
        for (l, q) in l.iter().zip(&q) {
            assert!((l * 0.25 - q).abs() < 1e-6);
        }
    }
}
