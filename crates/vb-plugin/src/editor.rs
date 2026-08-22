//! The plugin's in-window editor — the full node editor, embedded (000067 → constructor
//! in the DAW).
//!
//! Since egui was unified on 0.31 (PR #133), the plugin hosts the **same**
//! [`vb_editor::EditorApp`] the standalone uses, rendered through `nih_plug_egui` into the
//! plugin's own window — one editor, not a second drawing layer. The editor edits the
//! plugin's shared project document; on a structural change it mirrors the new graph back
//! and kicks a background engine rebuild, so the sound follows the canvas **inside the DAW**.
//!
//! Limits inherent to the plugin format, not bugs:
//! - The host-automatable **parameter list is fixed at load** (a DAW plugin cannot add
//!   parameters at runtime), so parameters of nodes *added* in-session are not exposed to
//!   host automation until the plugin is reloaded.
//! - The rebuild is **structural**: existing nodes keep their live parameter values; nodes
//!   added in-session take their registry defaults.
//!
//! The whole module compiles only under the `gui` feature (see `lib.rs`), so the default
//! build — and the headless clap-validator gate — never links an editor.

use std::sync::{Arc, Mutex};

use nih_plug::prelude::{AsyncExecutor, Editor};
use nih_plug_egui::{create_egui_editor, EguiState};
use vb_editor::EditorApp;
use vb_project::ProjectDoc;

use crate::{PluginTask, VstBomb};

/// The editor's persistent state: the live editor plus the patch revision last mirrored into
/// the shared doc, so a rebuild is kicked only when the graph actually changed.
struct EditorHost {
    app: EditorApp,
    last_revision: u64,
}

/// Builds the embedded node editor for the plugin window.
///
/// `doc` is the instance's shared project (also read by the background rebuild); `executor`
/// schedules the off-audio-thread rebuild when the graph is edited.
pub(crate) fn create(
    egui_state: Arc<EguiState>,
    doc: Arc<Mutex<Arc<ProjectDoc>>>,
    executor: AsyncExecutor<VstBomb>,
) -> Option<Box<dyn Editor>> {
    // Seed the editor with the patch this instance is playing, not the demo.
    let mut app = EditorApp::new();
    if let Ok(current) = doc.lock() {
        app.load_doc(&current);
    }
    let last_revision = app.patch().revision();

    create_egui_editor(
        egui_state,
        EditorHost { app, last_revision },
        |_ctx, _state| {},
        move |ctx, _setter, state| {
            state.app.render(ctx);

            // A structural edit bumps the patch revision: mirror the new graph into the
            // shared doc and rebuild the engine off the audio thread.
            let revision = state.app.patch().revision();
            if revision != state.last_revision {
                state.last_revision = revision;
                let new_doc = state.app.patch().to_doc();
                if let Ok(mut slot) = doc.lock() {
                    *slot = Arc::new(new_doc);
                }
                executor.execute_background(PluginTask::RebuildEngine);
            }
        },
    )
}
