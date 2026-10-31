//! Polyphony: voice allocation plus per-voice note sources.
//!
//! The patch-level model: the user (or a macro) builds one voice chain per voice and
//! feeds each from a `voice-in` node with a different `voice` index; a merger sums
//! them. [`PolyNoteSource`] is the shared mailbox — the MIDI thread allocates voices
//! and writes per-voice slots (one packed atomic each, same layout as the mono
//! [`crate::midi::NoteSource`]), the audio thread only reads.
//!
//! Allocation policy: retrigger the voice already playing the note, else take a free
//! voice, else steal the oldest sounding one (by strike order).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use seclab_core::{ProcessContext, Processor};

use crate::midi::midi_note_to_hz;

/// Number of voice slots a [`PolyNoteSource`] carries.
pub const MAX_VOICES: usize = 8;

/// Pure voice-assignment bookkeeping (single-threaded, lives behind the source's
/// mutex on the MIDI thread).
#[derive(Default)]
pub struct VoiceAllocator {
    /// The note each voice is sounding, if any.
    notes: [Option<u8>; MAX_VOICES],
    /// Strike order: larger = more recent. Steals take the smallest.
    serials: [u64; MAX_VOICES],
    next_serial: u64,
}

impl VoiceAllocator {
    /// Creates an allocator with every voice free.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Assigns a voice for `note`: the voice already playing it (retrigger), else a
    /// free voice, else the oldest sounding voice (stolen).
    pub fn note_on(&mut self, note: u8) -> usize {
        let voice = self
            .notes
            .iter()
            .position(|n| *n == Some(note))
            .or_else(|| self.notes.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                // All busy: steal the least recently struck.
                self.serials
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, s)| **s)
                    .map_or(0, |(i, _)| i)
            });
        self.notes[voice] = Some(note);
        self.serials[voice] = self.next_serial;
        self.next_serial += 1;
        voice
    }

    /// Releases the voice sounding `note`, returning its index.
    pub fn note_off(&mut self, note: u8) -> Option<usize> {
        let voice = self.notes.iter().position(|n| *n == Some(note))?;
        self.notes[voice] = None;
        Some(voice)
    }
}

/// Packed per-voice state: `gate << 16 | note << 8 | velocity`.
fn pack(gate: bool, note: u8, velocity: u8) -> u32 {
    (u32::from(gate) << 16) | (u32::from(note) << 8) | u32::from(velocity)
}

struct PolyShared {
    slots: [AtomicU32; MAX_VOICES],
    alloc: Mutex<VoiceAllocator>,
}

/// Lock-free-readable polyphonic note mailbox.
///
/// Writers (`note_on`/`note_off`) run on the MIDI thread and take a short internal
/// mutex for allocation bookkeeping; readers ([`PolyVoiceIn`]) only load atomics.
#[derive(Clone)]
pub struct PolyNoteSource {
    shared: Arc<PolyShared>,
}

impl Default for PolyNoteSource {
    fn default() -> Self {
        Self {
            shared: Arc::new(PolyShared {
                slots: std::array::from_fn(|_| AtomicU32::new(0)),
                alloc: Mutex::new(VoiceAllocator::new()),
            }),
        }
    }
}

impl PolyNoteSource {
    /// Creates a silent source with all voices free.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A key was pressed: allocates a voice and returns its index.
    pub fn note_on(&self, note: u8, velocity: u8) -> usize {
        let voice = match self.shared.alloc.lock() {
            Ok(mut alloc) => alloc.note_on(note),
            Err(_) => 0,
        };
        self.shared.slots[voice].store(pack(true, note, velocity), Ordering::Release);
        voice
    }

    /// A key was released: drops the gate of the voice sounding `note`.
    pub fn note_off(&self, note: u8) {
        let voice = match self.shared.alloc.lock() {
            Ok(mut alloc) => alloc.note_off(note),
            Err(_) => None,
        };
        if let Some(voice) = voice {
            let current = self.shared.slots[voice].load(Ordering::Acquire);
            self.shared.slots[voice].store(current & 0xffff, Ordering::Release);
        }
    }

    /// Current `(gate, note, velocity)` of `voice` (out of range reads silence).
    #[must_use]
    pub fn read(&self, voice: usize) -> (bool, u8, u8) {
        let s = self
            .shared
            .slots
            .get(voice)
            .map_or(0, |slot| slot.load(Ordering::Acquire));
        (s >> 16 != 0, ((s >> 8) & 0xff) as u8, (s & 0xff) as u8)
    }

    /// Whether `note` is currently held (gate on) in any voice. This is the note-addressed
    /// view a drum kit needs: each drum listens for its own key, and because voices are
    /// polyphonic, several keys (kick + hat) can be held at once.
    #[must_use]
    pub fn note_gate(&self, note: u8) -> bool {
        self.shared.slots.iter().any(|slot| {
            let s = slot.load(Ordering::Acquire);
            s >> 16 != 0 && ((s >> 8) & 0xff) as u8 == note
        })
    }
}

/// A per-note gate: emits `1.0` while a specific MIDI note is held in any voice, else
/// `0.0`. Wire one per drum (kick on 36, snare on 38, …) to trigger each voice's
/// envelope independently, so a kit plays polyphonically off one note stream.
pub struct KeyGate {
    source: PolyNoteSource,
    note: u8,
}

impl KeyGate {
    /// Creates a gate that fires while `note` is held in `source`.
    #[must_use]
    pub fn new(source: PolyNoteSource, note: u8) -> Self {
        Self { source, note }
    }
}

impl Processor for KeyGate {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let Some(out) = outputs.first_mut() else {
            return;
        };
        let g = if self.source.note_gate(self.note) {
            1.0
        } else {
            0.0
        };
        out[..ctx.frames].fill(g);
    }
}

