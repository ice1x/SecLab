//! End-to-end MVP scenario (backlog 000125): a scripted assistant dialogue produces a
//! validated patch, that patch survives the on-disk export/load contract, and it
//! compiles into a runnable instrument that streams finite audio.
//!
//! This ties the whole spine together — Epic 12 (the assistant), Epic 5 (the project
//! format), and Epic 7 (the plugin engine) — against a scripted, offline provider so it
//! stays deterministic and needs no network or API key:
//!
//! 1. **Scripted dialogue → validated graph.** Two turns through `vb-ai` — a `generate`
//!    ("a simple synth") then an `edit` ("add a low-pass filter") — each runs the same
//!    mandatory validation (registry kinds + `vb-core` structural validation) the real
//!    assistant uses.
//! 2. **Export contract.** The final `ProjectDoc` is serialized to RON and read back —
//!    exactly the format the exported plugin loads from `VB_PATCH` — proving the
//!    assistant's output is export-ready.
//! 3. **Runnable instrument.** The reloaded patch compiles via `engine_from_doc` (the
//!    plugin's own builder) and processes a block into finite audio at its sink.
//!
//! The physical `.clap` / `.vst3` bundle is produced by the existing
//! `cargo build -p vb-plugin` + clap-validator CI path, not rebuilt here; this test
//! proves the assistant's patch is a valid, export-ready, runnable instrument — the
//! content that bundle loads.

use vb_ai::{edit_patch, generate_patch, ProviderConfig, ScriptedProvider};
use vb_core::{Edge, GraphNode, NodeId, Port, PortId, ProcessContext, SignalType};
use vb_nodes::{BuildContext, Registry};
use vb_plugin::engine_from_doc;
use vb_project::{from_ron, to_ron, NodeEntry, ProjectDoc, FORMAT_VERSION};

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

/// The assistant's first proposal: a minimal synth, oscillator(0) → gain(1).
fn synth_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![
            node(0, "oscillator", vec![], vec![audio_out("out")]),
            node(1, "gain", vec![audio_in("in")], vec![audio_out("out")]),
        ],
        edges: vec![edge(0, 0, 1, 0)],
    }
}

/// The edited proposal: insert a biquad filter, keeping the osc(0)/gain(1) ids stable.
fn filtered_doc() -> ProjectDoc {
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

/// Frames a doc the way the assistant replies: a sentence of prose then a fenced RON block.
fn reply(prose: &str, doc: &ProjectDoc) -> String {
    let ron = to_ron(doc).expect("serialize scripted doc");
    format!("{prose}\n\n```ron\n{ron}\n```")
}

#[test]
fn scripted_dialogue_yields_an_export_ready_runnable_instrument() {
    let registry = Registry::with_builtins();
    let config = ProviderConfig::default();

    // One provider replays both turns of the dialogue in order.
    let provider = ScriptedProvider::replies(&[
        &reply("Here is a simple subtractive synth.", &synth_doc()),
        &reply(
            "Added a low-pass filter after the oscillator.",
            &filtered_doc(),
        ),
    ]);

    // Turn 1 — generate from a blank canvas.
    let generated = generate_patch(
        &provider,
        &registry,
        &config,
        "a simple subtractive synth",
        None,
        3,
    )
    .expect("the first turn produces a validated patch");
    assert_eq!(generated.doc.nodes.len(), 2, "osc → gain");
    assert!(generated.doc.is_valid());

    // Turn 2 — edit the generated patch.
    let edited = edit_patch(
        &provider,
        &registry,
        &config,
        &generated.doc,
        "add a low-pass filter after the oscillator",
        3,
    )
    .expect("the second turn produces a validated edit");
    assert!(
        edited
            .diff
            .added_nodes
            .iter()
            .any(|(_, kind)| kind == "biquad"),
        "the dialogue inserted a filter: {}",
        edited.diff.summary()
    );
    let final_doc = edited.doc;
    assert!(final_doc.is_valid(), "the final patch is valid");

    // Export contract: serialize to RON and read it back — the exact format the exported
    // plugin loads from VB_PATCH. The reloaded doc must be identical.
    let ron = to_ron(&final_doc).expect("export to RON");
    let reloaded = from_ron(&ron).expect("the exported patch reloads");
    assert_eq!(reloaded, final_doc, "the patch round-trips through export");

    // Runnable instrument: compile the reloaded patch with the plugin's own builder and
    // process a block. The sink must exist and every sample must be finite.
    let ctx = BuildContext::default();
    let (mut engine, sink) = engine_from_doc(&reloaded, &registry, &ctx, 256)
        .expect("the assistant's patch builds a runnable engine");
    let sink = sink.expect("the instrument has an audio sink to stream");

    engine.process(&ProcessContext {
        sample_rate: 48_000.0,
        frames: 256,
    });
    let out = engine
        .node_output(sink)
        .and_then(|b| b.channel(0))
        .expect("the sink produces audio");
    assert_eq!(out.len(), 256, "a full block of audio");
    assert!(
        out.iter().all(|s| s.is_finite()),
        "the instrument streams finite audio"
    );
}
