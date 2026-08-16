//! Node registry: create nodes (their ports and their processor) by kind name.
//!
//! The editor and project loader need to turn a kind string like `"oscillator"` into
//! both a [`GraphNode`] (ports for wiring) and a boxed [`Processor`] (the DSP behaviour)
//! without hard-coding every node. This registry maps kind → factories and carries the
//! metadata the UI palette needs (kind, category).

use std::collections::BTreeMap;

use vb_core::{GraphNode, Parameter, Port, Processor, SignalType};

use crate::envelope::Adsr;
use crate::filter::{Biquad, FilterKind};
use crate::lfo::{Division, Lfo};
use crate::math::{Add, Clamp, Gain, Mix, Mul};
use crate::midi::{BuildContext, MidiIn};
use crate::noise::{Noise, NoiseKind};
use crate::oscillator::{Oscillator, Waveform};

/// Metadata and factories for one node kind.
pub struct NodeDescriptor {
    kind: &'static str,
    category: &'static str,
    make_node: fn() -> GraphNode,
    make_processor: fn() -> Box<dyn Processor>,
    make_params: fn() -> Vec<Parameter>,
    make_processor_with: fn(&BuildContext, &[f32]) -> Box<dyn Processor>,
}

impl NodeDescriptor {
    /// Creates a descriptor from its kind, UI category and factories.
    #[must_use]
    pub fn new(
        kind: &'static str,
        category: &'static str,
        make_node: fn() -> GraphNode,
        make_processor: fn() -> Box<dyn Processor>,
        make_params: fn() -> Vec<Parameter>,
        make_processor_with: fn(&BuildContext, &[f32]) -> Box<dyn Processor>,
    ) -> Self {
        Self {
            kind,
            category,
            make_node,
            make_processor,
            make_params,
            make_processor_with,
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

    /// The editable parameters of this kind (name, range, default), in UI order.
    ///
    /// Enum-like choices (waveform, filter kind) are modelled as stepped float
    /// parameters until dedicated control widgets land (000049).
    #[must_use]
    pub fn params(&self) -> Vec<Parameter> {
        (self.make_params)()
    }

    /// Builds a boxed [`Processor`] configured from `values`, one per parameter in
    /// [`NodeDescriptor::params`] order. Missing values fall back to the parameter's
    /// default, so a stale or truncated value list still builds a working processor.
    /// Context-dependent kinds (`midi-in`) attach to the live inputs in `ctx`.
    #[must_use]
    pub fn processor_with(&self, ctx: &BuildContext, values: &[f32]) -> Box<dyn Processor> {
        (self.make_processor_with)(ctx, values)
    }
}

/// `values[i]` or `default` — keeps `processor_with` total on short value lists.
fn arg(values: &[f32], i: usize, default: f32) -> f32 {
    values.get(i).copied().unwrap_or(default)
}

/// Maps a stepped float to a [`Waveform`] (0 sine, 1 saw, 2 square, 3 triangle).
fn waveform_from(v: f32) -> Waveform {
    match v.round() as i32 {
        1 => Waveform::Saw,
        2 => Waveform::Square,
        3 => Waveform::Triangle,
        _ => Waveform::Sine,
    }
}

/// Maps a stepped float to a [`Division`] (0 whole … 4 sixteenth).
fn division_from(v: f32) -> Division {
    match v.round() as i32 {
        0 => Division::Whole,
        1 => Division::Half,
        3 => Division::Eighth,
        4 => Division::Sixteenth,
        _ => Division::Quarter,
    }
}

/// Maps a stepped float to a [`NoiseKind`] (0 white, 1 pink).
fn noise_kind_from(v: f32) -> NoiseKind {
    if v.round() as i32 == 1 {
        NoiseKind::Pink
    } else {
        NoiseKind::White
    }
}

/// Maps a stepped float to a [`FilterKind`] (0 LP, 1 HP, 2 BP, 3 notch).
fn filter_kind_from(v: f32) -> FilterKind {
    match v.round() as i32 {
        1 => FilterKind::HighPass,
        2 => FilterKind::BandPass,
        3 => FilterKind::Notch,
        _ => FilterKind::LowPass,
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
            || {
                vec![
                    Parameter::new("waveform", 0.0, 3.0, 0.0),
                    Parameter::new("freq_hz", 20.0, 20_000.0, 440.0),
                ]
            },
            |_, v| {
                Box::new(Oscillator::new(
                    waveform_from(arg(v, 0, 0.0)),
                    arg(v, 1, 440.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "biquad",
            "filter",
            || GraphNode::new("biquad", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Biquad::new(FilterKind::LowPass, 1_000.0, 0.707)),
            || {
                vec![
                    Parameter::new("kind", 0.0, 3.0, 0.0),
                    Parameter::new("cutoff_hz", 20.0, 20_000.0, 1_000.0),
                    Parameter::new("q", 0.1, 10.0, 0.707),
                ]
            },
            |_, v| {
                Box::new(Biquad::new(
                    filter_kind_from(arg(v, 0, 0.0)),
                    arg(v, 1, 1_000.0),
                    arg(v, 2, 0.707),
                ))
            },
        ),
        NodeDescriptor::new(
            "gain",
            "math",
            || GraphNode::new("gain", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Gain::new(1.0)),
            || vec![Parameter::new("gain", 0.0, 2.0, 1.0)],
            |_, v| Box::new(Gain::new(arg(v, 0, 1.0))),
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
            Vec::new,
            |_, _| Box::new(Add),
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
            Vec::new,
            |_, _| Box::new(Mul),
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
            || vec![Parameter::new("t", 0.0, 1.0, 0.5)],
            |_, v| Box::new(Mix::new(arg(v, 0, 0.5))),
        ),
        NodeDescriptor::new(
            "clamp",
            "math",
            || GraphNode::new("clamp", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Clamp::new(-1.0, 1.0)),
            || {
                vec![
                    Parameter::new("min", -1.0, 1.0, -1.0),
                    Parameter::new("max", -1.0, 1.0, 1.0),
                ]
            },
            |_, v| Box::new(Clamp::new(arg(v, 0, -1.0), arg(v, 1, 1.0))),
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
            || {
                vec![
                    Parameter::new("attack_s", 0.0, 5.0, 0.01),
                    Parameter::new("decay_s", 0.0, 5.0, 0.1),
                    Parameter::new("sustain", 0.0, 1.0, 0.7),
                    Parameter::new("release_s", 0.0, 5.0, 0.2),
                ]
            },
            |_, v| {
                Box::new(Adsr::new(
                    arg(v, 0, 0.01),
                    arg(v, 1, 0.1),
                    arg(v, 2, 0.7),
                    arg(v, 3, 0.2),
                ))
            },
        ),
        NodeDescriptor::new(
            "lfo",
            "modulation",
            || {
                GraphNode::new(
                    "lfo",
                    vec![],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(Lfo::new(Waveform::Sine, 2.0)),
            || {
                vec![
                    Parameter::new("waveform", 0.0, 3.0, 0.0),
                    Parameter::new("sync", 0.0, 1.0, 0.0),
                    Parameter::new("freq_hz", 0.01, 50.0, 2.0),
                    Parameter::new("bpm", 20.0, 300.0, 120.0),
                    Parameter::new("division", 0.0, 4.0, 2.0),
                ]
            },
            |_, v| {
                let waveform = waveform_from(arg(v, 0, 0.0));
                if arg(v, 1, 0.0).round() as i32 == 1 {
                    Box::new(Lfo::synced(
                        waveform,
                        arg(v, 3, 120.0),
                        division_from(arg(v, 4, 2.0)),
                    ))
                } else {
                    Box::new(Lfo::new(waveform, arg(v, 2, 2.0)))
                }
            },
        ),
        NodeDescriptor::new(
            "noise",
            "generator",
            || GraphNode::new("noise", vec![], vec![audio_out("out")]),
            || Box::new(Noise::new(NoiseKind::White, 0.5)),
            || {
                vec![
                    Parameter::new("kind", 0.0, 1.0, 0.0),
                    Parameter::new("level", 0.0, 1.0, 0.5),
                ]
            },
            |_, v| Box::new(Noise::new(noise_kind_from(arg(v, 0, 0.0)), arg(v, 1, 0.5))),
        ),
        NodeDescriptor::new(
            "midi-in",
            "input",
            || {
                GraphNode::new(
                    "midi-in",
                    vec![],
                    vec![
                        Port::output("gate", SignalType::Control),
                        Port::output("pitch_hz", SignalType::Control),
                        Port::output("velocity", SignalType::Control),
                    ],
                )
            },
            // Without a context there is nothing to listen to: a fresh silent source.
            || Box::new(MidiIn::new(crate::midi::NoteSource::new())),
            Vec::new,
            |ctx, _| Box::new(MidiIn::new(ctx.notes.clone())),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::Registry;
    use crate::midi::BuildContext;
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

    #[test]
    fn every_builtin_exposes_well_formed_params() {
        let r = Registry::with_builtins();
        for kind in r.kinds() {
            let params = r.get(kind).unwrap().params();
            for p in &params {
                assert!(p.min <= p.max, "{kind}.{}: inverted range", p.name);
                assert!(
                    (p.min..=p.max).contains(&p.default),
                    "{kind}.{}: default out of range",
                    p.name
                );
                assert!(!p.name.is_empty(), "{kind}: unnamed parameter");
            }
        }
    }

    #[test]
    fn param_metadata_matches_the_processors_defaults() {
        let r = Registry::with_builtins();
        // Spot-check the kinds whose constructors carry the same defaults.
        let osc: Vec<_> = r.get("oscillator").unwrap().params();
        assert_eq!(osc[1].name, "freq_hz");
        assert!((osc[1].default - 440.0).abs() < f32::EPSILON);

        let biquad = r.get("biquad").unwrap().params();
        assert_eq!(biquad.len(), 3);
        assert!((biquad[1].default - 1_000.0).abs() < f32::EPSILON);

        // Pure combinators have nothing to edit.
        assert!(r.get("add").unwrap().params().is_empty());
        assert!(r.get("mul").unwrap().params().is_empty());
    }

    /// Runs a lone source processor for one block and returns its output.
    fn run_source(mut p: Box<dyn Processor>, frames: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
        };
        let mut out = vec![0.0; frames];
        p.process(&ctx, &[], &mut [&mut out]);
        out
    }

    /// Runs a one-in/one-out processor over `input` and returns its output.
    fn run_effect(mut p: Box<dyn Processor>, input: &[f32]) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames: input.len(),
        };
        let mut out = vec![0.0; input.len()];
        p.process(&ctx, &[input], &mut [&mut out]);
        out
    }

    #[test]
    fn processor_with_applies_param_values() {
        let r = Registry::with_builtins();
        let input = vec![1.0_f32; 64];

        // gain 0.5 halves the signal; default gain passes it through.
        let halved = run_effect(
            r.get("gain")
                .unwrap()
                .processor_with(&BuildContext::default(), &[0.5]),
            &input,
        );
        assert!(halved.iter().all(|&s| (s - 0.5).abs() < 1e-6));
        let default = run_effect(r.get("gain").unwrap().processor(), &input);
        assert!(default.iter().all(|&s| (s - 1.0).abs() < 1e-6));

        // A 100 Hz sine crosses zero going upward once per 480 samples at 48 kHz;
        // a 1000 Hz one ten times as often — the freq value audibly matters.
        let slow = run_source(
            r.get("oscillator")
                .unwrap()
                .processor_with(&BuildContext::default(), &[0.0, 100.0]),
            4_800,
        );
        let fast = run_source(
            r.get("oscillator")
                .unwrap()
                .processor_with(&BuildContext::default(), &[0.0, 1_000.0]),
            4_800,
        );
        let crossings = |xs: &[f32]| xs.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!(crossings(&fast) > crossings(&slow) * 5);
    }

    #[test]
    fn processor_with_falls_back_to_defaults_on_short_values() {
        let r = Registry::with_builtins();
        let input = vec![1.0_f32; 32];
        // No values at all: behaves like the default constructor.
        let out = run_effect(
            r.get("gain")
                .unwrap()
                .processor_with(&BuildContext::default(), &[]),
            &input,
        );
        assert!(out.iter().all(|&s| (s - 1.0).abs() < 1e-6));
        // Enum mappers accept any float without panicking.
        let _ = r
            .get("biquad")
            .unwrap()
            .processor_with(&BuildContext::default(), &[99.0]);
        let _ = r
            .get("oscillator")
            .unwrap()
            .processor_with(&BuildContext::default(), &[-7.0, 55.0]);
    }
}
