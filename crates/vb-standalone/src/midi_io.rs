//! Hardware MIDI input: midir ports in, [`NoteSource`] events out.
//!
//! The byte-level parsing is pure and unit-tested; the midir plumbing at the bottom
//! is a thin shell around it. The connection callback runs on midir's thread and only
//! does an atomic store into the shared [`NoteSource`] — no locks, no allocation.

use midir::{MidiInput, MidiInputConnection, MidiInputPort};
use vb_nodes::{BuildContext, NoteSource, PolyNoteSource};

/// A note event extracted from a raw MIDI message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteEvent {
    /// Key pressed (velocity 1..=127).
    On {
        /// MIDI note number.
        note: u8,
        /// Strike velocity.
        velocity: u8,
    },
    /// Key released.
    Off {
        /// MIDI note number.
        note: u8,
    },
}

/// Parses one MIDI message into a note event, on any channel.
///
/// Handles the two shapes real keyboards send: note-off (`0x8n`) and note-on
/// (`0x9n`) — including the common "note-on with velocity 0 means note-off"
/// convention. Everything else (CC, pitch bend, clock, …) returns `None`.
#[must_use]
pub fn parse_note_event(message: &[u8]) -> Option<NoteEvent> {
    let (&status, rest) = message.split_first()?;
    let (&note, rest) = rest.split_first()?;
    match (status & 0xf0, rest.first()) {
        (0x80, Some(_)) => Some(NoteEvent::Off { note }),
        (0x90, Some(&0)) => Some(NoteEvent::Off { note }),
        (0x90, Some(&velocity)) => Some(NoteEvent::On { note, velocity }),
        _ => None,
    }
}

/// Feeds a parsed event into the shared note mailbox.
pub fn apply_note_event(notes: &NoteSource, event: NoteEvent) {
    match event {
        NoteEvent::On { note, velocity } => notes.note_on(note, velocity),
        NoteEvent::Off { note } => notes.note_off(note),
    }
}

/// Feeds a parsed event into the polyphonic mailbox.
pub fn apply_poly_event(poly: &PolyNoteSource, event: NoteEvent) {
    match event {
        NoteEvent::On { note, velocity } => {
            poly.note_on(note, velocity);
        }
        NoteEvent::Off { note } => poly.note_off(note),
    }
}

/// Names of the currently available MIDI input ports.
#[must_use]
pub fn list_ports() -> Vec<String> {
    let Ok(input) = MidiInput::new("vb-standalone") else {
        return Vec::new();
    };
    input
        .ports()
        .iter()
        .map(|p| input.port_name(p).unwrap_or_else(|_| "unknown port".into()))
        .collect()
}

/// Connects to the input port at `index` (as listed by [`list_ports`]) and routes
/// its note events into `notes`. Dropping the returned connection disconnects.
pub fn connect_port(index: usize, ctx: BuildContext) -> Result<MidiInputConnection<()>, String> {
    let input = MidiInput::new("vb-standalone").map_err(|e| e.to_string())?;
    let ports = input.ports();
    let port: &MidiInputPort = ports.get(index).ok_or("MIDI port disappeared")?;
    let name = input.port_name(port).unwrap_or_else(|_| "unknown".into());
    input
        .connect(
            port,
            &name,
            move |_timestamp, message, ()| {
                if let Some(event) = parse_note_event(message) {
                    // Mono midi-in nodes and poly voice-in nodes both hear it.
                    apply_note_event(&ctx.notes, event);
                    apply_poly_event(&ctx.poly, event);
                }
            },
            (),
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_on_and_off_parse_on_any_channel() {
        assert_eq!(
            parse_note_event(&[0x90, 69, 100]),
            Some(NoteEvent::On {
                note: 69,
                velocity: 100
            })
        );
        // Channel nibble is irrelevant: channel 5 note-off.
        assert_eq!(
            parse_note_event(&[0x85, 69, 64]),
            Some(NoteEvent::Off { note: 69 })
        );
        assert_eq!(
            parse_note_event(&[0x9f, 12, 1]),
            Some(NoteEvent::On {
                note: 12,
                velocity: 1
            })
        );
    }

    #[test]
    fn note_on_with_zero_velocity_is_a_note_off() {
        assert_eq!(
            parse_note_event(&[0x90, 60, 0]),
            Some(NoteEvent::Off { note: 60 })
        );
    }

    #[test]
    fn non_note_messages_and_garbage_are_ignored() {
        assert_eq!(parse_note_event(&[0xb0, 1, 64]), None, "control change");
        assert_eq!(parse_note_event(&[0xe0, 0, 64]), None, "pitch bend");
        assert_eq!(parse_note_event(&[0xf8]), None, "clock");
        assert_eq!(parse_note_event(&[0x90, 60]), None, "truncated");
        assert_eq!(parse_note_event(&[]), None, "empty");
    }

    #[test]
    fn events_drive_the_poly_source_with_voice_allocation() {
        let poly = PolyNoteSource::new();
        apply_poly_event(
            &poly,
            NoteEvent::On {
                note: 60,
                velocity: 100,
            },
        );
        apply_poly_event(
            &poly,
            NoteEvent::On {
                note: 64,
                velocity: 80,
            },
        );
        assert_eq!(poly.read(0), (true, 60, 100));
        assert_eq!(poly.read(1), (true, 64, 80));
        apply_poly_event(&poly, NoteEvent::Off { note: 60 });
        assert!(!poly.read(0).0);
        assert!(poly.read(1).0);
    }

    #[test]
    fn events_drive_the_note_source() {
        let notes = NoteSource::new();
        apply_note_event(
            &notes,
            NoteEvent::On {
                note: 72,
                velocity: 90,
            },
        );
        assert_eq!(notes.read(), (true, 72, 90));
        apply_note_event(&notes, NoteEvent::Off { note: 72 });
        assert!(!notes.read().0);
    }
}
