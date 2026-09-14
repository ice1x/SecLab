//! End-to-end interface tests.
//!
//! These drive [`vb_editor::EditorApp`] through the **same public API the plugin and
//! standalone UI call** — the launch seed, canvas camera, click/drag hit-testing, the
//! inspector's parameter edits, and the project RON import/export buttons — exercising the
//! whole editing surface as a user's session would, not one method at a time. They are
//! headless (no window/egui frame is pumped): every interaction goes through the public
//! coordinate-mapped entry points, so a regression in wiring, selection, parameter handling,
//! or serialization fails here the way it would in the real editor.
//!
//! This complements the in-crate unit tests (which check each interaction in isolation) with
//! a composed journey and the two invariants that today's shipped bugs violated: the launch
//! canvas must show the real gated default voice, and an edited parameter must survive a
//! project round-trip.

use egui::{Pos2, Vec2};
use vb_core::{PortDirection, PortId};
use vb_editor::{EditorApp, NodeStyle};

/// The screen point of a node's port, mapped through the live camera — what a mouse would
/// have to hit to grab that port.
fn port_on_screen(app: &EditorApp, node: vb_core::NodeId, dir: PortDirection, port: usize) -> Pos2 {
    let anchor = app
        .patch()
        .port_anchor(node, dir, PortId(port), &NodeStyle::default())
        .expect("port anchor");
    app.camera().canvas_to_screen(anchor)
}

fn node_kinds(app: &EditorApp) -> Vec<String> {
    let mut kinds: Vec<String> = app
        .patch()
        .draw_order()
        .map(|(_, n, _)| n.kind.clone())
        .collect();
    kinds.sort();
    kinds
}

/// The editor must open on the **real shipped default voice**, and that voice must be
/// note-gated (a `vca` shut by an envelope off a `midi-in`) — not a bare oscillator wired
/// straight to the output. This is exactly the invariant a stale drone patch broke.
#[test]
fn the_editor_launches_showing_the_gated_default_voice() {
    let app = EditorApp::new();
    let kinds = node_kinds(&app);

    assert!(
        kinds.len() > 10,
        "the default is the full Minitaur voice, not a two-node demo: {kinds:?}"
    );
    for required in ["vca", "midi-in", "adsr", "ladder"] {
        assert!(
            kinds.iter().any(|k| k == required),
            "a gated voice must contain a {required}: {kinds:?}"
        );
    }
}

/// A full session: launch → add a node → wire it on the canvas through the camera → edit a
/// parameter in the inspector → export the project → re-import it → select a node. Every step
/// uses the public UI entry point, so the journey covers the editing surface end to end.
#[test]
fn a_full_editing_session_round_trips_through_the_public_api() {
    let mut app = EditorApp::new();

    // 1. Add two nodes via the palette's spawn path (which attaches their registry
    //    parameters), then move the second clear of the first so the camera hit-test is
    //    unambiguous — spawn drops every node on the same point.
    let osc = app.spawn_node("oscillator").expect("spawn oscillator");
    let gain = app.spawn_node("gain").expect("spawn gain");
    app.patch_mut().move_node(gain, Vec2::new(440.0, 170.0));
    let edges_before = app.patch().graph().edges().len();

    // 2. Pan/zoom so the screen↔canvas mapping is non-trivial, then drag a wire from the
    //    oscillator's output to the gain's input exactly as a mouse would.
    app.pan(Vec2::new(30.0, -20.0));
    app.zoom(Pos2::new(50.0, 50.0), 1.3);
    let from = port_on_screen(&app, osc, PortDirection::Output, 0);
    let to = port_on_screen(&app, gain, PortDirection::Input, 0);
    app.drag_wire(from, to);

    let edges = app.patch().graph().edges();
    assert_eq!(
        edges.len(),
        edges_before + 1,
        "dragging osc.out → gain.in must add exactly one edge"
    );
    let new = edges.last().unwrap();
    assert_eq!(
        (new.from_node, new.to_node),
        (osc, gain),
        "the new wire connects the two nodes we dragged between"
    );

    // 3. Edit a parameter in the inspector path: gain's level, clamped to its range.
    let clamped = app
        .patch_mut()
        .set_param(gain, 0, 0.25)
        .expect("gain has a parameter at index 0");
    assert!(
        (clamped - 0.25).abs() < 1e-4,
        "an in-range gain edit stores as-is: {clamped}"
    );

    // 4. Export the whole project to RON (the "export ⤓" button), then re-import it (the
    //    "import ⤒" button). The graph must survive the round-trip unchanged.
    let kinds_before = node_kinds(&app);
    let ron = app.export_project_ron().expect("project exports to RON");
    assert!(ron.contains("oscillator") && ron.contains("gain"));
    app.import_project_ron(&ron)
        .expect("the exported RON re-imports cleanly");
    assert_eq!(
        node_kinds(&app),
        kinds_before,
        "import(export(patch)) preserves every node"
    );

    // 5. Selection through the camera: clicking a node's frame selects it; clicking empty
    //    canvas clears the selection.
    let osc_after = app
        .patch()
        .draw_order()
        .find(|(_, n, _)| n.kind == "oscillator")
        .map(|(id, _, _)| id)
        .expect("oscillator survived the round-trip");
    let title = app
        .patch()
        .draw_order()
        .find(|(id, _, _)| *id == osc_after)
        .map(|(_, _, pos)| app.camera().canvas_to_screen(pos + Vec2::new(12.0, 10.0)))
        .unwrap();
    app.click(title);
    assert_eq!(
        app.patch().selected(),
        Some(osc_after),
        "clicking a node's frame selects it"
    );
    app.click(app.camera().canvas_to_screen(Pos2::new(-4_000.0, -4_000.0)));
    assert_eq!(
        app.patch().selected(),
        None,
        "clicking empty canvas clears the selection"
    );
}

/// A parameter edited in the inspector must survive a project export → fresh-editor import —
/// the value round-trips through the `.vbp` RON, not just the node shape. (An earlier bug
/// dropped stored parameter values on load; this guards the whole edit→save→load path.)
#[test]
fn an_edited_parameter_survives_a_project_round_trip() {
    let mut app = EditorApp::new();
    let gain = app.spawn_node("gain").expect("spawn gain");
    app.patch_mut()
        .set_param(gain, 0, 0.33)
        .expect("gain param 0");
    let ron = app.export_project_ron().expect("export");

    // Load the exported project into a brand-new editor (a fresh session opening the file).
    let mut reopened = EditorApp::new();
    reopened.import_project_ron(&ron).expect("reimport");
    let reopened_gain = reopened
        .patch()
        .draw_order()
        .find(|(_, n, _)| n.kind == "gain")
        .map(|(id, _, _)| id)
        .expect("gain present after reopen");
    let value = reopened.patch().params(reopened_gain)[0].value;
    assert!(
        (value - 0.33).abs() < 1e-4,
        "the edited gain value must round-trip through the project RON: {value}"
    );
}
