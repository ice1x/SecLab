//! Modal resonator: a bank of tuned, high-Q resonant modes that **ring** when excited —
//! the "resonator" half of physical modelling, the complement to the [`crate::Karplus`]
//! exciter.
//!
//! Strike a bell, a marimba bar or a wine glass and it rings at a set of characteristic
//! frequencies (its *modes*) that decay over time. This node models that with a bank of
//! high-Q band-pass filters tuned to partials of a base pitch and summed: feed it any
//! excitation — an impulse, a noise burst, a drum hit, even a full signal — and it rings that
//! excitation into a pitched, metallic tone. The pitch tracks a `pitch_hz` control input
//! (falling back to `base_hz` at rest), so it plays melodies.
//!
//! `decay` sets how long the modes ring (mapped to the modes' Q — longer is more bell-like and
//! sustained). `structure` morphs the partials from a **harmonic** series (1·f, 2·f, 3·f… —
//! string/tube-like) toward an **inharmonic**, bell-like set (1, 2.76, 5.40… — the classic
//! struck-metal ratios), so it sweeps from tonal to clangorous.

use seclab_core::{ProcessContext, Processor};

/// Number of resonant modes in the bank.
const MODES: usize = 5;

/// Harmonic partial ratios (structure = 0): a plain overtone series.
const HARMONIC: [f32; MODES] = [1.0, 2.0, 3.0, 4.0, 5.0];
/// Inharmonic partial ratios (structure = 1): the classic struck-bell/tubular set.
const INHARMONIC: [f32; MODES] = [1.0, 2.76, 5.40, 8.93, 13.34];

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// One RBJ band-pass biquad (constant 0 dB peak gain), retuned per block, state preserved
/// across blocks so a high-Q mode rings on.
#[derive(Debug, Clone, Default)]
struct Mode {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Mode {
    fn tick(&mut self, xn: f32, freq: f32, q: f32, sample_rate: f32) -> f32 {
        let f0 = freq.clamp(1.0, 0.49 * sample_rate);
        let w0 = core::f32::consts::TAU * (f0 / sample_rate);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q.max(1e-4));
        let a0 = 1.0 + alpha;
        let b0 = alpha / a0;
        let b2 = -alpha / a0;
        let a1 = -2.0 * cos / a0;
        let a2 = (1.0 - alpha) / a0;

        let yn = b0 * xn + b2 * self.x2 - a1 * self.y1 - a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = xn;
        self.y2 = self.y1;
        self.y1 = yn;
        yn
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A modal (struck-body) resonator: a bank of tuned high-Q modes summed.
#[derive(Debug, Clone)]
pub struct Resonator {
    base_hz: f32,
    /// Ring length in `[0, 1]` (mapped to the modes' Q).
    decay: f32,
    /// Harmonic (0) → inharmonic/bell (1) partial spread.
    structure: f32,
    modes: [Mode; MODES],
}

impl Resonator {
    /// Creates a resonator resting at `base_hz` with the given `decay` and `structure`, both
    /// clamped to `[0, 1]`.
    #[must_use]
    pub fn new(base_hz: f32, decay: f32, structure: f32) -> Self {
        Self {
            base_hz,
            decay: decay.clamp(0.0, 1.0),
            structure: structure.clamp(0.0, 1.0),
            modes: Default::default(),
        }
    }
}

impl Processor for Resonator {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        let pitch = inputs
            .get(1)
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let Some(out) = outputs.first_mut() else {
            return;
        };
        if ctx.sample_rate <= 0.0 {
            for (o, x) in out.iter_mut().zip(input.iter()).take(ctx.frames) {
                *o = *x;
            }
            return;
        }

        // Control-rate: latch pitch, per-mode frequency (morphed spread) and a shared Q.
        let tracked = if pitch >= MIN_HZ { pitch } else { self.base_hz };
        let f0 = tracked.clamp(MIN_HZ, MAX_HZ);
        // decay → Q: short and damped at 0, long and bell-like near 1.
        let q = 10.0 + self.decay * self.decay * 990.0;
        let mut freqs = [0.0f32; MODES];
        for (f, (&h, &ih)) in freqs.iter_mut().zip(HARMONIC.iter().zip(INHARMONIC.iter())) {
            let ratio = h + (ih - h) * self.structure;
            *f = f0 * ratio;
        }

