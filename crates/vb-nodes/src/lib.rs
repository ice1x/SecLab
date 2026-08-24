//! `vb-nodes` — the DSP node library for VST_bomb.
//!
//! Concrete nodes that implement [`vb_core::Processor`]: oscillators, filters,
//! envelopes, math and routing. Each node is a small, independently tested unit that
//! plugs into the [`vb_core`] graph engine.

pub mod ad_env;
pub mod cards;
pub mod delay;
pub mod docs;
pub mod env_follower;
pub mod envelope;
pub mod filter;
pub mod fm_op;
pub mod glide;
pub mod io;
pub mod ladder;
pub mod lfo;
pub mod math;
pub mod midi;
pub mod noise;
pub mod oscillator;
pub mod pitch_osc;
pub mod poly;
pub mod registry;
pub mod reverb;
pub mod routing;
pub mod smooth;
pub mod supersaw;
pub mod svf;
pub mod vca;
pub mod vcf;
pub mod vco;
pub mod waveshaper;

pub use ad_env::AdEnv;
pub use cards::{node_card, node_reference};
pub use delay::{FbDelay, MAX_DELAY_S};
pub use docs::node_reference_markdown;
pub use env_follower::EnvFollower;
pub use envelope::Adsr;
pub use filter::{Biquad, FilterKind};
pub use fm_op::FmOp;
pub use glide::Glide;
pub use io::{AudioBus, AudioInput, AudioOutput, MAX_IO_CHANNELS};
pub use ladder::Ladder;
pub use lfo::{Division, Lfo};
pub use math::{Add, Clamp, Gain, Mix, Mul};
pub use midi::{midi_note_to_hz, BuildContext, MidiIn, NoteSource};
pub use noise::{Noise, NoiseKind};
pub use oscillator::{Oscillator, Waveform};
pub use pitch_osc::PitchOsc;
pub use poly::{KeyGate, PolyNoteSource, PolyVoiceIn, VoiceAllocator, MAX_VOICES};
pub use registry::{NodeDescriptor, Registry};
pub use reverb::Reverb;
pub use routing::{Merger, Selector, Splitter};
pub use smooth::{ramp_samples, Smoother};
pub use supersaw::Supersaw;
pub use svf::Svf;
pub use vca::Vca;
pub use vcf::Vcf;
pub use vco::Vco;
pub use waveshaper::{Shape, Waveshaper};
