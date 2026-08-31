//! Complements `crates/vb-nodes/tests/custom_node.rs`: that test proves a third-party node
//! runs through the low-level `vb_core::Engine::compile`; this one proves it also survives
//! the **project-document** pipeline the plugin and editor actually use —
//! `ProjectDoc::from_graph` → [`engine_from_doc`], where the builder must resolve the custom
//! kind by name out of the supplied [`Registry`]. A builder that only knew the built-in
//! kinds would pass the vb-nodes test but fail here.

use vb_core::{Graph, GraphNode, Parameter, Port, PortId, ProcessContext, Processor, SignalType};
use vb_nodes::{BuildContext, NodeDescriptor, Registry};
use vb_plugin::engine_builder::engine_from_doc;
use vb_project::ProjectDoc;

/// A minimal custom node: scale the input by a `factor` parameter.
struct Scale {
    factor: f32,
}

impl Processor for Scale {
    fn process(&mut self, ctx: &ProcessContext, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let input = inputs.first().copied().unwrap_or(&[]);
        for (o, x) in outputs[0].iter_mut().zip(input.iter()).take(ctx.frames) {
            *o = x * self.factor;
        }
    }
}

fn scale_descriptor() -> NodeDescriptor {
    NodeDescriptor::new(
        "scale",
        "math",
        || {
            GraphNode::new(
                "scale",
                vec![Port::input("in", SignalType::Audio)],
                vec![Port::output("out", SignalType::Audio)],
            )
        },
        || Box::new(Scale { factor: 1.0 }),
        || vec![Parameter::new("factor", 0.0, 10.0, 2.0)],
        |_ctx, values| {
            Box::new(Scale {
                factor: values.first().copied().unwrap_or(2.0),
            })
        },
    )
}

#[test]
fn a_custom_node_builds_and_runs_through_engine_from_doc() {
    let mut registry = Registry::with_builtins();
    registry.register(scale_descriptor());

    // A patch that uses the custom kind: oscillator → scale (terminal audio sink).
    let mut g = Graph::new();
    let osc = g.add_node(registry.graph_node("oscillator").unwrap());
    let scale = g.add_node(registry.graph_node("scale").unwrap());
    g.connect(osc, PortId(0), scale, PortId(0)).unwrap();

    let mut doc = ProjectDoc::from_graph(&g);
    doc.set_node_values(osc, vec![0.0, 220.0]); // sine, 220 Hz
    doc.set_node_values(scale, vec![0.5]); // half gain — from the stored values

    // The doc-based builder must resolve "scale" from the supplied registry.
    let (mut engine, sink) =
        engine_from_doc(&doc, &registry, &BuildContext::default(), 256).expect("builds");
    let sink = sink.expect("the custom node is the terminal audio sink");
    engine.process(&ProcessContext {
        sample_rate: 48_000.0,
        frames: 256,
    });
    let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
    assert!(out.iter().all(|s| s.is_finite()));
    let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!(
        peak > 0.0,
        "the custom node passed audio through the doc builder: {peak}"
    );
    assert!(
        peak <= 0.5 + 1e-3,
        "the stored factor 0.5 was applied: {peak}"
    );
}

#[test]
fn engine_from_doc_rejects_an_unregistered_kind() {
    // The flip side: without registering the descriptor, the same patch must fail to build
    // rather than silently dropping the node — the guarantee a downstream user relies on.
    let plain = Registry::with_builtins();
    let mut authoring = Registry::with_builtins();
    authoring.register(scale_descriptor());

    let mut g = Graph::new();
    let osc = g.add_node(authoring.graph_node("oscillator").unwrap());
    let scale = g.add_node(authoring.graph_node("scale").unwrap());
    g.connect(osc, PortId(0), scale, PortId(0)).unwrap();
    let doc = ProjectDoc::from_graph(&g);

    // Built against a registry that never learned about "scale".
    assert!(
        engine_from_doc(&doc, &plain, &BuildContext::default(), 256).is_err(),
        "an unknown kind must be a build error, not a silent drop"
    );
}