        for n in 0..ctx.frames {
            let xn = input.get(n).copied().unwrap_or(0.0);
            let mut acc = 0.0;
            for (i, (m, &f)) in self.modes.iter_mut().zip(freqs.iter()).enumerate() {
                // Higher modes quieter, as in a real struck body. A high-Q band-pass has a
                // tiny transient gain (~1/Q), so a `q` makeup restores an audible ring whose
                // amplitude does not collapse as `decay` (and thus Q) rises.
                let gain = q / (i as f32 + 1.0);
                acc += m.tick(xn, f, q, ctx.sample_rate) * gain;
            }
            if let Some(o) = out.get_mut(n) {
                *o = acc;
            }
        }
    }

    fn reset(&mut self) {
        for m in &mut self.modes {
            m.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(r: &mut Resonator, input: &[f32], pitch_hz: f32) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: input.len(),
        };
        let pitch = vec![pitch_hz; input.len()];
        let mut out = vec![0.0; input.len()];
        {
            let ins: [&[f32]; 2] = [input, &pitch];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            r.process(&ctx, &ins, &mut outs);
        }
        out
    }

    /// A short noise burst then silence — the "strike" that excites the modes (a single unit
    /// impulse barely stirs a high-Q band-pass, just as a real body needs a real hit).
    fn strike(frames: usize) -> Vec<f32> {
        let mut v = vec![0.0; frames];
        let mut s = 0x0123_4567u32;
        for x in v.iter_mut().take(256) {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            *x = (s as f32 / u32::MAX as f32) * 2.0 - 1.0;
        }
        v
    }

    fn energy(v: &[f32]) -> f32 {
        v.iter().map(|s| s * s).sum()
    }

    #[test]
    fn rings_after_a_strike() {
        // The strike lasts 256 samples; the resonator must keep ringing long after it stops.
        let mut r = Resonator::new(220.0, 0.9, 0.0);
        let out = run(&mut r, &strike(8192), 220.0);
        assert!(out.iter().all(|s| s.is_finite()));
        let late = energy(&out[4000..8000]);
        assert!(
            late > 1.0,
            "the resonator must ring on after the strike: {late}"
        );
    }

    #[test]
    fn decay_controls_the_ring_length() {
        // A longer decay leaves far more energy in the tail than a short one.
        let tail = |decay: f32| {
            let mut r = Resonator::new(220.0, decay, 0.0);
            let out = run(&mut r, &strike(12_000), 220.0);
            energy(&out[8000..12_000])
        };
        let short = tail(0.3);
        let long = tail(0.95);
        assert!(
            long > short * 4.0,
            "more decay must ring longer: short {short}, long {long}"
        );
    }

    #[test]
    fn tracks_the_pitch() {
        // The first mode rings at the base pitch: recover its period by autocorrelation and
        // check a lower pitch rings at a longer period.
        let period = |hz: f32| {
            let mut r = Resonator::new(110.0, 0.95, 0.0);
            let out = run(&mut r, &strike(12_000), hz);
            let w = &out[3000..12_000];
            let mut best_lag = 0;
            let mut best = f32::MIN;
            for lag in 40..800 {
                let c: f32 = w[..w.len() - lag]
                    .iter()
                    .zip(&w[lag..])
                    .map(|(a, b)| a * b)
                    .sum();
                if c > best {
                    best = c;
                    best_lag = lag;
                }
            }
            best_lag
        };
        assert!(
            period(110.0) > period(440.0),
            "lower pitch must ring at a longer period: 110={}, 440={}",
            period(110.0),
            period(440.0)
        );
    }

    #[test]
    fn structure_changes_the_partials() {
        // Harmonic vs inharmonic spreads put mode energy at different frequencies, so the
        // ringing output differs substantially for the same strike.
        let mut harm = Resonator::new(220.0, 0.9, 0.0);
        let mut bell = Resonator::new(220.0, 0.9, 1.0);
        let a = run(&mut harm, &strike(8192), 220.0);
        let b = run(&mut bell, &strike(8192), 220.0);
        let diff: f32 = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum();
        assert!(
            diff > 10.0,
            "harmonic and inharmonic structures must differ: {diff}"
        );
    }

    #[test]
    fn reset_silences_the_ring() {
        let mut r = Resonator::new(220.0, 0.9, 0.0);
        run(&mut r, &strike(4096), 220.0);
        r.reset();
        let out = run(&mut r, &vec![0.0; 2048], 220.0);
        assert_eq!(energy(&out), 0.0, "reset must clear the ringing modes");
    }
}
