//! Integration test: wire a small realistic patch and assert the graph accepts it.
//!
//! Mirrors a first-plugin workflow: oscillator → filter → output.

use vb_core::{Graph, GraphNode, Port, PortId, SignalType};

fn oscillator() -> GraphNode {
    GraphNode::new(
        "oscillator",
        vec![Port::input("freq", SignalType::Control)],
        vec![Port::output("out", SignalType::Audio)],
    )
}

fn filter() -> GraphNode {
    GraphNode::new(
        "filter",
        vec![
            Port::input("in", SignalType::Audio),
            Port::input("cutoff", SignalType::Control),
        ],
        vec![Port::output("out", SignalType::Audio)],
    )
}

fn output() -> GraphNode {
    GraphNode::new("output", vec![Port::input("in", SignalType::Audio)], vec![])
}

#[test]
fn osc_filter_output_chain_wires_up() {
    let mut g = Graph::new();
    let osc = g.add_node(oscillator());
    let flt = g.add_node(filter());
    let out = g.add_node(output());

    // osc.out (audio) -> filter.in (audio)
    g.connect(osc, PortId(0), flt, PortId(0))
        .expect("osc→filter audio");
    // filter.out (audio) -> output.in (audio)
    g.connect(flt, PortId(0), out, PortId(0))
        .expect("filter→output audio");

    assert_eq!(g.node_count(), 3);
    assert_eq!(g.edges().len(), 2);
}

#[test]
fn audio_into_control_input_is_rejected() {
    let mut g = Graph::new();
    let osc = g.add_node(oscillator());
    let flt = g.add_node(filter());

    // osc.out is audio; filter.cutoff (PortId 1) is control — must be rejected.
    let err = g.connect(osc, PortId(0), flt, PortId(1));
    assert!(err.is_err(), "audio must not feed a control input");
}
