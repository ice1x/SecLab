//! Import tests (backlog 000095): a RON patch (`ProjectDoc`) writes metadata and kind
//! links into the library, and the library becomes queryable by contained node kind.
//!
//! Compiles only under the `library` feature: `cargo test -p vb-library --features library`.
#![cfg(feature = "library")]

use vb_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};
use vb_library::{import_patch, kind, patches_with_node_kind, Library};
use vb_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

fn audio_in(name: &str) -> Port {
    Port::input(name, SignalType::Audio)
}
fn audio_out(name: &str) -> Port {
    Port::output(name, SignalType::Audio)
}

fn node(id: u64, kind: &str, ins: Vec<Port>, outs: Vec<Port>) -> NodeEntry {
    NodeEntry::new(NodeId(id), GraphNode::new(kind, ins, outs))
}

/// oscillator(0) → biquad(1) → gain(2).
fn synth_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![
            node(0, "oscillator", vec![], vec![audio_out("out")]),
            node(1, "biquad", vec![audio_in("in")], vec![audio_out("out")]),
            node(2, "gain", vec![audio_in("in")], vec![audio_out("out")]),
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

#[test]
fn importing_a_patch_stores_summary_metadata() {
    let lib = Library::open_in_memory().unwrap();
    let patch = import_patch(&lib, &synth_doc(), "My Synth", "a warm analog bass").unwrap();

    assert_eq!(patch.kind, kind::PATCH);
    assert_eq!(patch.title, "My Synth");

    let props = &patch.properties.0;
    assert_eq!(props["node_count"], serde_json::json!(3));
    assert_eq!(props["edge_count"], serde_json::json!(2));
    assert_eq!(props["format_version"], serde_json::json!(FORMAT_VERSION));
    // Distinct kinds, sorted.
    assert_eq!(
        props["kinds"],
        serde_json::json!(["biquad", "gain", "oscillator"])
    );
}

#[test]
fn imported_patches_are_queryable_by_contained_node_kind() {
    let lib = Library::open_in_memory().unwrap();
    import_patch(&lib, &synth_doc(), "Synth A", "").unwrap();

    // A second patch that also contains a biquad but no separate gain path.
    let filter_doc = ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![
            node(0, "oscillator", vec![], vec![audio_out("out")]),
            node(1, "biquad", vec![audio_in("in")], vec![audio_out("out")]),
        ],
        edges: vec![Edge {
            from_node: NodeId(0),
            from_port: PortId(0),
            to_node: NodeId(1),
            to_port: PortId(0),
        }],
    };
    import_patch(&lib, &filter_doc, "Filter B", "").unwrap();

    // Both patches contain a biquad.
    let mut with_biquad: Vec<String> = patches_with_node_kind(&lib, "biquad")
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    with_biquad.sort();
    assert_eq!(with_biquad, vec!["Filter B", "Synth A"]);

    // Only the first contains a gain.
    let with_gain: Vec<String> = patches_with_node_kind(&lib, "gain")
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert_eq!(with_gain, vec!["Synth A"]);

    // A kind nobody uses returns nothing (and does not error on a missing tag).
    assert!(patches_with_node_kind(&lib, "reverb").unwrap().is_empty());
}

#[test]
fn a_duplicate_title_is_rejected() {
    // Titles are the library key. drevo stopped enforcing title uniqueness (#595), so
    // `import_patch` owns the invariant and must refuse a repeat with DuplicateTitle.
    let lib = Library::open_in_memory().unwrap();
    import_patch(&lib, &synth_doc(), "Unique", "").unwrap();
    let err = import_patch(&lib, &synth_doc(), "Unique", "")
        .expect_err("re-importing under the same title is refused");
    assert!(
        matches!(err, drevo::error::DrevoError::DuplicateTitle(ref t) if t == "Unique"),
        "the refusal is a DuplicateTitle for the taken title, got {err:?}"
    );
    // A different title still imports cleanly.
    import_patch(&lib, &synth_doc(), "Different", "").expect("a fresh title still imports");
}
