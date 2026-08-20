//! RAG retrieval tests (backlog 000109): the library retrieves relevant example patches
//! and renders them as prompt context.
//!
//! Compiles only under the `library` feature: `cargo test -p vb-library --features library`.
#![cfg(feature = "library")]

use vb_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};
use vb_library::{import_patch, Library};
use vb_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

fn audio_in(name: &str) -> Port {
    Port::input(name, SignalType::Audio)
}
fn audio_out(name: &str) -> Port {
    Port::output(name, SignalType::Audio)
}

/// oscillator(0) → biquad(1) → gain(2).
fn synth_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![
            NodeEntry {
                id: NodeId(0),
                node: GraphNode::new("oscillator", vec![], vec![audio_out("out")]),
            },
            NodeEntry {
                id: NodeId(1),
                node: GraphNode::new("biquad", vec![audio_in("in")], vec![audio_out("out")]),
            },
            NodeEntry {
                id: NodeId(2),
                node: GraphNode::new("gain", vec![audio_in("in")], vec![audio_out("out")]),
            },
        ],
        edges: vec![
            Edge {
                from_node: NodeId(0),
                from_port: PortId(0),
                to_node: NodeId(1),
                to_port: PortId(0),
            },
            Edge {
                from_node: NodeId(1),
                from_port: PortId(0),
                to_node: NodeId(2),
                to_port: PortId(0),
            },
        ],
    }
}

fn seeded() -> Library {
    let lib = Library::open_in_memory().unwrap();
    import_patch(&lib, &synth_doc(), "Fat Bass", "a warm analog bass").unwrap();
    import_patch(&lib, &synth_doc(), "Bright Lead", "a screaming saw lead").unwrap();
    lib
}

#[test]
fn retrieve_context_renders_relevant_patches() {
    let lib = seeded();
    let context = lib.retrieve_context("bass", 5).unwrap();

    assert!(context.contains("Relevant example patches"));
    assert!(
        context.contains("Fat Bass"),
        "the matching patch is included"
    );
    assert!(context.contains("a warm analog bass"), "its description");
    // The node kinds it contains are listed.
    assert!(context.contains("oscillator"));
    assert!(context.contains("biquad"));
    // The irrelevant patch is not pulled in by a "bass" query.
    assert!(!context.contains("Bright Lead"));
}

#[test]
fn retrieve_patches_ranks_and_limits() {
    let lib = seeded();
    let hits = lib.retrieve_patches("lead", 5).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Bright Lead");

    // A limit caps the count even when more could match.
    let both = lib.retrieve_patches("saw analog", 1).unwrap();
    assert!(both.len() <= 1);
}

#[test]
fn no_match_yields_empty_context() {
    let lib = seeded();
    assert_eq!(lib.retrieve_context("zzzqqq", 5).unwrap(), "");
    assert!(lib.retrieve_patches("zzzqqq", 5).unwrap().is_empty());
}
