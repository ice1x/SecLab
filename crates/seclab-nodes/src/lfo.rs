//! LFO: a control-rate oscillator, free-running in Hz or synced to a tempo.
//!
//! Outputs a bipolar [-1, 1] Control signal; depth and offset are the job of the
//! math nodes downstream (gain/add), keeping this node single-purpose. No
//! band-limiting — at control rates aliasing is inaudible, so shapes are exact.
//!
//! Tempo sync is expressed as beats-per-minute plus a note division; the standalone
//! supplies the BPM as a parameter for now, and the plugin host's transport can feed
//! the same field later (000066).

use seclab_core::{ProcessContext, Processor};

use crate::oscillator::Waveform;

/// Note divisions the sync mode can lock to, in fractions of a whole note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Division {
    /// Whole note (4 beats).
    Whole,
    /// Half note.
    Half,
    /// Quarter note (one beat).
    Quarter,
    /// Eighth note.
    Eighth,
    /// Sixteenth note.
    Sixteenth,
}

impl Division {
    /// Maps a stepped float parameter to a division (0 whole, 1 half, 2 quarter, 3 eighth,
    /// 4 sixteenth), rounding and clamping — the encoding the registry's `division` parameter uses.
    #[must_use]
    pub fn from_stepped(v: f32) -> Self {
        match v.round() as i32 {
            0 => Division::Whole,
            1 => Division::Half,
            3 => Division::Eighth,
            4 => Division::Sixteenth,
            _ => Division::Quarter,
        }
    }

    /// Cycle frequency in Hz for this division at `bpm` (a beat is a quarter note).
    #[must_use]
    pub fn hz(self, bpm: f32) -> f32 {
        let beats_per_cycle = match self {
            Self::Whole => 4.0,
            Self::Half => 2.0,
            Self::Quarter => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
        };
        bpm / 60.0 / beats_per_cycle
    }
}

/// Low-frequency oscillator emitting one bipolar control channel.
pub struct Lfo {
    waveform: Waveform,
    freq_hz: f32,
    /// Phase in cycles, wrapped to [0, 1).
    phase: f32,
}

impl Lfo {
    /// Free-running LFO at `freq_hz`.
    #[must_use]
    pub fn new(waveform: Waveform, freq_hz: f32) -> Self {
        Self {
            waveform,
            freq_hz,
            phase: 0.0,
        }
    }

    /// Tempo-synced LFO: one cycle per `division` at `bpm`.
    #[must_use]
    pub fn synced(waveform: Waveform, bpm: f32, division: Division) -> Self {
        Self::new(waveform, division.hz(bpm))
    }

    /// The effective cycle frequency in Hz.
    #[must_use]
    pub fn freq_hz(&self) -> f32 {
        self.freq_hz
    }

    /// Sample value at phase `p` in [0, 1).
    fn shape(&self, p: f32) -> f32 {
        match self.waveform {
            Waveform::Sine => (std::f32::consts::TAU * p).sin(),
            Waveform::Saw => 2.0f32.mul_add(p, -1.0),
            Waveform::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => {
                if p < 0.5 {
                    4.0f32.mul_add(p, -1.0)
                } else {
                    4.0f32.mul_add(-p, 3.0)
                }
            }
        }
    }
}

impl Processor for Lfo {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(out) = outputs.first_mut() else {
            return;
        };
        let step = if ctx.sample_rate > 0.0 {
            self.freq_hz / ctx.sample_rate
        } else {
            0.0
        };
        for o in out.iter_mut().take(ctx.frames) {
            *o = self.shape(self.phase);
            self.phase += step;
            if self.phase >= 1.0 {
                self.phase -= self.phase.floor();
            }
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Live-updates the LFO from `waveform`, `sync`, `freq_hz`, `bpm`, `division` (registry order),
    /// preserving the phase so the modulation keeps running. In sync mode the effective frequency
    /// is re-derived from `bpm` + `division`; otherwise it is the `freq_hz` parameter — mirroring
    /// how the registry builds a free vs synced LFO.
    fn set_params(&mut self, values: &[f32]) -> bool {
        if let Some(&v) = values.first() {
            self.waveform = Waveform::from_stepped(v);
        }
        let synced = values.get(1).copied().unwrap_or(0.0).round() as i32 == 1;
        if synced {
            let bpm = values.get(3).copied().unwrap_or(120.0);
            let division = Division::from_stepped(values.get(4).copied().unwrap_or(2.0));
            self.freq_hz = division.hz(bpm);
        } else if let Some(&v) = values.get(2) {
            self.freq_hz = v;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn render(lfo: &mut Lfo, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: SR,
            frames,
        };
        let mut out = vec![0.0; frames];
        lfo.process(&ctx, &[], &mut [&mut out]);
        out
    }

    fn upward_zero_crossings(xs: &[f32]) -> usize {
        xs.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn sine_cycles_at_the_requested_frequency() {
        let mut lfo = Lfo::new(Waveform::Sine, 5.0);
        let out = render(&mut lfo, SR as usize); // one second
        assert_eq!(upward_zero_crossings(&out), 5);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(*s));
        let trough = out.iter().fold(0.0f32, |m, s| m.min(*s));
        assert!(peak > 0.99 && trough < -0.99, "full bipolar swing");
    }

    #[test]
    fn tempo_sync_matches_the_equivalent_free_frequency() {
        // 120 BPM quarters = 2 Hz; eighths = 4 Hz; whole notes = 0.5 Hz.
        assert!((Division::Quarter.hz(120.0) - 2.0).abs() < 1e-6);
        assert!((Division::Eighth.hz(120.0) - 4.0).abs() < 1e-6);
        assert!((Division::Whole.hz(120.0) - 0.5).abs() < 1e-6);
        assert!((Division::Sixteenth.hz(90.0) - 6.0).abs() < 1e-6);

        let mut synced = Lfo::synced(Waveform::Sine, 120.0, Division::Eighth);
        let mut free = Lfo::new(Waveform::Sine, 4.0);
        assert_eq!(render(&mut synced, 4_800), render(&mut free, 4_800));
    }

    #[test]
    fn square_and_triangle_shapes_behave() {
        let mut square = Lfo::new(Waveform::Square, 10.0);
        let s = render(&mut square, SR as usize / 10); // exactly one cycle
        assert!(s.iter().all(|&v| v == 1.0 || v == -1.0));
        let highs = s.iter().filter(|&&v| v == 1.0).count();
        assert!(
            (highs as f32 / s.len() as f32 - 0.5).abs() < 0.01,
            "50% duty"
        );

        let mut triangle = Lfo::new(Waveform::Triangle, 10.0);
        let t = render(&mut triangle, SR as usize / 10);
        assert!(t.iter().all(|&v| (-1.0..=1.0).contains(&v)));
        // Continuous: per-sample step of a 10 Hz triangle at 48 kHz is 4*f/sr.
        let max_step = 4.0 * 10.0 / SR * 1.5;
        assert!(t
            .windows(2)
            .all(|w| (w[1] - w[0]).abs() <= max_step || (w[1] - w[0]).abs() > 1.9));
    }

    #[test]
    fn reset_restarts_the_cycle() {
        let mut lfo = Lfo::new(Waveform::Saw, 3.0);
        let first = render(&mut lfo, 1_000);
        lfo.reset();
        assert_eq!(render(&mut lfo, 1_000), first);
    }
}
