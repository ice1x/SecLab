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

/// Compiles `doc` into an engine plus the sink node (a terminal audio output).
///
/// Format v1 carries no parameter values, so processors are built with their
/// defaults; parameter routing arrives with host automation (000065).
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
    let graph = doc
        .to_graph()
        .map_err(|e| BuildError::Project(e.to_string()))?;

    let mut procs: HashMap<NodeId, Box<dyn Processor>> = HashMap::new();
    for id in graph.node_ids() {
        let kind = &graph.node(id).expect("listed id exists").kind;
        let descriptor = registry
            .get(kind)
            .ok_or_else(|| BuildError::UnknownKind(kind.clone()))?;
        procs.insert(id, descriptor.processor_with(ctx, &[]));
    }

    // Sink: the last terminal audio node in document order (deterministic).
    let sink = doc
        .nodes
        .iter()
        .rev()
        .find(|entry| {
            entry
                .node
                .outputs
                .iter()
                .any(|p| p.signal == SignalType::Audio)
                && !graph.edges().iter().any(|e| e.from_node == entry.id)
        })
        .map(|entry| entry.id);

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
}
