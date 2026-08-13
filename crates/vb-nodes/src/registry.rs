//! Node registry: create nodes (their ports and their processor) by kind name.
//!
//! The editor and project loader need to turn a kind string like `"oscillator"` into
//! both a [`GraphNode`] (ports for wiring) and a boxed [`Processor`] (the DSP behaviour)
//! without hard-coding every node. This registry maps kind → factories and carries the
//! metadata the UI palette needs (kind, category).

use std::collections::BTreeMap;

use vb_core::{GraphNode, Port, Processor, SignalType};

use crate::envelope::Adsr;
use crate::filter::{Biquad, FilterKind};
use crate::math::{Add, Clamp, Gain, Mix, Mul};
use crate::oscillator::{Oscillator, Waveform};

/// Metadata and factories for one node kind.
pub struct NodeDescriptor {
    kind: &'static str,
    category: &'static str,
    make_node: fn() -> GraphNode,
    make_processor: fn() -> Box<dyn Processor>,
}

impl NodeDescriptor {
    /// Creates a descriptor from its kind, UI category and factories.
    #[must_use]
    pub fn new(
        kind: &'static str,
        category: &'static str,
        make_node: fn() -> GraphNode,
        make_processor: fn() -> Box<dyn Processor>,
    ) -> Self {
        Self {
            kind,
            category,
            make_node,
            make_processor,
        }
    }

    /// The unique kind name (e.g. `"oscillator"`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// The UI category (e.g. `"generator"`).
    #[must_use]
    pub fn category(&self) -> &'static str {
        self.category
    }

    /// Builds a fresh [`GraphNode`] (its ports) for this kind.
    #[must_use]
    pub fn graph_node(&self) -> GraphNode {
        (self.make_node)()
    }

    /// Builds a fresh boxed [`Processor`] with default parameters.
    #[must_use]
    pub fn processor(&self) -> Box<dyn Processor> {
        (self.make_processor)()
    }
}

/// A collection of node kinds keyed by name.
pub struct Registry {
    by_kind: BTreeMap<&'static str, NodeDescriptor>,
}

impl Registry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_kind: BTreeMap::new(),
        }
    }

    /// Creates a registry pre-populated with every built-in node.
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        for d in builtins() {
            r.register(d);
        }
        r
    }

    /// Registers (or replaces) a descriptor.
    pub fn register(&mut self, descriptor: NodeDescriptor) {
        self.by_kind.insert(descriptor.kind, descriptor);
    }

    /// Looks up a descriptor by kind.
    #[must_use]
    pub fn get(&self, kind: &str) -> Option<&NodeDescriptor> {
        self.by_kind.get(kind)
    }

    /// Builds a [`GraphNode`] for `kind`, if registered.
    #[must_use]
    pub fn graph_node(&self, kind: &str) -> Option<GraphNode> {
        self.get(kind).map(NodeDescriptor::graph_node)
    }

    /// Builds a boxed [`Processor`] for `kind`, if registered.
    #[must_use]
    pub fn processor(&self, kind: &str) -> Option<Box<dyn Processor>> {
        self.get(kind).map(NodeDescriptor::processor)
    }

    /// Iterates registered kind names in sorted order.
    pub fn kinds(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_kind.keys().copied()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

fn audio_in(name: &'static str) -> Port {
    Port::input(name, SignalType::Audio)
}

fn audio_out(name: &'static str) -> Port {
    Port::output(name, SignalType::Audio)
}

/// The built-in node descriptors.
fn builtins() -> Vec<NodeDescriptor> {
    vec![
        NodeDescriptor::new(
            "oscillator",
            "generator",
            || GraphNode::new("oscillator", vec![], vec![audio_out("out")]),
            || Box::new(Oscillator::new(Waveform::Sine, 440.0)),
        ),
        NodeDescriptor::new(
            "biquad",
            "filter",
            || GraphNode::new("biquad", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Biquad::new(FilterKind::LowPass, 1_000.0, 0.707)),
        ),
        NodeDescriptor::new(
            "gain",
            "math",
            || GraphNode::new("gain", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Gain::new(1.0)),
        ),
        NodeDescriptor::new(
            "add",
            "math",
            || {
                GraphNode::new(
                    "add",
                    vec![audio_in("a"), audio_in("b")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Add),
        ),
        NodeDescriptor::new(
            "mul",
            "math",
            || {
                GraphNode::new(
                    "mul",
                    vec![audio_in("a"), audio_in("b")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Mul),
        ),
        NodeDescriptor::new(
            "mix",
            "math",
            || {
                GraphNode::new(
                    "mix",
                    vec![audio_in("a"), audio_in("b")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Mix::new(0.5)),
        ),
        NodeDescriptor::new(
            "clamp",
            "math",
            || GraphNode::new("clamp", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Clamp::new(-1.0, 1.0)),
        ),
        NodeDescriptor::new(
            "adsr",
            "envelope",
            || {
                GraphNode::new(
                    "adsr",
                    vec![Port::input("gate", SignalType::Control)],
                    vec![Port::output("level", SignalType::Control)],
                )
            },
            || Box::new(Adsr::new(0.01, 0.1, 0.7, 0.2)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::Registry;
    use std::collections::HashMap;
    use vb_core::{Engine, Graph, PortId, ProcessContext, Processor};

    #[test]
    fn builtins_are_registered() {
        let r = Registry::with_builtins();
        let kinds: Vec<&str> = r.kinds().collect();
        for expected in [
            "oscillator",
            "biquad",
            "gain",
            "add",
            "mul",
            "mix",
            "clamp",
            "adsr",
        ] {
            assert!(kinds.contains(&expected), "missing kind {expected}");
        }
    }

    #[test]
    fn graph_node_has_expected_ports() {
        let r = Registry::with_builtins();
        let osc = r.graph_node("oscillator").unwrap();
        assert_eq!(osc.inputs.len(), 0);
        assert_eq!(osc.outputs.len(), 1);

        let add = r.graph_node("add").unwrap();
        assert_eq!(add.inputs.len(), 2);
    }

    #[test]
    fn unknown_kind_is_none() {
        let r = Registry::with_builtins();
        assert!(r.get("does-not-exist").is_none());
        assert!(r.graph_node("nope").is_none());
        assert!(r.processor("nope").is_none());
    }

    #[test]
    fn default_is_populated() {
        assert!(Registry::default().kinds().count() >= 8);
    }

    #[test]
    fn registry_built_nodes_run_in_the_engine() {
        // Build oscillator -> gain purely from the registry, then run it.
        let r = Registry::with_builtins();
        let mut g = Graph::new();
        let osc = g.add_node(r.graph_node("oscillator").unwrap());
        let amp = g.add_node(r.graph_node("gain").unwrap());
        g.connect(osc, PortId(0), amp, PortId(0)).unwrap();

        let mut procs: HashMap<_, Box<dyn Processor>> = HashMap::new();
        procs.insert(osc, r.processor("oscillator").unwrap());
        procs.insert(amp, r.processor("gain").unwrap());

        let mut engine = Engine::compile(&g, procs, 128).unwrap();
        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 128,
        });

        let out = engine.node_output(amp).unwrap().channel(0).unwrap();
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(
            out.iter().any(|&s| s.abs() > 0.01),
            "registry-built patch should produce sound"
        );
    }
}
