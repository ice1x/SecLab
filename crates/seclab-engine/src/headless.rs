//! [`HeadlessSynth`] — a self-contained, real-time-shaped instrument driver.
//!
//! It owns the registry, the build context (the live note source), the compiled
//! engine and its sink, and renders audio one fixed-size block at a time. It is the
//! substance behind the browser's `AudioWorklet` binding (see [`crate::wasm`]) but
//! carries nothing platform-specific, so it is fully exercised by native unit tests.

use seclab_core::{NodeId, ProcessContext};
use seclab_nodes::{BuildContext, Registry};
use seclab_runtime::RtEngine;

use crate::builder::{default_patch, engine_from_doc, BuildError};

/// A patch compiled into a running voice, driven block by block.
///
/// The block size is fixed at [`HeadlessSynth::BLOCK`] — one WebAudio render quantum —
/// so [`render_block`](HeadlessSynth::render_block) maps one-to-one onto an
/// `AudioWorkletProcessor::process` call.
pub struct HeadlessSynth {
    // Kept alive for the engine's lifetime: the note source lives inside `ctx`, and
    // MIDI events are injected through it (interior mutability), so the running engine
    // observes them on the next block.
    ctx: BuildContext,
    engine: RtEngine,
    sink: Option<NodeId>,
    sample_rate: f32,
}

impl HeadlessSynth {
    /// The render-block size, in samples: one WebAudio `AudioWorklet` quantum.
    pub const BLOCK: usize = 128;

    /// Builds the embedded [`default_patch`] at `sample_rate`.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError`] if the default patch fails to compile (should not happen
    /// with the built-in registry).
    pub fn with_default_patch(sample_rate: f32) -> Result<Self, BuildError> {
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        Self::from_doc_and_registry(sample_rate, &doc, &registry)
    }

    /// Builds the patch described by a `.vbp` RON string at `sample_rate`.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::Project`] if the RON does not parse or validate, or any
    /// other [`BuildError`] if the resulting document cannot be compiled.
    pub fn from_ron(sample_rate: f32, ron: &str) -> Result<Self, BuildError> {
        let registry = Registry::with_builtins();
        let doc = seclab_project::open_ron(ron).map_err(|e| BuildError::Project(e.to_string()))?;
        Self::from_doc_and_registry(sample_rate, &doc, &registry)
    }

    fn from_doc_and_registry(
        sample_rate: f32,
        doc: &seclab_project::ProjectDoc,
        registry: &Registry,
    ) -> Result<Self, BuildError> {
        let ctx = BuildContext::default();
        let (engine, sink) = engine_from_doc(doc, registry, &ctx, Self::BLOCK)?;
        Ok(Self {
            ctx,
            engine,
            sink,
            sample_rate,
        })
    }

    /// Starts a note. Both the monophonic and polyphonic note sources are fed, so the
    /// same call drives mono and poly patches alike.
    pub fn note_on(&self, note: u8, velocity: u8) {
        self.ctx.notes.note_on(note, velocity);
        self.ctx.poly.note_on(note, velocity);
    }

    /// Releases a note previously started with [`note_on`](HeadlessSynth::note_on).
    pub fn note_off(&self, note: u8) {
        self.ctx.notes.note_off(note);
        self.ctx.poly.note_off(note);
    }

    /// Renders exactly one [`BLOCK`](HeadlessSynth::BLOCK)-sample mono block into `out`.
    ///
    /// `out` must be at least [`BLOCK`](HeadlessSynth::BLOCK) samples long; only the
    /// first `BLOCK` are written. A patch with no terminal audio node writes silence.
    pub fn render_block(&mut self, out: &mut [f32]) {
        let block = Self::BLOCK.min(out.len());
        self.engine.process(&ProcessContext {
            sample_rate: self.sample_rate,
            frames: Self::BLOCK,
        });
        let dst = &mut out[..block];
        match self
            .sink
            .and_then(|s| self.engine.node_output(s))
            .and_then(|buf| buf.channel(0))
        {
            Some(src) => {
                let n = block.min(src.len());
                dst[..n].copy_from_slice(&src[..n]);
                dst[n..].fill(0.0);
            }
            None => dst.fill(0.0),
        }
    }

    /// The sample rate this synth was built for.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_patch_is_silent_until_a_note_plays() {
        let mut synth = HeadlessSynth::with_default_patch(48_000.0).expect("builds");
        let mut out = [0.0_f32; HeadlessSynth::BLOCK];
        synth.render_block(&mut out);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            out.iter().all(|&s| s.abs() < 1e-6),
            "a fresh synth with no note held must be silent"
        );
    }

    #[test]
    fn default_patch_becomes_audible_on_a_note() {
        let mut synth = HeadlessSynth::with_default_patch(48_000.0).expect("builds");
        synth.note_on(69, 127);
        // Let the envelope open across a few blocks.
        let mut out = [0.0_f32; HeadlessSynth::BLOCK];
        let mut peak = 0.0_f32;
        for _ in 0..8 {
            synth.render_block(&mut out);
            peak = peak.max(out.iter().fold(0.0_f32, |m, &s| m.max(s.abs())));
        }
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(peak > 0.001, "a held note must make the synth audible");
    }

    #[test]
    fn a_released_note_returns_to_silence() {
        let mut synth = HeadlessSynth::with_default_patch(48_000.0).expect("builds");
        synth.note_on(69, 127);
        let mut out = [0.0_f32; HeadlessSynth::BLOCK];
        for _ in 0..8 {
            synth.render_block(&mut out);
        }
        synth.note_off(69);
        // The amp envelope releases; after enough blocks the tail decays to near silence.
        let mut peak = f32::INFINITY;
        for _ in 0..400 {
            synth.render_block(&mut out);
            peak = out.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        }
        assert!(
            peak < 1e-3,
            "after release the voice must decay back toward silence (peak was {peak})"
        );
    }

    #[test]
    fn builds_from_a_ron_patch_string() {
        // The exact browser path: a `.vbp` RON string → a running synth.
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let ron = seclab_project::to_ron(&doc).expect("serializes");
        let mut synth = HeadlessSynth::from_ron(44_100.0, &ron).expect("builds from RON");
        assert!((synth.sample_rate() - 44_100.0).abs() < f32::EPSILON);
        synth.note_on(60, 100);
        let mut out = [0.0_f32; HeadlessSynth::BLOCK];
        let mut peak = 0.0_f32;
        for _ in 0..8 {
            synth.render_block(&mut out);
            peak = peak.max(out.iter().fold(0.0_f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.001, "the RON-loaded patch must play");
    }

    #[test]
    fn malformed_ron_is_reported_not_panicked() {
        let err = HeadlessSynth::from_ron(48_000.0, "this is not ron");
        assert!(matches!(err, Err(BuildError::Project(_))));
    }

    #[test]
    fn render_block_fills_the_whole_quantum() {
        // A sentinel-filled buffer must be fully overwritten (silence included).
        let mut synth = HeadlessSynth::with_default_patch(48_000.0).expect("builds");
        let mut out = [42.0_f32; HeadlessSynth::BLOCK];
        synth.render_block(&mut out);
        assert!(
            out.iter().all(|&s| s == 0.0),
            "an idle synth must overwrite the entire block with silence"
        );
    }
}
