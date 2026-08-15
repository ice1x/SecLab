//! Integration: a patch survives the trip to RON text / disk and back.

use vb_core::{Graph, GraphNode, Port, PortId, SignalType};
use vb_project::{
    from_ron, load_from_path, open_path, save_to_path, to_ron, ProjectDoc, FORMAT_VERSION,
};

fn sample_graph() -> Graph {
    let mut g = Graph::new();
    let osc = g.add_node(GraphNode::new(
        "oscillator",
        vec![],
        vec![Port::output("out", SignalType::Audio)],
    ));
    let flt = g.add_node(GraphNode::new(
        "biquad",
        vec![Port::input("in", SignalType::Audio)],
        vec![Port::output("out", SignalType::Audio)],
    ));
    g.connect(osc, PortId(0), flt, PortId(0)).unwrap();
    g
}

#[test]
fn ron_round_trip_preserves_document() {
    let doc = ProjectDoc::from_graph(&sample_graph());
    assert_eq!(doc.version, FORMAT_VERSION);
    let parsed = from_ron(&to_ron(&doc).unwrap()).unwrap();
    assert_eq!(doc, parsed);
}

#[test]
fn graph_to_doc_to_graph_preserves_topology() {
    let original = sample_graph();
    let doc = ProjectDoc::from_graph(&original);
    let rebuilt = doc.to_graph().unwrap();
    assert_eq!(rebuilt.node_count(), original.node_count());
    assert_eq!(rebuilt.edges().len(), original.edges().len());

    let kinds: Vec<String> = ProjectDoc::from_graph(&rebuilt)
        .nodes
        .iter()
        .map(|n| n.node.kind.clone())
        .collect();
    assert!(kinds.iter().any(|k| k == "oscillator"));
    assert!(kinds.iter().any(|k| k == "biquad"));
}

#[test]
fn rebuilt_graph_has_no_cycle() {
    let doc = ProjectDoc::from_graph(&sample_graph());
    assert!(!doc.to_graph().unwrap().has_cycle());
}

#[test]
fn file_save_and_open_round_trip() {
    let doc = ProjectDoc::from_graph(&sample_graph());
    let path = std::env::temp_dir().join("vb_project_file_round_trip.ron");
    save_to_path(&doc, &path).unwrap();

    assert_eq!(load_from_path(&path).unwrap(), doc);
    // The strict path also migrates and validates.
    assert_eq!(open_path(&path).unwrap(), doc);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn bad_ron_is_an_error() {
    assert!(from_ron("this is not ron").is_err());
}