/// Per-voice counterpart of [`crate::midi::MidiIn`]: emits `gate` / `pitch_hz` /
/// `velocity` control signals for one voice slot, constant over each block.
pub struct PolyVoiceIn {
    source: PolyNoteSource,
    voice: usize,
    last_hz: f32,
}

impl PolyVoiceIn {
    /// Creates the processor reading `voice` from `source`.
    #[must_use]
    pub fn new(source: PolyNoteSource, voice: usize) -> Self {
        Self {
            source,
            voice,
            last_hz: midi_note_to_hz(69),
        }
    }
}

impl Processor for PolyVoiceIn {
    fn process(&mut self, ctx: &ProcessContext, _inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let (gate, note, velocity) = self.source.read(self.voice);
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

    #[test]
    fn allocator_fills_free_voices_then_steals_the_oldest() {
        let mut a = VoiceAllocator::new();
        // 8 distinct notes take 8 distinct voices.
        let voices: Vec<usize> = (0..MAX_VOICES as u8).map(|n| a.note_on(60 + n)).collect();
        let mut sorted = voices.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..MAX_VOICES).collect::<Vec<_>>());

        // A ninth note steals voice of the FIRST (oldest) note.
        let stolen = a.note_on(90);
        assert_eq!(stolen, voices[0]);

        // The stolen note is gone: releasing it finds nothing.
        assert_eq!(a.note_off(60), None);
        assert_eq!(a.note_off(90), Some(stolen));
    }

    #[test]
    fn allocator_retriggers_the_voice_already_playing_a_note() {
        let mut a = VoiceAllocator::new();
        let v1 = a.note_on(64);
        let v2 = a.note_on(64);
        assert_eq!(v1, v2, "same note retriggers, no duplicate voice");
        assert_eq!(a.note_off(64), Some(v1));
        assert_eq!(a.note_off(64), None, "already released");
    }

    #[test]
    fn chords_land_in_separate_slots_and_release_independently() {
        let source = PolyNoteSource::new();
        let va = source.note_on(60, 100);
        let vb = source.note_on(64, 90);
        let vc = source.note_on(67, 80);
        assert_ne!(va, vb);
        assert_ne!(vb, vc);

        assert_eq!(source.read(va), (true, 60, 100));
        assert_eq!(source.read(vb), (true, 64, 90));
        assert_eq!(source.read(vc), (true, 67, 80));

        source.note_off(64);
        assert!(!source.read(vb).0, "released voice gates off");
        assert!(
            source.read(va).0 && source.read(vc).0,
            "others keep sounding"
        );
    }

    #[test]
    fn note_gate_reports_each_key_independently() {
        let source = PolyNoteSource::new();
        assert!(!source.note_gate(36), "nothing held yet");
        source.note_on(36, 100); // kick
        source.note_on(42, 90); // hat, held at the same time
        assert!(source.note_gate(36), "kick key held");
        assert!(source.note_gate(42), "hat key held");
        assert!(!source.note_gate(38), "snare key not held");
        source.note_off(36);
        assert!(!source.note_gate(36), "kick released");
        assert!(source.note_gate(42), "hat still held");
    }

    #[test]
    fn key_gate_node_fires_only_for_its_note() {
        let source = PolyNoteSource::new();
        let mut kick = KeyGate::new(source.clone(), 36);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: 4,
        };
        let run = |node: &mut KeyGate| {
            let mut out = [0.0_f32; 4];
            node.process(&ctx, &[], &mut [&mut out]);
            out[0]
        };
        assert_eq!(run(&mut kick), 0.0, "silent before any note");
        source.note_on(38, 100); // snare, not kick
        assert_eq!(run(&mut kick), 0.0, "wrong note does not open the gate");
        source.note_on(36, 100);
        assert_eq!(run(&mut kick), 1.0, "kick note opens the gate");
        source.note_off(36);
        assert_eq!(run(&mut kick), 0.0, "gate closes on release");
    }

    #[test]
    fn voice_in_nodes_track_their_own_slot() {
        let source = PolyNoteSource::new();
        let mut voice0 = PolyVoiceIn::new(source.clone(), 0);
        let mut voice1 = PolyVoiceIn::new(source.clone(), 1);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: 8,
        };
        let run = |node: &mut PolyVoiceIn| {
            let (mut g, mut f, mut v) = ([0.0_f32; 8], [0.0_f32; 8], [0.0_f32; 8]);
            node.process(&ctx, &[], &mut [&mut g, &mut f, &mut v]);
            (g[0], f[0], v[0])
        };

        let v_low = source.note_on(57, 127); // A3 = 220 Hz
        let v_high = source.note_on(81, 64); // A5 = 880 Hz
        assert_eq!((v_low, v_high), (0, 1), "first two voices in order");

        let (g0, f0, _) = run(&mut voice0);
        let (g1, f1, vel1) = run(&mut voice1);
        assert_eq!(g0, 1.0);
        assert!((f0 - 220.0).abs() < 1e-3);
        assert_eq!(g1, 1.0);
        assert!((f1 - 880.0).abs() < 1e-3);
        assert!((vel1 - 64.0 / 127.0).abs() < 1e-6);

        // Release the low note: voice0 gates off but holds pitch; voice1 unaffected.
        source.note_off(57);
        let (g0, f0, _) = run(&mut voice0);
        assert_eq!(g0, 0.0);
        assert!(
            (f0 - 220.0).abs() < 1e-3,
            "pitch holds for the release tail"
        );
        assert_eq!(run(&mut voice1).0, 1.0);
    }
}
