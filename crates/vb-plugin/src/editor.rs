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
use nih_plug_egui::resizable_window::ResizableWindow;
use nih_plug_egui::{create_egui_editor, EguiState};
use vb_core::{Edge, NodeId};
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

/// A cheap fingerprint of a document's **structure** (node ids + kinds, and the edges) — the
/// parts a full engine rebuild is needed for. It deliberately ignores parameter values: those
/// reach the running engine live through the host parameters (see [`create`]), so a value change
/// must NOT be seen as structural or it would trigger the rebuild-and-swap that clicks.
fn structure_of(doc: &ProjectDoc) -> (Vec<(NodeId, String)>, Vec<Edge>) {
    (
        doc.nodes
            .iter()
            .map(|e| (e.id, e.node.kind.clone()))
            .collect(),
        doc.edges.clone(),
    )
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
    /// Structure (nodes + edges) last built into the engine, so a rebuild is kicked ONLY when the
    /// graph shape actually changes — never for a mere parameter tweak, which is applied live.
    last_structure: (Vec<(NodeId, String)>, Vec<Edge>),
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
    let last_structure = structure_of(&app.patch().to_doc());
    // Seed from the host-parameter snapshot (always the full, current value set), so the first
    // edit diffs against what the host actually holds and only real knob moves are pushed.
    let last_values = params.values_by_node();

    // A clone for the render closure: `ResizableWindow` needs the state to store the size the
    // drag corner requests, which `create_egui_editor`'s own loop then asks the host to apply.
    let resize_state = egui_state.clone();

    create_egui_editor(
        egui_state,
        EditorHost {
            app,
            last_revision,
            last_view_revision,
            last_structure,
            last_values,
        },
        |_ctx, _state| {},
        move |ctx, setter, state| {
            // Host a **resizable** window: `ResizableWindow` adds a drag corner that stores the
            // requested size, and `create_egui_editor`'s loop then asks the host to apply it via
            // the *implemented* `request_resize` path (unlike host-frame-drag, which routes through
            // nih-plug's still-`TODO` `ext_gui_set_size` and would leave the surface black). The
            // editor draws INSIDE the window's `ui` via `render_inside`, so its own side/central
            // panels no longer clash with the wrapper's — the layout conflict behind the earlier
            // black-window revert (000178→000180).
            ResizableWindow::new("vb-resizable-window")
                .min_size((640.0, 480.0))
                .show(ctx, &resize_state, |ui| state.app.render_inside(ui));

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

                // Rebuild the engine ONLY when the graph structure changed (a node or wire added
                // or removed) — a pure parameter tweak is applied live to the running engine via
                // the host parameters (`process` → `set_doc_params`), so rebuilding for it would
                // swap in a fresh, state-reset engine and CLICK. This is the fix for "turning a
                // knob during playback corrupts the sound": the earlier fix live-updated in
                // `process` but this mirror still rebuilt on every audible edit, clobbering it.
                let structural = {
                    let sig = structure_of(&new_doc);
                    let changed = sig != state.last_structure;
                    state.last_structure = sig;
                    changed
                };
                if let Ok(mut slot) = doc.lock() {
                    *slot = Arc::new(new_doc);
                }
                if structural {
                    executor.execute_background(PluginTask::RebuildEngine);
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::structure_of;
    use vb_nodes::Registry;

    #[test]
    fn a_value_change_is_not_structural_but_a_node_or_wire_change_is() {
        let registry = Registry::with_builtins();
        let doc = crate::default_patch(&registry);

        // A parameter tweak leaves the structure identical — so the editor must NOT rebuild the
        // engine for it (the tweak reaches audio live via the host params). This is the guard for
        // "turning a knob still clicks".
        let mut retuned = doc.clone();
        let ladder = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "ladder")
            .map(|e| e.id)
            .expect("default patch has a ladder");
        assert!(retuned.set_node_values(ladder, vec![500.0, 0.5, 2_000.0]));
        assert_eq!(
            structure_of(&doc),
            structure_of(&retuned),
            "a value change must not read as structural"
        );

        // Removing a node changes the structure — that DOES need a rebuild.
        let mut fewer = doc.clone();
        fewer.nodes.pop();
        assert_ne!(
            structure_of(&doc),
            structure_of(&fewer),
            "removing a node is structural"
        );

        // Removing an edge changes the structure too.
        let mut unwired = doc.clone();
        unwired.edges.pop();
        assert_ne!(
            structure_of(&doc),
            structure_of(&unwired),
            "removing a wire is structural"
        );
    }
}
