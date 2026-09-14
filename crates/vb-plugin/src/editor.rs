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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nih_plug::prelude::{AsyncExecutor, Editor};
use nih_plug_egui::{create_egui_editor, EguiState};
use vb_core::NodeId;
use vb_editor::EditorApp;
use vb_project::ProjectDoc;

use crate::{changed_values, PluginTask, VstBomb, VstBombParams};

/// Per-node parameter values (registry `params()` order) of a document — the snapshot
/// [`changed_values`] diffs to find the knob the user moved in the editor.
fn doc_values(doc: &ProjectDoc) -> HashMap<NodeId, Vec<f32>> {
    doc.nodes
        .iter()
        .map(|entry| (entry.id, entry.values.clone()))
        .collect()
}

/// The editor's persistent state: the live editor, the patch revision last mirrored into the
/// shared doc (so a rebuild is kicked only when the graph actually changed), and the parameter
/// values last pushed to the host (so a knob edit is written through to exactly its host
/// parameter — see [`create`]).
struct EditorHost {
    app: EditorApp,
    last_revision: u64,
    /// View-only revision last persisted, so a panel drag / control retype re-serializes the
    /// document into the host state without rebuilding the engine.
    last_view_revision: u64,
    last_values: HashMap<NodeId, Vec<f32>>,
}

/// Builds the embedded node editor for the plugin window.
///
/// `doc` is the instance's shared project (also read by the background rebuild); `executor`
/// schedules the off-audio-thread rebuild when the graph is edited.
pub(crate) fn create(
    egui_state: Arc<EguiState>,
    params: Arc<VstBombParams>,
    doc: Arc<Mutex<Arc<ProjectDoc>>>,
    executor: AsyncExecutor<VstBomb>,
) -> Option<Box<dyn Editor>> {
    // Seed the editor with the patch this instance is playing, not the demo.
    let mut app = EditorApp::new();
    if let Ok(current) = doc.lock() {
        app.load_doc(&current);
    }
    let last_revision = app.patch().revision();
    let last_view_revision = app.patch().view_revision();
    // Seed from the host-parameter snapshot (always the full, current value set), so the first
    // edit diffs against what the host actually holds and only real knob moves are pushed.
    let last_values = params.values_by_node();

    create_egui_editor(
        egui_state,
        EditorHost {
            app,
            last_revision,
            last_view_revision,
            last_values,
        },
        |_ctx, _state| {},
        move |ctx, setter, state| {
            // Fixed-size window: render the whole editor into the context.
            //
            // A `ResizableWindow` drag corner was tried (000178) but reverted (000180): on the
            // AU / clap-wrapper path the host grows the window without egui's surface tracking
            // the new size, so a resize left most of the enlarged window black. nih-plug's CLAP
            // wrapper leaves host-driven resize unimplemented (`ext_gui_can_resize` == false),
            // and there is no headless surface to test the window layer against, so a real
            // resize can't be verified here — a fixed, correctly-filled window beats a broken
            // resizable one. Revisit when nih-plug implements host resize upstream.
            state.app.render(ctx);

            // Two edit streams reach the document: audible edits (a knob move, a wire, an added
            // node) bump the patch revision and need an engine rebuild; view-only edits (a panel
            // drag, a control retype) bump only the view revision and just need re-persisting.
            let revision = state.app.patch().revision();
            let view_revision = state.app.patch().view_revision();
            let audible = revision != state.last_revision;
            let presentational = view_revision != state.last_view_revision;
            if audible || presentational {
                state.last_revision = revision;
                state.last_view_revision = view_revision;
                let new_doc = state.app.patch().to_doc();

                if audible {
                    // Write each moved knob through to its host parameter. The engine rebuild
                    // sources node values from the host-parameter snapshot, and that snapshot
                    // wins over the document's stored values — so without this, an in-editor knob
                    // edit is silently reverted on the next rebuild (the bug behind "the knobs
                    // don't change the sound"). Pushing through the ParamSetter also lets the DAW
                    // record the gesture as automation.
                    let new_values = doc_values(&new_doc);
                    for (node, index, value) in changed_values(&state.last_values, &new_values) {
                        params.set_bound(setter, node, index, value);
                    }
                    state.last_values = new_values;
                }

                // Persist the document into the host's saved state so a DAW project reopens with
                // the current patch — including a rearranged/ retyped panel, which is otherwise
                // inaudible and would be lost. Then mirror it into the shared doc.
                if let Ok(ron) = vb_project::to_ron(&new_doc) {
                    params.set_patch_ron(ron);
                }
                if let Ok(mut slot) = doc.lock() {
                    *slot = Arc::new(new_doc);
                }

                // Only an audible change needs the audio engine rebuilt; a panel move does not.
                if audible {
                    executor.execute_background(PluginTask::RebuildEngine);
                }
            }
        },
    )
}
