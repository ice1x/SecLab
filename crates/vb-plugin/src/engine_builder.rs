//! Building a runnable engine out of a serialized project document.
//!
//! The plugin-side twin of the standalone's live builder: a [`ProjectDoc`] (loaded
//! from a `.vbp` RON file or the embedded default patch) becomes an allocation-free
//! [`RtEngine`] with processors from the registry, plus the node to listen to.

use std::collections::HashMap;

use vb_core::{NodeId, Processor, SignalType};
use vb_nodes::{BuildContext, Registry};
use vb_project::ProjectDoc;
use vb_runtime::RtEngine;

/// Why a document could not become an engine.
#[derive(Debug)]
pub enum BuildError {
    /// The document's graph failed structural validation.
    Project(String),
    /// A node kind is not in the registry.
    UnknownKind(String),
    /// The engine rejected the compiled graph.
    Engine(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Project(e) => write!(f, "project error: {e}"),
            Self::UnknownKind(kind) => write!(f, "unknown node kind '{kind}'"),
            Self::Engine(e) => write!(f, "engine error: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Compiles `doc` into an engine plus the sink node (a terminal audio output),
/// with every processor built from its defaults.
///
/// # Errors
///
/// Returns [`BuildError`] if the document is invalid, references unknown kinds, or
/// the engine rejects the graph.
pub fn engine_from_doc(
    doc: &ProjectDoc,
    registry: &Registry,
    ctx: &BuildContext,
    frames: usize,
) -> Result<(RtEngine, Option<NodeId>), BuildError> {
    engine_from_doc_with_values(doc, registry, ctx, frames, &HashMap::new())
}

/// Like [`engine_from_doc`], but any node whose document id appears in `overrides`
/// is built from those parameter values (in registry `params()` order) instead of
/// the values the document itself stores. This is the host-automation path: the plugin
/// snapshots its automatable parameters and rebuilds through here, while the document's
/// own stored values pin the preset's sound for every node it does not override.
///
/// `overrides` is keyed by **document** node id (as stored in [`ProjectDoc::nodes`]);
/// the runtime ids the engine uses are resolved internally via the graph remap.
///
/// # Errors
///
/// Same as [`engine_from_doc`].
pub fn engine_from_doc_with_values(
    doc: &ProjectDoc,
    registry: &Registry,
    ctx: &BuildContext,
    frames: usize,
    overrides: &HashMap<NodeId, Vec<f32>>,
) -> Result<(RtEngine, Option<NodeId>), BuildError> {
    let (graph, remap) = doc
        .to_graph_with_remap()
        .map_err(|e| BuildError::Project(e.to_string()))?;

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    for entry in &doc.nodes {
        let kind = &entry.node.kind;
        let descriptor = registry
            .get(kind)
            .ok_or_else(|| BuildError::UnknownKind(kind.clone()))?;
        // An override wins over the document's stored values; a node with neither
        // falls back to the registry defaults (empty slice).
        let node_values = overrides
            .get(&entry.id)
            .map_or(entry.values.as_slice(), Vec::as_slice);
        let runtime_id = remap[&entry.id];
        procs.insert(runtime_id, descriptor.processor_with(ctx, node_values));
    }

    // Sink: the last terminal audio node in document order, addressed by its runtime id
    // (deterministic).
    let sink = doc
        .nodes
        .iter()
        .rev()
        .filter(|entry| {
            entry
                .node
                .outputs
                .iter()
                .any(|p| p.signal == SignalType::Audio)
        })
        .map(|entry| remap[&entry.id])
        .find(|&runtime_id| !graph.edges().iter().any(|e| e.from_node == runtime_id));

    let engine =
        RtEngine::compile(&graph, procs, frames).map_err(|e| BuildError::Engine(e.to_string()))?;
    Ok((engine, sink))
}

/// The patch the plugin ships with: a note-gated voice, built entirely from the
/// registry so it always matches the current node set.
///
/// Audio path `oscillator → svf → vca.in`; control path `midi-in.gate → adsr → vca.cv`.
/// The VCA is stored biased shut (`bias = 0`, so `out = in · cv`), so the envelope is
/// the only thing that opens it — the instrument is **silent until a note plays**
/// instead of droning the instant the plugin loads.
#[must_use]
pub fn default_patch(registry: &Registry) -> ProjectDoc {
    use vb_core::PortId;

    let mut graph = vb_core::Graph::new();
    let osc = graph.add_node(registry.graph_node("oscillator").expect("builtin"));
    let svf = graph.add_node(registry.graph_node("svf").expect("builtin"));
    let midi = graph.add_node(registry.graph_node("midi-in").expect("builtin"));
    let env = graph.add_node(registry.graph_node("adsr").expect("builtin"));
    let vca = graph.add_node(registry.graph_node("vca").expect("builtin"));

    // Audio path: oscillator → filter → VCA audio input (port 0).
    graph
        .connect(osc, PortId(0), svf, PortId(0))
        .expect("osc → svf");
    graph
        .connect(svf, PortId(0), vca, PortId(0))
        .expect("svf → vca.in");
    // Control path: note gate → envelope → VCA control input (port 1).
    graph
        .connect(midi, PortId(0), env, PortId(0))
        .expect("midi.gate → adsr.gate");
    graph
        .connect(env, PortId(0), vca, PortId(1))
        .expect("adsr.level → vca.cv");

    let mut doc = ProjectDoc::from_graph(&graph);
    // Store the VCA biased shut (registry default is bias = 1, i.e. always open).
    let vca_id = doc
        .nodes
        .iter()
        .find(|e| e.node.kind == "vca")
        .expect("vca in default patch")
        .id;
    doc.set_node_values(vca_id, vec![0.0, 1.0]); // bias = 0, amount = 1
    doc
}

#[cfg(test)]
mod tests {
    use vb_core::{Graph, GraphNode, Port, ProcessContext};

    use super::*;

    /// An oscillator → gain document (structure only), for exercising stored values
    /// on a node independently of the default patch's shape.
    fn osc_gain_doc(registry: &Registry) -> ProjectDoc {
        use vb_core::PortId;
        let mut graph = Graph::new();
        let osc = graph.add_node(registry.graph_node("oscillator").expect("builtin"));
        let amp = graph.add_node(registry.graph_node("gain").expect("builtin"));
        graph
            .connect(osc, PortId(0), amp, PortId(0))
            .expect("osc → gain");
        ProjectDoc::from_graph(&graph)
    }

    fn process_once(engine: &mut RtEngine, sink: NodeId) -> Vec<f32> {
        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        engine
            .node_output(sink)
            .unwrap()
            .channel(0)
            .unwrap()
            .to_vec()
    }

    #[test]
    fn the_default_patch_is_silent_at_rest() {
        // With no note held (a fresh, empty note source) the VCA stays shut, so the
        // instrument does not drone the instant it loads.
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let (mut engine, sink) =
            engine_from_doc(&doc, &registry, &BuildContext::default(), 256).expect("compiles");
        let sink = sink.expect("terminal vca is the sink");
        let out = process_once(&mut engine, sink);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            out.iter().all(|&s| s.abs() < 1e-6),
            "default patch must be silent until a note plays"
        );
    }

    #[test]
    fn the_default_patch_opens_on_a_note() {
        // Hold a note in the build context's note source: the gate rises, the envelope
        // opens the VCA, and the voice becomes audible.
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let ctx = BuildContext::default();
        ctx.notes.note_on(69, 127);
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256).expect("compiles");
        let sink = sink.expect("terminal vca is the sink");
        let out = process_once(&mut engine, sink);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            out.iter().any(|&s| s.abs() > 0.001),
            "a held note must make the default patch audible"
        );
    }

