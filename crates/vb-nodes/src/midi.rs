//! MIDI note input: hardware events in, `gate` / `pitch_hz` / `velocity` signals out.
//!
//! The transport is split from the DSP so the registry can keep building processors
//! from plain factories: a [`NoteSource`] is a lock-free, cloneable mailbox the MIDI
//! thread writes note events into, and [`MidiIn`] is a processor that reads it once
//! per block and emits three control signals. The standalone (or a plugin host
//! adapter) owns one `NoteSource`, feeds every rebuilt engine through the registry's
//! `BuildContext`, and processors across engine swaps all hear the same notes.
//!
//! Monophonic with last-note priority: a new note-on replaces the current note; a
//! note-off only releases the gate when it matches the sounding note, so releasing an
//! older key under a newer one is ignored.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use vb_core::{ProcessContext, Processor};

/// Converts a MIDI note number to its equal-tempered frequency (A4 = 69 = 440 Hz).
#[must_use]
pub fn midi_note_to_hz(note: u8) -> f32 {
    440.0 * ((f32::from(note) - 69.0) / 12.0).exp2()
}

/// Packed note state: `gate << 16 | note << 8 | velocity`.
///
/// One atomic holds the whole state so a block never sees a gate from one event and
/// a velocity from another.
fn pack(gate: bool, note: u8, velocity: u8) -> u32 {
    (u32::from(gate) << 16) | (u32::from(note) << 8) | u32::from(velocity)
}

/// Lock-free, cloneable mailbox for monophonic note events.
#[derive(Clone, Default)]
pub struct NoteSource {
    state: Arc<AtomicU32>,
}

impl NoteSource {
    /// Creates a silent source (gate off).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A key was pressed: the new note takes over (last-note priority).
    pub fn note_on(&self, note: u8, velocity: u8) {
        self.state
            .store(pack(true, note, velocity), Ordering::Release);
    }

    /// A key was released. Only the currently sounding note releases the gate;
    /// releasing an older key under a newer one changes nothing.
    pub fn note_off(&self, note: u8) {
        let _ = self
            .state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                let sounding = ((current >> 8) & 0xff) as u8;
                let gate = current >> 16 != 0;
                (gate && sounding == note).then_some(current & 0xffff)
            });
    }

    /// Current `(gate, note, velocity)`.
    #[must_use]
    pub fn read(&self) -> (bool, u8, u8) {
        let s = self.state.load(Ordering::Acquire);
        (s >> 16 != 0, ((s >> 8) & 0xff) as u8, (s & 0xff) as u8)
    }
}

/// Everything a registry factory may need beyond parameter values.
///
/// Passed to `NodeDescriptor::processor`/`processor_with` so context-dependent nodes
/// (like [`MidiIn`]) can attach to live inputs while pure DSP nodes ignore it.
#[derive(Clone, Default)]
pub struct BuildContext {
    /// The note mailbox `midi-in` nodes listen to.
    pub notes: NoteSource,
}

/// Processor emitting `gate` (0/1), `pitch_hz`, and `velocity` (0..1) control
/// signals from a [`NoteSource`], constant over each block.
///
/// `pitch_hz` holds the last note's frequency after release so envelopes can finish
/// their release phase on the right pitch.
pub struct MidiIn {
    source: NoteSource,
    last_hz: f32,
}

impl MidiIn {
    /// Creates the processor listening to `source`.
    #[must_use]
    pub fn new(source: NoteSource) -> Self {
        Self {
            source,
            last_hz: midi_note_to_hz(69),
        }
    }
}

impl Processor for MidiIn {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (gate, note, velocity) = self.source.read();
        if gate {
            self.last_hz = midi_note_to_hz(note);
        }
        let gate_v = if gate { 1.0 } else { 0.0 };
        let vel_v = f32::from(velocity) / 127.0;
        let [gate_out, pitch_out, vel_out, ..] = outputs else {
            return;
        };
        gate_out[..ctx.frames].fill(gate_v);
        pitch_out[..ctx.frames].fill(self.last_hz);
        vel_out[..ctx.frames].fill(vel_v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_block(p: &mut MidiIn, frames: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
        };
        let (mut g, mut f, mut v) = (vec![0.0; frames], vec![0.0; frames], vec![0.0; frames]);
        p.process(&ctx, &[], &mut [&mut g, &mut f, &mut v]);
        (g, f, v)
    }

    #[test]
    fn note_numbers_map_to_equal_temperament() {
        assert!((midi_note_to_hz(69) - 440.0).abs() < 1e-3);
        assert!((midi_note_to_hz(57) - 220.0).abs() < 1e-3);
        assert!((midi_note_to_hz(81) - 880.0).abs() < 1e-3);
        assert!((midi_note_to_hz(60) - 261.626).abs() < 1e-2, "middle C");
    }

    #[test]
    fn note_on_raises_the_gate_with_pitch_and_velocity() {
        let source = NoteSource::new();
        let mut node = MidiIn::new(source.clone());

        let (gate, _, _) = run_block(&mut node, 16);
        assert!(gate.iter().all(|&s| s == 0.0), "silent before any note");

        source.note_on(69, 127);
        let (gate, pitch, vel) = run_block(&mut node, 16);
        assert!(gate.iter().all(|&s| s == 1.0));
        assert!(pitch.iter().all(|&s| (s - 440.0).abs() < 1e-3));
        assert!(vel.iter().all(|&s| (s - 1.0).abs() < 1e-6));
    }

    #[test]
    fn note_off_releases_but_pitch_holds_for_the_release_tail() {
        let source = NoteSource::new();
        let mut node = MidiIn::new(source.clone());
        source.note_on(81, 64);
        run_block(&mut node, 16);

        source.note_off(81);
        let (gate, pitch, vel) = run_block(&mut node, 16);
        assert!(gate.iter().all(|&s| s == 0.0));
        assert!(
            pitch.iter().all(|&s| (s - 880.0).abs() < 1e-3),
            "pitch persists after release"
        );
        assert!(vel.iter().all(|&s| (s - 64.0 / 127.0).abs() < 1e-6));
    }

    #[test]
    fn last_note_priority_ignores_stale_note_offs() {
        let source = NoteSource::new();
        source.note_on(60, 100);
        source.note_on(64, 100); // second key while the first is held
        source.note_off(60); // releasing the *old* key must not cut the new note
        let (gate, note, _) = source.read();
        assert!(gate);
        assert_eq!(note, 64);

        source.note_off(64);
        assert!(
            !source.read().0,
            "releasing the sounding note drops the gate"
        );
    }
}
