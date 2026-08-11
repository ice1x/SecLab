//! `vb-nodes` — the DSP node library for VST_bomb.
//!
//! Concrete nodes that implement [`vb_core::Processor`]: oscillators, filters,
//! envelopes, math and routing. Each node is a small, independently tested unit that
//! plugs into the [`vb_core`] graph engine.

pub mod oscillator;

pub use oscillator::{Oscillator, Waveform};
