//! Granular processor: a "grain cloud" that continuously records its input and replays it as
//! a swarm of short, overlapping, windowed **grains**.
//!
//! Granular synthesis chops sound into tens-of-milliseconds fragments and sprays many of them
//! back with independent position, pitch and timing — the result ranges from lush time-stretched
//! clouds to shimmering pitch-shifts to glitchy textures. This node records whatever is fed to
//! its audio input into a rolling buffer, and spawns grains that read back from a chosen point
//! behind the write head. Because playback position is decoupled from the record head, the same
//! node **time-stretches** (grains linger over old audio) and **pitch-shifts** (grains play back
//! faster or slower than they were recorded) without changing the other.
//!
//! - `grain_ms` — grain length; short is granular/buzzy, long is smooth and pad-like.
//! - `density` — grains spawned per second (their overlap fills the output).
//! - `position` — how far behind the live write head grains start (summed with a `position`
//!   CV input), so sweeping it scrubs through the recent past.
//! - `pitch` — grain playback speed (2 = up an octave, 0.5 = down an octave).
//! - `spray` — randomises each grain's start position for a looser, more diffuse cloud.
//!
//! Each grain is amplitude-windowed with a Hann window so grains fade in and out without clicks.

use seclab_core::{ProcessContext, Processor};

/// Rolling record buffer length in seconds.
const BUFFER_SECS: f32 = 2.0;
/// Buffer capacity in samples, sized for `BUFFER_SECS` at a generous sample rate.
const BUFFER_LEN: usize = 192_000; // 2 s at 96 kHz
/// Maximum simultaneously-active grains.
const MAX_GRAINS: usize = 64;

/// One active grain: a windowed read head into the record buffer.
#[derive(Debug, Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Fractional read position into the record buffer.
    read_pos: f32,
    /// Playback rate (samples advanced per output sample); the pitch factor.
    rate: f32,
    /// Samples elapsed since the grain started.
    age: f32,
    /// Total grain length in samples.
    length: f32,
}

/// A granular ("grain cloud") processor.
#[derive(Debug, Clone)]
pub struct Granular {
    grain_ms: f32,
    density: f32,
    position: f32,
    pitch: f32,
    spray: f32,
    buffer: Vec<f32>,
    /// Number of buffer samples actually in use (depends on sample rate).
    ring_len: usize,
    write_head: usize,
    grains: [Grain; MAX_GRAINS],
    /// Fractional accumulator that fires a new grain when it crosses 1.
    spawn_acc: f32,
    rng: u32,
}

impl Granular {
    /// Creates a granular processor.
    ///
    /// `grain_ms` is clamped to `[1, 1000]`, `density` to `[0.1, 200]` grains/s, `position` and
    /// `spray` to `[0, 1]`, and `pitch` to `[0.125, 8]`.
    #[must_use]
    pub fn new(grain_ms: f32, density: f32, position: f32, pitch: f32, spray: f32) -> Self {
        Self {
            grain_ms: grain_ms.clamp(1.0, 1000.0),
            density: density.clamp(0.1, 200.0),
            position: position.clamp(0.0, 1.0),
            pitch: pitch.clamp(0.125, 8.0),
            spray: spray.clamp(0.0, 1.0),
            buffer: vec![0.0; BUFFER_LEN],
            ring_len: BUFFER_LEN,
            write_head: 0,
            grains: [Grain::default(); MAX_GRAINS],
            spawn_acc: 0.0,
            rng: 0x9e37_79b9,
        }
    }

    fn rand01(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x as f32 / u32::MAX as f32
    }

    /// Reads the record buffer at fractional index `pos` with linear interpolation.
    fn read_buffer(&self, pos: f32) -> f32 {
        let len = self.ring_len as f32;
        let p = pos.rem_euclid(len);
        let i0 = p as usize % self.ring_len;
        let i1 = (i0 + 1) % self.ring_len;
        let frac = p - p.floor();
        self.buffer[i0] + (self.buffer[i1] - self.buffer[i0]) * frac
    }

    /// Starts a new grain behind the write head (or does nothing if all slots are busy).
    fn spawn(&mut self, position: f32, sample_rate: f32) {
        let length = (self.grain_ms * 0.001 * sample_rate).max(1.0);
        let back = position.clamp(0.0, 1.0) * BUFFER_SECS;
        let spray = self.spray * self.rand01() * BUFFER_SECS * 0.5;
        let offset = ((back + spray) * sample_rate) as isize;
        let start = (self.write_head as isize - offset).rem_euclid(self.ring_len as isize) as f32;

        let rate = self.pitch;
        if let Some(g) = self.grains.iter_mut().find(|g| !g.active) {
            *g = Grain {
                active: true,
                read_pos: start,
                rate,
                age: 0.0,
                length,
            };
        }
    }
}

impl Processor for Granular {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        let pos_cv = inputs
            .get(1)
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let Some(out) = outputs.first_mut() else {
            return;
        };
        if ctx.sample_rate <= 0.0 {
            for o in out.iter_mut().take(ctx.frames) {
                *o = 0.0;
            }
            return;
        }

        self.ring_len = ((BUFFER_SECS * ctx.sample_rate) as usize).clamp(1, BUFFER_LEN);
        let position = (self.position + pos_cv).clamp(0.0, 1.0);
        let spawn_rate = self.density / ctx.sample_rate;

