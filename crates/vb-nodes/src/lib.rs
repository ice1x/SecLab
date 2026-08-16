//! `vb-nodes` — the DSP node library for VST_bomb.
//!
//! Concrete nodes that implement [`vb_core::Processor`]: oscillators, filters,
//! envelopes, math and routing. Each node is a small, independently tested unit that
//! plugs into the [`vb_core`] graph engine.

pub mod delay;
pub mod envelope;
pub mod filter;
pub mod lfo;
pub mod math;
pub mod midi;
pub mod noise;
pub mod oscillator;
pub mod poly;
pub mod registry;
pub mod reverb;
pub mod routing;
pub mod svf;
pub mod waveshaper;

pub use delay::{FbDelay, MAX_DELAY_S};
pub use envelope::Adsr;
pub use filter::{Biquad, FilterKind};
pub use lfo::{Division, Lfo};
pub use math::{Add, Clamp, Gain, Mix, Mul};
pub use midi::{midi_note_to_hz, BuildContext, MidiIn, NoteSource};
pub use noise::{Noise, NoiseKind};
pub use oscillator::{Oscillator, Waveform};
pub use poly::{PolyNoteSource, PolyVoiceIn, VoiceAllocator, MAX_VOICES};
pub use registry::{NodeDescriptor, Registry};
pub use reverb::Reverb;
pub use routing::{Merger, Selector, Splitter};
pub use svf::Svf;
pub use waveshaper::{Shape, Waveshaper};
