//! Hard-sync oscillator: a slave saw force-reset by a master at the played pitch.
//!
//! Two sawtooths in one node. The *master* runs at the tracked note pitch; the *slave* runs
//! faster, at `sync_ratio ×` the master, but every time the master completes a cycle it
//! **hard-resets the slave's phase to zero**. That forced restart mid-cycle injects a
//! bright, tearing formant whose position follows `sync_ratio` — sweeping the ratio is the
//! aggressive "sync lead" sound. At `sync_ratio = 1` the slave and master coincide and it is
//! a plain saw. Tracks a `pitch_hz` control input like [`crate::PitchOsc`].

use seclab_core::{ProcessContext, Processor};

/// Lowest and highest master frequency (Hz).
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

/// A pitch-tracking hard-sync sawtooth pair.
///
/// Input 0 is `pitch_hz` (falls back to `base_hz`). Output 0 is the slave saw, reset each
/// master cycle. Params: `base_hz`, `sync_ratio`.
#[derive(Debug, Clone)]
pub struct HardSync {
    base_hz: f32,
    sync_ratio: f32,
    freq_hz: f32,
    master_phase: f32,
    slave_phase: f32,
}

impl HardSync {
    /// Creates a hard-sync oscillator resting at `base_hz` with the slave at `sync_ratio`
    /// times the master (clamped to `>= 1`).
    #[must_use]
    pub fn new(base_hz: f32, sync_ratio: f32) -> Self {
        Self {
            base_hz,
            sync_ratio: sync_ratio.max(1.0),
            freq_hz: base_hz,
            master_phase: 0.0,
            slave_phase: 0.0,
        }
    }

    /// The master frequency currently in effect (Hz).
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.freq_hz
    }
}

impl Processor for HardSync {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let sr = ctx.sample_rate.max(1.0);
        let pitch = inputs
            .first()
            .and_then(|c| c.first())
            .copied()
            .unwrap_or(0.0);
        let tracked = if pitch >= MIN_HZ { pitch } else { self.base_hz };
        self.freq_hz = tracked.clamp(MIN_HZ, MAX_HZ);
        let master_dt = self.freq_hz / sr;
        let slave_dt = master_dt * self.sync_ratio;

        for out in outputs[0].iter_mut().take(ctx.frames) {
            *out = 2.0 * self.slave_phase - 1.0; // slave saw in [-1, 1]
            self.slave_phase += slave_dt;
            if self.slave_phase >= 1.0 {
                self.slave_phase -= 1.0;
            }
            self.master_phase += master_dt;
            if self.master_phase >= 1.0 {
                self.master_phase -= 1.0;
                self.slave_phase = 0.0; // hard sync: master cycle resets the slave
            }
        }
    }

    fn reset(&mut self) {
        self.master_phase = 0.0;
        self.slave_phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(osc: &mut HardSync, pitch: Option<f32>, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        let pitch_buf = pitch.map(|p| vec![p; frames]);
        let ins: Vec<&[f32]> = pitch_buf
            .as_ref()
            .map(|b| vec![b.as_slice()])
            .unwrap_or_default();
        {
            let mut slice = [&mut out[..]];
            osc.process(&ctx, &ins, &mut slice);
        }
        out
    }

    fn zero_crossings(xs: &[f32]) -> usize {
        xs.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    }

    #[test]
    fn ratio_one_is_a_plain_saw() {
        // sync_ratio 1: slave == master, a plain saw at 220 Hz. Over 4800 samples at 48 kHz
        // that is ~22 cycles, and a saw crosses zero twice per cycle (the ramp through 0 and
        // the wrap discontinuity), so ~44 crossings.
        let out = render(&mut HardSync::new(220.0, 1.0), Some(220.0), 4_800);
        let c = zero_crossings(&out);
        assert!((40..=46).contains(&c), "plain saw ≈ 44 crossings, got {c}");
        assert!(out.iter().all(|s| s.abs() <= 1.0 + 1e-6));
    }

    #[test]
    fn a_higher_ratio_adds_harmonic_content() {
        // sync_ratio 3 runs the slave 3× faster → more crossings than ratio 1, but still
        // locked to the master period.
        let one = zero_crossings(&render(&mut HardSync::new(220.0, 1.0), Some(220.0), 4_800));
        let three = zero_crossings(&render(&mut HardSync::new(220.0, 3.0), Some(220.0), 4_800));
        assert!(three > one, "sync tearing adds crossings: {one} vs {three}");
    }

    #[test]
    fn the_output_is_periodic_at_the_master_pitch() {
        // Whatever the sync ratio, the waveform repeats at the master period. At 480 Hz over
        // 9600 samples (= 96 master cycles) the second half matches the first cycle-for-cycle
        // in gross shape: compare block RMS of the two halves.
        let out = render(&mut HardSync::new(100.0, 2.5), Some(480.0), 9_600);
        let rms = |xs: &[f32]| (xs.iter().map(|s| s * s).sum::<f32>() / xs.len() as f32).sqrt();
        let a = rms(&out[..4_800]);
        let b = rms(&out[4_800..]);
        assert!((a - b).abs() < 0.05, "steady periodic energy: {a} vs {b}");
    }

    #[test]
    fn it_tracks_pitch_and_clamps_ratio() {
        let low = zero_crossings(&render(&mut HardSync::new(100.0, 2.0), Some(200.0), 4_800));
        let high = zero_crossings(&render(&mut HardSync::new(100.0, 2.0), Some(800.0), 4_800));
        assert!(high > low * 2, "4× pitch → more crossings: {low} vs {high}");
        assert!(
            (HardSync::new(220.0, 0.2).sync_ratio - 1.0).abs() < 1e-6,
            "ratio clamped to >= 1"
        );
    }
}