        for n in 0..ctx.frames {
            // Record the live input into the rolling buffer.
            let xn = input.get(n).copied().unwrap_or(0.0);
            self.buffer[self.write_head] = xn;
            self.write_head = (self.write_head + 1) % self.ring_len;

            // Fire new grains at the requested density.
            self.spawn_acc += spawn_rate;
            while self.spawn_acc >= 1.0 {
                self.spawn_acc -= 1.0;
                self.spawn(position, ctx.sample_rate);
            }

            // Sum the windowed output of every active grain. Indexed so each buffer read
            // (which borrows `self`) does not clash with mutating the grain's own state.
            let mut acc = 0.0;
            for idx in 0..MAX_GRAINS {
                if !self.grains[idx].active {
                    continue;
                }
                let g = self.grains[idx];
                let progress = g.age / g.length;
                // Hann window: fades each grain in and out to avoid clicks.
                let win = 0.5 * (1.0 - (core::f32::consts::TAU * progress).cos());
                acc += self.read_buffer(g.read_pos) * win;

                let grain = &mut self.grains[idx];
                grain.read_pos += g.rate;
                grain.age += 1.0;
                if grain.age >= grain.length {
                    grain.active = false;
                }
            }
            // Two grains overlap on average with a Hann window; trim to keep unity-ish level.
            if let Some(o) = out.get_mut(n) {
                *o = acc * 0.5;
            }
        }
    }

    fn reset(&mut self) {
        self.buffer.iter_mut().for_each(|s| *s = 0.0);
        self.write_head = 0;
        self.grains = [Grain::default(); MAX_GRAINS];
        self.spawn_acc = 0.0;
        self.rng = 0x9e37_79b9;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(g: &mut Granular, input: &[f32], pos_cv: f32) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: input.len(),
        };
        let cv = vec![pos_cv; input.len()];
        let mut out = vec![0.0; input.len()];
        {
            let ins: [&[f32]; 2] = [input, &cv];
            let mut outs: [&mut [f32]; 1] = [&mut out];
            g.process(&ctx, &ins, &mut outs);
        }
        out
    }

    fn sine(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (core::f32::consts::TAU * hz * n as f32 / 48_000.0).sin())
            .collect()
    }

    fn energy(v: &[f32]) -> f32 {
        v.iter().map(|s| s * s).sum()
    }

    fn brightness(v: &[f32]) -> f32 {
        v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / v.len() as f32
    }

    #[test]
    fn produces_a_finite_audible_cloud() {
        let mut g = Granular::new(80.0, 30.0, 0.2, 1.0, 0.0);
        let out = run(&mut g, &sine(200.0, 24_000), 0.0);
        assert!(out.iter().all(|s| s.is_finite()));
        // After the buffer has filled and grains are flowing, the tail must be audible.
        assert!(
            energy(&out[12_000..]) > 1.0,
            "the grain cloud must produce sound: {}",
            energy(&out[12_000..])
        );
    }

    #[test]
    fn silent_without_input() {
        // Grains reading a silent buffer produce silence.
        let mut g = Granular::new(80.0, 30.0, 0.2, 1.0, 0.0);
        let out = run(&mut g, &vec![0.0; 24_000], 0.0);
        assert_eq!(energy(&out), 0.0, "no input → no grains of sound");
    }

    #[test]
    fn more_density_makes_more_sound() {
        // A denser cloud (more overlapping grains) carries more energy than a sparse one.
        let input = sine(200.0, 24_000);
        let sparse = {
            let mut g = Granular::new(40.0, 5.0, 0.2, 1.0, 0.0);
            energy(&run(&mut g, &input, 0.0)[12_000..])
        };
        let dense = {
            let mut g = Granular::new(40.0, 60.0, 0.2, 1.0, 0.0);
            energy(&run(&mut g, &input, 0.0)[12_000..])
        };
        assert!(
            dense > sparse * 2.0,
            "higher density must add energy: sparse {sparse}, dense {dense}"
        );
    }

    #[test]
    fn pitch_shifts_the_grains_up() {
        // Recording a low sine and playing grains back at 3x reproduces a much brighter
        // (higher-frequency) tone than playing them at unity.
        let input = sine(150.0, 24_000);
        let unity = {
            let mut g = Granular::new(120.0, 40.0, 0.3, 1.0, 0.0);
            brightness(&run(&mut g, &input, 0.0)[12_000..])
        };
        let up = {
            let mut g = Granular::new(120.0, 40.0, 0.3, 3.0, 0.0);
            brightness(&run(&mut g, &input, 0.0)[12_000..])
        };
        assert!(
            up > unity * 1.5,
            "pitching grains up must brighten the output: unity {unity}, up {up}"
        );
    }

    #[test]
    fn reset_clears_the_cloud() {
        let mut g = Granular::new(80.0, 30.0, 0.2, 1.0, 0.0);
        run(&mut g, &sine(200.0, 24_000), 0.0);
        g.reset();
        let out = run(&mut g, &vec![0.0; 12_000], 0.0);
        assert_eq!(energy(&out), 0.0, "reset must empty the buffer and grains");
    }
}
