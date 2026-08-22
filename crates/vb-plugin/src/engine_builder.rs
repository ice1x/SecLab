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

/// The patch the plugin ships with: oscillator → svf → gain, built entirely from
/// the registry so it always matches the current node set.
#[must_use]
pub fn default_patch(registry: &Registry) -> ProjectDoc {
    let mut graph = vb_core::Graph::new();
    let osc = graph.add_node(registry.graph_node("oscillator").expect("builtin"));
    let svf = graph.add_node(registry.graph_node("svf").expect("builtin"));
    let amp = graph.add_node(registry.graph_node("gain").expect("builtin"));
    graph
        .connect(osc, vb_core::PortId(0), svf, vb_core::PortId(0))
        .expect("osc → svf");
    graph
        .connect(svf, vb_core::PortId(0), amp, vb_core::PortId(0))
        .expect("svf → gain");
    ProjectDoc::from_graph(&graph)
}

#[cfg(test)]
mod tests {
    use vb_core::{Graph, GraphNode, Port, ProcessContext};

    use super::*;

    #[test]
    fn the_default_patch_compiles_and_makes_sound() {
        let registry = Registry::with_builtins();
        let doc = default_patch(&registry);
        let (mut engine, sink) =
            engine_from_doc(&doc, &registry, &BuildContext::default(), 256).expect("compiles");
        let sink = sink.expect("terminal gain is the sink");

        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let out = engine.node_output(sink).unwrap().channel(0).unwrap();
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            out.iter().any(|&s| s.abs() > 0.001),
            "default patch is audible"
        );
    }

    #[test]
    fn stored_node_values_pin_the_sound_without_overrides() {
        // A preset is structure AND knob settings: a gain of 0.0 stored on the
        // terminal node must silence the patch through the plain `engine_from_doc`
        // path — no host-automation overrides supplied.
        let registry = Registry::with_builtins();
        let mut doc = default_patch(&registry);
        // The gain node is the terminal (its output feeds nothing).
        let gain_id = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "gain")
            .expect("default patch has a gain")
            .id;
        doc.set_node_values(gain_id, vec![0.0]);

        let (mut engine, sink) =
            engine_from_doc(&doc, &registry, &BuildContext::default(), 256).expect("compiles");
        let sink = sink.expect("terminal gain is the sink");
        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let out = engine.node_output(sink).unwrap().channel(0).unwrap();
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
        let mut doc = default_patch(&registry);
        let gain_id = doc
            .nodes
            .iter()
            .find(|e| e.node.kind == "gain")
            .expect("default patch has a gain")
            .id;
        doc.set_node_values(gain_id, vec![0.0]);

        let ron = vb_project::to_ron(&doc).expect("serializes");
        let restored = vb_project::open_ron(&ron).expect("parses and validates");

        let (mut engine, sink) =
            engine_from_doc(&restored, &registry, &BuildContext::default(), 256).expect("compiles");
        let sink = sink.expect("terminal gain is the sink");
        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let out = engine.node_output(sink).unwrap().channel(0).unwrap();
        assert!(
            out.iter().all(|&s| s.abs() < 1e-6),
            "a preset's stored gain=0 must stay silent across a RON round trip"
        );
    }
}
