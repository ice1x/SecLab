//! Browser audio binding (backlog 000083).
//!
//! `WasmSynth` is the object a WebAudio `AudioWorkletProcessor` drives from JS: build
//! it from a patch, push MIDI note on/off from the main thread, and pull one render
//! quantum per `process()` call. It is a thin shim over [`HeadlessSynth`] — all the
//! DSP and every test live there — so this file exists only to cross the JS boundary
//! and compiles solely for `wasm32`.

use wasm_bindgen::prelude::*;

use crate::headless::HeadlessSynth;

/// A running instrument exposed to the browser's audio thread.
#[wasm_bindgen]
pub struct WasmSynth {
    inner: HeadlessSynth,
}

#[wasm_bindgen]
impl WasmSynth {
    /// Builds the embedded default patch at `sample_rate` (the `AudioContext` rate).
    ///
    /// # Errors
    ///
    /// Throws a JS error if the default patch fails to compile.
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: f32) -> Result<WasmSynth, JsError> {
        HeadlessSynth::with_default_patch(sample_rate)
            .map(|inner| WasmSynth { inner })
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Builds a synth from a `.vbp` RON patch string.
    ///
    /// # Errors
    ///
    /// Throws a JS error if the RON does not parse/validate or cannot be compiled.
    #[wasm_bindgen(js_name = fromRon)]
    pub fn from_ron(sample_rate: f32, ron: &str) -> Result<WasmSynth, JsError> {
        HeadlessSynth::from_ron(sample_rate, ron)
            .map(|inner| WasmSynth { inner })
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Presses a MIDI note (call from the main thread as keys/notes arrive).
    #[wasm_bindgen(js_name = noteOn)]
    pub fn note_on(&self, note: u8, velocity: u8) {
        self.inner.note_on(note, velocity);
    }

    /// Releases a MIDI note.
    #[wasm_bindgen(js_name = noteOff)]
    pub fn note_off(&self, note: u8) {
        self.inner.note_off(note);
    }

    /// Renders one `AudioWorklet` quantum of mono audio ([`block_size`] samples).
    ///
    /// The returned `Float32Array` is copied straight into the worklet's output
    /// channel. This allocates per call — acceptable for the prototype; a
    /// zero-copy shared buffer is the follow-up once the path is proven end to end.
    ///
    /// [`block_size`]: WasmSynth::block_size
    pub fn render(&mut self) -> Vec<f32> {
        let mut out = vec![0.0; HeadlessSynth::BLOCK];
        self.inner.render_block(&mut out);
        out
    }

    /// The fixed render-quantum size, in samples (matches WebAudio's 128).
    #[wasm_bindgen(js_name = blockSize)]
    #[must_use]
    pub fn block_size() -> usize {
        HeadlessSynth::BLOCK
    }
}
