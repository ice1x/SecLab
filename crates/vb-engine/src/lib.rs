//! `vb-engine` — turn a serialized patch into a runnable audio engine.
//!
//! This is the plugin's and the browser's shared instrument core, split out of
//! `vb-plugin` so it carries **no** host-framework dependency (no `nih_plug`). A
//! [`ProjectDoc`](vb_project::ProjectDoc) — loaded from a `.vbp` RON file or the
//! embedded [`default_patch`] — becomes an allocation-free
//! [`RtEngine`](vb_runtime::RtEngine) with processors from the registry.
//!
//! Because the crate depends only on the DSP core (`vb-core`, `vb-nodes`,
//! `vb-project`, `vb-runtime`), it compiles cleanly to `wasm32-unknown-unknown`.
//! [`HeadlessSynth`] is the native-testable driver — build from a patch, feed MIDI,
//! render fixed-size blocks — and the wasm32-only [`wasm`] module wraps it as a
//! `WasmSynth` a WebAudio `AudioWorklet` can drive (backlog 000083).

mod builder;
mod headless;

pub use builder::{default_patch, engine_from_doc, engine_from_doc_with_values, BuildError};
pub use headless::HeadlessSynth;

// The browser audio binding compiles only for wasm32; the native build never sees it.
#[cfg(target_arch = "wasm32")]
mod wasm;
