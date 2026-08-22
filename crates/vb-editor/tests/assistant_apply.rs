//! Integration: wiring an accepted assistant proposal into the editor (backlog 000124).
//!
//! The full round-trip a user drives: the canvas holds a patch, they ask the assistant
//! for an edit, the assistant returns a validated `ProjectDoc` plus a `PatchDiff` to
//! preview, and — on accept — the editor applies it as **one undoable command** whose
//! revision bump is the "audition" signal an embedder watches to rebuild the audio
//! engine. This exercises that path end to end against a scripted (offline) provider,
//! so it stays deterministic and needs no network.

use vb_ai::{edit_patch, ProviderConfig, ScriptedProvider};
use vb_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};
use vb_editor::PatchView;
use vb_nodes::Registry;
use vb_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

fn audio_in(name: &str) -> Port {
    Port::input(name, SignalType::Audio)
}
fn audio_out(name: &str) -> Port {
    Port::output(name, SignalType::Audio)
}
fn control_in(name: &str) -> Port {
    Port::input(name, SignalType::Control)
}

fn node(id: u64, kind: &str, ins: Vec<Port>, outs: Vec<Port>) -> NodeEntry {
    NodeEntry::new(NodeId(id), GraphNode::new(kind, ins, outs))
}

fn edge(fnode: u64, fport: usize, tnode: u64, tport: usize) -> Edge {
    Edge {
        from_node: NodeId(fnode),
        from_port: PortId(fport),
        to_node: NodeId(tnode),
        to_port: PortId(tport),
    }
}

/// The starting patch on the canvas: oscillator(0) → gain(1).
fn current_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![
            node(0, "oscillator", vec![], vec![audio_out("out")]),
            node(1, "gain", vec![audio_in("in")], vec![audio_out("out")]),
        ],
        edges: vec![edge(0, 0, 1, 0)],
    }
}

/// The assistant's proposal: insert a biquad filter between the oscillator and the gain,
/// keeping the oscillator (0) and gain (1) ids stable so the diff reads as an insert.
fn proposal_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![
            node(0, "oscillator", vec![], vec![audio_out("out")]),
            node(1, "gain", vec![audio_in("in")], vec![audio_out("out")]),
            node(
                2,
                "biquad",
                vec![audio_in("in"), control_in("cutoff")],
                vec![audio_out("out")],
            ),
        ],
        edges: vec![edge(0, 0, 2, 0), edge(2, 0, 1, 0)],
    }
}

/// Frames a proposal doc as the assistant would reply: a sentence of explanation and the
/// full updated patch in a fenced RON block.
fn scripted_reply(doc: &ProjectDoc) -> String {
    let ron = vb_project::to_ron(doc).expect("serialize proposal");
    format!("Inserted a low-pass filter after the oscillator.\n\n```ron\n{ron}\n```")
}

#[test]
fn an_accepted_assistant_edit_applies_as_one_undoable_audition() {
    let registry = Registry::with_builtins();
    let config = ProviderConfig::default();

    // The editor starts with osc → gain on the canvas.
    let mut patch = PatchView::new();
    assert!(patch.apply_doc(&current_doc(), &registry));
    patch.clear_history(); // the initial load is not something the user should undo into.
    assert_eq!(patch.graph().node_count(), 2);

    // The assistant proposes an edit against the editor's own exported structure.
    let provider = ScriptedProvider::replies(&[&scripted_reply(&proposal_doc())]);
    let outcome = edit_patch(
        &provider,
        &registry,
        &config,
        &patch.to_doc(),
        "insert a low-pass filter after the oscillator",
        3,
    )
    .expect("the scripted proposal validates");

    // The preview (shown before accepting) reflects the insert.
    assert!(!outcome.diff.is_empty(), "there is a change to preview");
    assert!(
        outcome
            .diff
            .added_nodes
            .iter()
            .any(|(id, kind)| *id == NodeId(2) && kind == "biquad"),
        "the diff shows the inserted filter: {}",
        outcome.diff.summary()
    );
    assert!(
        outcome.diff.removed_edges.contains(&edge(0, 0, 1, 0)),
        "the old direct osc → gain wire is replaced"
    );

    // Accept: apply the proposal. The revision must advance — that bump is the audition
    // trigger an embedder watches to rebuild the engine.
    let before = patch.revision();
    assert!(patch.apply_doc(&outcome.doc, &registry), "proposal applied");
    assert!(
        patch.revision() > before,
        "applying an accepted edit is an audible change (audition)"
    );
    assert_eq!(patch.graph().node_count(), 3, "filter is on the canvas");
    assert_eq!(patch.graph().edges().len(), 2, "rewired through the filter");
    assert!(
        patch.draw_order().any(|(_, n, _)| n.kind == "biquad"),
        "the inserted node is present"
    );

    // The whole accept is a single undo step, back to the pre-edit patch.
    assert!(patch.undo(), "one undo reverts the entire accepted edit");
    assert_eq!(patch.graph().node_count(), 2);
    assert!(patch.draw_order().all(|(_, n, _)| n.kind != "biquad"));
    assert!(!patch.can_undo(), "nothing before the edit to undo into");
}

#[test]
fn a_rejected_proposal_leaves_the_canvas_untouched() {
    // Rejecting is simply "do not call apply_doc": the editor state is unchanged and no
    // audition is triggered. This documents that the proposal is inert until accepted.
    let registry = Registry::with_builtins();
    let config = ProviderConfig::default();

    let mut patch = PatchView::new();
    assert!(patch.apply_doc(&current_doc(), &registry));
    patch.clear_history();
    let before = patch.revision();

    let provider = ScriptedProvider::replies(&[&scripted_reply(&proposal_doc())]);
    let _outcome = edit_patch(
        &provider,
        &registry,
        &config,
        &patch.to_doc(),
        "insert a low-pass filter after the oscillator",
        3,
    )
    .expect("proposal validates");

    // User rejects → nothing applied.
    assert_eq!(patch.graph().node_count(), 2, "canvas unchanged");
    assert_eq!(
        patch.revision(),
        before,
        "no audition on a rejected proposal"
    );
    assert!(
        !patch.can_undo(),
        "no history entry from a rejected proposal"
    );
}
