//! Integration: corrupt or foreign project files are rejected with actionable issues,
//! and the format version gate behaves.

use vb_core::{Edge, Graph, GraphNode, NodeId, Port, PortDirection, PortId, SignalType};
use vb_project::{
    migrate_to_current, open_ron, to_ron, Issue, NodeEntry, ProjectDoc, ProjectError,
    FORMAT_VERSION, MIN_SUPPORTED_VERSION,
};

fn source(kind: &str) -> GraphNode {
    GraphNode::new(kind, vec![], vec![Port::output("out", SignalType::Audio)])
}

fn thru(kind: &str) -> GraphNode {
    GraphNode::new(
        kind,
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    )
}

/// A minimal sound document: `osc(0) -> gain(1)`.
fn valid_doc() -> ProjectDoc {
    let mut g = Graph::new();
    let osc = g.add_node(source("oscillator"));
    let amp = g.add_node(thru("gain"));
    g.connect(osc, PortId(0), amp, PortId(0)).unwrap();
    ProjectDoc::from_graph(&g)
}

#[test]
fn a_sound_document_has_no_issues() {
    let doc = valid_doc();
    assert!(doc.is_valid(), "unexpected issues: {:?}", doc.validate());
}

#[test]
fn duplicate_node_ids_are_reported() {
    let mut doc = valid_doc();
    let clashing = doc.nodes[0].id;
    doc.nodes.push(NodeEntry {
        id: clashing,
        node: source("oscillator"),
    });
    assert!(doc.validate().contains(&Issue::DuplicateNodeId(clashing)));
}

#[test]
fn an_empty_kind_is_reported() {
    let mut doc = valid_doc();
    doc.nodes[0].node.kind = String::new();
    let id = doc.nodes[0].id;
    assert!(doc.validate().contains(&Issue::EmptyKind(id)));
}

#[test]
fn an_edge_to_a_missing_node_is_reported() {
    let mut doc = valid_doc();
    let ghost = NodeId(9_999);
    doc.edges.push(Edge {
        from_node: doc.nodes[0].id,
        from_port: PortId(0),
        to_node: ghost,
        to_port: PortId(0),
    });
    assert!(doc
        .validate()
        .iter()
        .any(|i| matches!(i, Issue::UnknownEdgeEndpoint { node, .. } if *node == ghost)));
}

#[test]
fn an_out_of_range_port_is_reported() {
    let mut doc = valid_doc();
    // The gain node has a single input; index 7 does not exist.
    doc.edges[0].to_port = PortId(7);
    assert!(doc.validate().iter().any(|i| matches!(
        i,
        Issue::PortOutOfRange {
            direction: PortDirection::Input,
            port: PortId(7),
            ..
        }
    )));
}

#[test]
fn a_signal_type_mismatch_is_reported() {
    let mut g = Graph::new();
    let osc = g.add_node(source("oscillator"));
    let env_sink = g.add_node(GraphNode::new(
        "cv-sink",
        vec![Port::input("cv", SignalType::Control)],
        vec![],
    ));
    let mut doc = ProjectDoc::from_graph(&g);
    // The graph would refuse this wiring; a hand-edited file can still contain it.
    doc.edges.push(Edge {
        from_node: osc,
        from_port: PortId(0),
        to_node: env_sink,
        to_port: PortId(0),
    });
    assert!(doc
        .validate()
        .iter()
        .any(|i| matches!(i, Issue::SignalMismatch { .. })));
}

#[test]
fn a_double_connected_input_is_reported() {
    let mut g = Graph::new();
    let a = g.add_node(source("osc-a"));
    let b = g.add_node(source("osc-b"));
    let amp = g.add_node(thru("gain"));
    g.connect(a, PortId(0), amp, PortId(0)).unwrap();
    let mut doc = ProjectDoc::from_graph(&g);
    doc.edges.push(Edge {
        from_node: b,
        from_port: PortId(0),
        to_node: amp,
        to_port: PortId(0),
    });
    assert!(doc.validate().contains(&Issue::InputAlreadyConnected {
        node: amp,
        port: PortId(0)
    }));
}

#[test]
fn a_cycle_is_reported() {
    let mut g = Graph::new();
    let a = g.add_node(thru("a"));
    let b = g.add_node(thru("b"));
    g.connect(a, PortId(0), b, PortId(0)).unwrap();
    g.connect(b, PortId(0), a, PortId(0)).unwrap();
    assert!(ProjectDoc::from_graph(&g)
        .validate()
        .contains(&Issue::Cycle));
}

#[test]
fn open_ron_accepts_a_sound_document() {
    let doc = valid_doc();
    assert_eq!(open_ron(&to_ron(&doc).unwrap()).unwrap(), doc);
}

#[test]
fn open_ron_rejects_a_corrupt_document_with_issues() {
    let mut doc = valid_doc();
    doc.edges[0].to_port = PortId(7);
    match open_ron(&to_ron(&doc).unwrap()) {
        Err(ProjectError::Invalid(issues)) => assert!(!issues.is_empty()),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn a_current_version_document_migrates_unchanged() {
    let doc = valid_doc();
    assert_eq!(migrate_to_current(doc.clone()).unwrap(), doc);
    assert_eq!(MIN_SUPPORTED_VERSION, 1);
}

#[test]
fn a_future_version_document_is_rejected() {
    let mut doc = valid_doc();
    doc.version = FORMAT_VERSION + 1;
    match migrate_to_current(doc) {
        Err(ProjectError::UnsupportedVersion { found, current, .. }) => {
            assert_eq!(found, FORMAT_VERSION + 1);
            assert_eq!(current, FORMAT_VERSION);
        }
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[test]
fn open_ron_rejects_a_future_version() {
    let mut doc = valid_doc();
    doc.version = FORMAT_VERSION + 99;
    assert!(matches!(
        open_ron(&to_ron(&doc).unwrap()),
        Err(ProjectError::UnsupportedVersion { .. })
    ));
}