    #[test]
    fn stored_node_values_pin_the_sound_without_overrides() {
        // A preset is structure AND knob settings: a gain of 0.0 stored on the
        // terminal node must silence the patch through the plain `engine_from_doc`
        // path — no host-automation overrides supplied.
        let registry = Registry::with_builtins();
        let mut doc = osc_gain_doc(&registry);
        // The gain node is the terminal (its output feeds nothing).
        let gain_id = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "gain")
            .expect("osc_gain_doc has a gain")
            .id;
        doc.set_node_values(gain_id, vec![0.0]);

        let (mut engine, sink) =
            engine_from_doc(&doc, &registry, &BuildContext::default(), 256).expect("compiles");
        let sink = sink.expect("terminal gain is the sink");
        let out = process_once(&mut engine, sink);
        assert!(
            out.iter().all(|&s| s.abs() < 1e-6),
            "stored gain=0 must silence the patch"
        );
    }

    #[test]
    fn unknown_kinds_are_reported_by_name() {
        let mut graph = Graph::new();
        graph.add_node(GraphNode::new(
            "mystery",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let doc = ProjectDoc::from_graph(&graph);
        let result = engine_from_doc(
            &doc,
            &Registry::with_builtins(),
            &BuildContext::default(),
            64,
        );
        match result {
            Err(BuildError::UnknownKind(kind)) => assert_eq!(kind, "mystery"),
            Err(other) => panic!("wrong error: {other}"),
            Ok(_) => panic!("mystery kind must not build"),
        }
    }

    #[test]
    fn round_trips_through_ron_before_building() {
        // The path the plugin actually takes: doc → RON string → doc → engine.
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let ron = vb_project::to_ron(&doc).expect("serializes");
        let restored = vb_project::open_ron(&ron).expect("parses and validates");
        let built = engine_from_doc(&restored, &registry, &BuildContext::default(), 128);
        assert!(built.is_ok());
    }

    #[test]
    fn stored_values_survive_ron_and_still_pin_the_sound() {
        // The exact path a factory preset takes: a patch carrying knob settings is
        // saved to a `.vbp` RON file, reopened, and must sound the same — here a
        // stored gain of 0.0 keeps the reopened patch silent with no overrides.
        let registry = Registry::with_builtins();
        let mut doc = osc_gain_doc(&registry);
        let gain_id = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "gain")
            .expect("osc_gain_doc has a gain")
            .id;
        doc.set_node_values(gain_id, vec![0.0]);

        let ron = vb_project::to_ron(&doc).expect("serializes");
        let restored = vb_project::open_ron(&ron).expect("parses and validates");

        let (mut engine, sink) =
            engine_from_doc(&restored, &registry, &BuildContext::default(), 256).expect("compiles");
        let sink = sink.expect("terminal gain is the sink");
        let out = process_once(&mut engine, sink);
        assert!(
            out.iter().all(|&s| s.abs() < 1e-6),
            "a preset's stored gain=0 must stay silent across a RON round trip"
        );
    }
}
