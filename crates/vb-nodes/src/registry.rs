//! Node registry: create nodes (their ports and their processor) by kind name.
//!
//! The editor and project loader need to turn a kind string like `"oscillator"` into
//! both a [`GraphNode`] (ports for wiring) and a boxed [`Processor`] (the DSP behaviour)
//! without hard-coding every node. This registry maps kind → factories and carries the
//! metadata the UI palette needs (kind, category).

use std::collections::BTreeMap;

use vb_core::{GraphNode, Parameter, Port, Processor, SignalType};

use crate::ad_env::AdEnv;
use crate::additive::Additive;
use crate::ahr_env::AhrEnv;
use crate::ar_env::ArEnv;
use crate::bitcrusher::Bitcrusher;
use crate::chorus::Chorus;
use crate::clock::Clock;
use crate::clock_divider::ClockDivider;
use crate::comparator::Comparator;
use crate::compressor::Compressor;
use crate::dc_blocker::DcBlocker;
use crate::delay::{FbDelay, MAX_DELAY_S};
use crate::env_follower::EnvFollower;
use crate::envelope::Adsr;
use crate::euclid::Euclid;
use crate::filter::{Biquad, FilterKind};
use crate::fm_op::FmOp;
use crate::gate::Gate;
use crate::glide::Glide;
use crate::hard_sync::HardSync;
use crate::io::{AudioInput, AudioOutput, MAX_IO_CHANNELS};
use crate::karplus::Karplus;
use crate::ladder::Ladder;
use crate::lfo::{Division, Lfo};
use crate::limiter::Limiter;
use crate::math::{Add, Attenuvert, Clamp, Gain, Max, Min, Mix, Mul, Rectify, Sub};
use crate::midi::{BuildContext, MidiIn};
use crate::noise::{Noise, NoiseKind};
use crate::oscillator::{Oscillator, Waveform};
use crate::pan::Pan;
use crate::phaser::Phaser;
use crate::ping_pong::PingPong;
use crate::pitch_osc::PitchOsc;
use crate::poly::{KeyGate, PolyVoiceIn, MAX_VOICES};
use crate::pwm::Pwm;
use crate::quantizer::Quantizer;
use crate::reverb::Reverb;
use crate::ring_mod::RingMod;
use crate::routing::{Merger, Selector, Splitter};
use crate::sample_hold::SampleHold;
use crate::slew::Slew;
use crate::step_sequencer::StepSequencer;
use crate::sub_osc::SubOsc;
use crate::supersaw::Supersaw;
use crate::svf::Svf;
use crate::tone::Tone;
use crate::tremolo::Tremolo;
use crate::vca::Vca;
use crate::vcf::Vcf;
use crate::vco::Vco;
use crate::wavefolder::Wavefolder;
use crate::waveshaper::{Shape, Waveshaper};
use crate::wavetable::Wavetable;
use crate::width::Width;
use crate::xfade::Xfade;

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

/// Maps a stepped float to a [`Shape`] (0 saturate, 1 hard clip, 2 foldback).
fn shape_from(v: f32) -> Shape {
    match v.round() as i32 {
        1 => Shape::HardClip,
        2 => Shape::Foldback,
        _ => Shape::Saturate,
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

fn control_in(name: &'static str) -> Port {
    Port::input(name, SignalType::Control)
}

fn control_out(name: &'static str) -> Port {
    Port::output(name, SignalType::Control)
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
            "svf",
            "filter",
            || {
                GraphNode::new(
                    "svf",
                    vec![audio_in("in")],
                    vec![
                        audio_out("lowpass"),
                        audio_out("bandpass"),
                        audio_out("highpass"),
                    ],
                )
            },
            || Box::new(Svf::new(1_000.0, 0.707)),
            || {
                vec![
                    Parameter::new("cutoff_hz", 20.0, 20_000.0, 1_000.0),
                    Parameter::new("q", 0.1, 20.0, 0.707),
                ]
            },
            |_, v| Box::new(Svf::new(arg(v, 0, 1_000.0), arg(v, 1, 0.707))),
        ),
        NodeDescriptor::new(
            "dc-blocker",
            "filter",
            || GraphNode::new("dc-blocker", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(DcBlocker::new(0.995)),
            || vec![Parameter::new("r", 0.9, 0.9999, 0.995)],
            |_, v| Box::new(DcBlocker::new(arg(v, 0, 0.995))),
        ),
        NodeDescriptor::new(
            "tone",
            "filter",
            || GraphNode::new("tone", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Tone::new(0.0, 700.0)),
            || {
                vec![
                    Parameter::new("tilt", -1.0, 1.0, 0.0),
                    Parameter::new("pivot_hz", 100.0, 8_000.0, 700.0),
                ]
            },
            |_, v| Box::new(Tone::new(arg(v, 0, 0.0), arg(v, 1, 700.0))),
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
            "sub",
            "math",
            || {
                GraphNode::new(
                    "sub",
                    vec![audio_in("a"), audio_in("b")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Sub),
            Vec::new,
            |_, _| Box::new(Sub),
        ),
        NodeDescriptor::new(
            "min",
            "math",
            || {
                GraphNode::new(
                    "min",
                    vec![audio_in("a"), audio_in("b")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Min),
            Vec::new,
            |_, _| Box::new(Min),
        ),
        NodeDescriptor::new(
            "max",
            "math",
            || {
                GraphNode::new(
                    "max",
                    vec![audio_in("a"), audio_in("b")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Max),
            Vec::new,
            |_, _| Box::new(Max),
        ),
        NodeDescriptor::new(
            "rectify",
            "math",
            || GraphNode::new("rectify", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Rectify),
            Vec::new,
            |_, _| Box::new(Rectify),
        ),
        NodeDescriptor::new(
            "attenuvert",
            "math",
            || GraphNode::new("attenuvert", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Attenuvert::new(1.0, 0.0)),
            || {
                vec![
                    Parameter::new("scale", -2.0, 2.0, 1.0),
                    Parameter::new("offset", -1.0, 1.0, 0.0),
                ]
            },
            |_, v| Box::new(Attenuvert::new(arg(v, 0, 1.0), arg(v, 1, 0.0))),
        ),
        // Control-rate twins of `add` / `attenuvert`: the audio-typed ones cannot carry a
        // control signal (`Audio` is not `compatible_with` `Control`), so summing/scaling CV
        // — e.g. an envelope + an LFO into a filter cutoff — needs these. Same processors,
        // Control ports.
        NodeDescriptor::new(
            "cv-add",
            "math",
            || {
                GraphNode::new(
                    "cv-add",
                    vec![control_in("a"), control_in("b")],
                    vec![control_out("out")],
                )
            },
            || Box::new(Add),
            Vec::new,
            |_, _| Box::new(Add),
        ),
        NodeDescriptor::new(
            "cv-attenuvert",
            "math",
            || {
                GraphNode::new(
                    "cv-attenuvert",
                    vec![control_in("in")],
                    vec![control_out("out")],
                )
            },
            || Box::new(Attenuvert::new(1.0, 0.0)),
            || {
                vec![
                    Parameter::new("scale", -2.0, 2.0, 1.0),
                    Parameter::new("offset", -1.0, 1.0, 0.0),
                ]
            },
            |_, v| Box::new(Attenuvert::new(arg(v, 0, 1.0), arg(v, 1, 0.0))),
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
            "xfade",
            "math",
            || {
                GraphNode::new(
                    "xfade",
                    vec![
                        audio_in("a"),
                        audio_in("b"),
                        Port::input("mix", SignalType::Control),
                    ],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Xfade),
            Vec::new,
            |_, _| Box::new(Xfade),
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
            "ad",
            "envelope",
            || {
                GraphNode::new(
                    "ad",
                    vec![Port::input("gate", SignalType::Control)],
                    vec![Port::output("level", SignalType::Control)],
                )
            },
            || Box::new(AdEnv::new(0.005, 0.15)),
            || {
                vec![
                    Parameter::new("attack_s", 0.0, 5.0, 0.005),
                    Parameter::new("decay_s", 0.0, 5.0, 0.15),
                ]
            },
            |_, v| Box::new(AdEnv::new(arg(v, 0, 0.005), arg(v, 1, 0.15))),
        ),
        NodeDescriptor::new(
            "ar",
            "envelope",
            || {
                GraphNode::new(
                    "ar",
                    vec![Port::input("gate", SignalType::Control)],
                    vec![Port::output("level", SignalType::Control)],
                )
            },
            || Box::new(ArEnv::new(0.01, 0.2)),
            || {
                vec![
                    Parameter::new("attack_s", 0.0, 5.0, 0.01),
                    Parameter::new("release_s", 0.0, 5.0, 0.2),
                ]
            },
            |_, v| Box::new(ArEnv::new(arg(v, 0, 0.01), arg(v, 1, 0.2))),
        ),
        NodeDescriptor::new(
            "ahr",
            "envelope",
            || {
                GraphNode::new(
                    "ahr",
                    vec![Port::input("gate", SignalType::Control)],
                    vec![Port::output("level", SignalType::Control)],
                )
            },
            || Box::new(AhrEnv::new(0.01, 0.1, 0.1)),
            || {
                vec![
                    Parameter::new("attack_s", 0.0, 5.0, 0.01),
                    Parameter::new("hold_s", 0.0, 5.0, 0.1),
                    Parameter::new("release_s", 0.0, 5.0, 0.1),
                ]
            },
            |_, v| Box::new(AhrEnv::new(arg(v, 0, 0.01), arg(v, 1, 0.1), arg(v, 2, 0.1))),
        ),
        NodeDescriptor::new(
            "env-follower",
            "modulation",
            || {
                GraphNode::new(
                    "env-follower",
                    vec![audio_in("in")],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(EnvFollower::new(0.005, 0.05)),
            || {
                vec![
                    Parameter::new("attack_s", 0.0, 1.0, 0.005),
                    Parameter::new("release_s", 0.0, 2.0, 0.05),
                ]
            },
            |_, v| Box::new(EnvFollower::new(arg(v, 0, 0.005), arg(v, 1, 0.05))),
        ),
        NodeDescriptor::new(
            "fb-delay",
            "effect",
            || {
                // A feedback boundary: loops wired through this input are legal and
                // arrive one block late.
                GraphNode::feedback_node("fb-delay", vec![audio_in("in")], vec![audio_out("out")])
            },
            || Box::new(FbDelay::new(0.25, 0.4)),
            || {
                vec![
                    Parameter::new("delay_s", 0.001, MAX_DELAY_S, 0.25),
                    Parameter::new("feedback", 0.0, 0.95, 0.4),
                ]
            },
            |_, v| Box::new(FbDelay::new(arg(v, 0, 0.25), arg(v, 1, 0.4))),
        ),
        NodeDescriptor::new(
            "ping-pong",
            "effect",
            || {
                GraphNode::new(
                    "ping-pong",
                    vec![audio_in("in")],
                    vec![audio_out("left"), audio_out("right")],
                )
            },
            || Box::new(PingPong::new(0.25, 0.4, 0.5)),
            || {
                vec![
                    Parameter::new("delay_s", 0.001, crate::ping_pong::MAX_DELAY_S, 0.25),
                    Parameter::new("feedback", 0.0, 0.95, 0.4),
                    Parameter::new("mix", 0.0, 1.0, 0.5),
                ]
            },
            |_, v| {
                Box::new(PingPong::new(
                    arg(v, 0, 0.25),
                    arg(v, 1, 0.4),
                    arg(v, 2, 0.5),
                ))
            },
        ),
        NodeDescriptor::new(
            "width",
            "effect",
            || {
                GraphNode::new(
                    "width",
                    vec![audio_in("left"), audio_in("right")],
                    vec![audio_out("left"), audio_out("right")],
                )
            },
            || Box::new(Width::new(1.0)),
            || vec![Parameter::new("width", 0.0, 2.0, 1.0)],
            |_, v| Box::new(Width::new(arg(v, 0, 1.0))),
        ),
        NodeDescriptor::new(
            "reverb",
            "effect",
            || GraphNode::new("reverb", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Reverb::new(0.5, 0.5, 0.3)),
            || {
                vec![
                    Parameter::new("room_size", 0.0, 1.0, 0.5),
                    Parameter::new("damping", 0.0, 1.0, 0.5),
                    Parameter::new("mix", 0.0, 1.0, 0.3),
                ]
            },
            |_, v| Box::new(Reverb::new(arg(v, 0, 0.5), arg(v, 1, 0.5), arg(v, 2, 0.3))),
        ),
        NodeDescriptor::new(
            "waveshaper",
            "effect",
            || GraphNode::new("waveshaper", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Waveshaper::new(Shape::Saturate, 2.0, 0.8)),
            || {
                vec![
                    Parameter::new("shape", 0.0, 2.0, 0.0),
                    Parameter::new("drive", 0.1, 20.0, 2.0),
                    Parameter::new("level", 0.0, 1.0, 0.8),
                ]
            },
            |_, v| {
                Box::new(Waveshaper::new(
                    shape_from(arg(v, 0, 0.0)),
                    arg(v, 1, 2.0),
                    arg(v, 2, 0.8),
                ))
            },
        ),
        NodeDescriptor::new(
            "wavefolder",
            "effect",
            || GraphNode::new("wavefolder", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Wavefolder::new(1.0, 0.0, 1.0)),
            || {
                vec![
                    Parameter::new("drive", 0.0, 10.0, 1.0),
                    Parameter::new("symmetry", -1.0, 1.0, 0.0),
                    Parameter::new("level", 0.0, 1.0, 1.0),
                ]
            },
            |_, v| {
                Box::new(Wavefolder::new(
                    arg(v, 0, 1.0),
                    arg(v, 1, 0.0),
                    arg(v, 2, 1.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "ring-mod",
            "effect",
            || GraphNode::new("ring-mod", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(RingMod::new(200.0, 1.0)),
            || {
                vec![
                    Parameter::new("freq_hz", 1.0, 8_000.0, 200.0),
                    Parameter::new("mix", 0.0, 1.0, 1.0),
                ]
            },
            |_, v| Box::new(RingMod::new(arg(v, 0, 200.0), arg(v, 1, 1.0))),
        ),
        NodeDescriptor::new(
            "bitcrusher",
            "effect",
            || GraphNode::new("bitcrusher", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Bitcrusher::new(8.0, 1, 1.0)),
            || {
                vec![
                    Parameter::new("bits", 1.0, 16.0, 8.0),
                    Parameter::new("downsample", 1.0, 64.0, 1.0),
                    Parameter::new("mix", 0.0, 1.0, 1.0),
                ]
            },
            |_, v| {
                Box::new(Bitcrusher::new(
                    arg(v, 0, 8.0),
                    arg(v, 1, 1.0).round().max(1.0) as usize,
                    arg(v, 2, 1.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "tremolo",
            "effect",
            || GraphNode::new("tremolo", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Tremolo::new(5.0, 0.5)),
            || {
                vec![
                    Parameter::new("rate_hz", 0.1, 20.0, 5.0),
                    Parameter::new("depth", 0.0, 1.0, 0.5),
                ]
            },
            |_, v| Box::new(Tremolo::new(arg(v, 0, 5.0), arg(v, 1, 0.5))),
        ),
        NodeDescriptor::new(
            "chorus",
            "effect",
            || GraphNode::new("chorus", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Chorus::new(0.8, 4.0, 0.5)),
            || {
                vec![
                    Parameter::new("rate_hz", 0.05, 10.0, 0.8),
                    Parameter::new("depth_ms", 0.0, 10.0, 4.0),
                    Parameter::new("mix", 0.0, 1.0, 0.5),
                ]
            },
            |_, v| Box::new(Chorus::new(arg(v, 0, 0.8), arg(v, 1, 4.0), arg(v, 2, 0.5))),
        ),
        NodeDescriptor::new(
            "compressor",
            "effect",
            || GraphNode::new("compressor", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Compressor::new(-18.0, 4.0, 10.0, 100.0, 0.0)),
            || {
                vec![
                    Parameter::new("threshold_db", -60.0, 0.0, -18.0),
                    Parameter::new("ratio", 1.0, 20.0, 4.0),
                    Parameter::new("attack_ms", 0.1, 100.0, 10.0),
                    Parameter::new("release_ms", 1.0, 1000.0, 100.0),
                    Parameter::new("makeup_db", 0.0, 24.0, 0.0),
                ]
            },
            |_, v| {
                Box::new(Compressor::new(
                    arg(v, 0, -18.0),
                    arg(v, 1, 4.0),
                    arg(v, 2, 10.0),
                    arg(v, 3, 100.0),
                    arg(v, 4, 0.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "phaser",
            "effect",
            || GraphNode::new("phaser", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Phaser::new(0.5, 0.7, 0.3, 0.5)),
            || {
                vec![
                    Parameter::new("rate_hz", 0.05, 10.0, 0.5),
                    Parameter::new("depth", 0.0, 1.0, 0.7),
                    Parameter::new("feedback", 0.0, 0.95, 0.3),
                    Parameter::new("mix", 0.0, 1.0, 0.5),
                ]
            },
            |_, v| {
                Box::new(Phaser::new(
                    arg(v, 0, 0.5),
                    arg(v, 1, 0.7),
                    arg(v, 2, 0.3),
                    arg(v, 3, 0.5),
                ))
            },
        ),
        NodeDescriptor::new(
            "splitter",
            "routing",
            || {
                GraphNode::new(
                    "splitter",
                    vec![audio_in("in")],
                    vec![audio_out("a"), audio_out("b")],
                )
            },
            || Box::new(Splitter),
            Vec::new,
            |_, _| Box::new(Splitter),
        ),
        NodeDescriptor::new(
            "pan",
            "routing",
            || {
                GraphNode::new(
                    "pan",
                    vec![audio_in("in"), Port::input("pan", SignalType::Control)],
                    vec![audio_out("left"), audio_out("right")],
                )
            },
            || Box::new(Pan::new(0.0)),
            || vec![Parameter::new("pan", -1.0, 1.0, 0.0)],
            |_, v| Box::new(Pan::new(arg(v, 0, 0.0))),
        ),
        NodeDescriptor::new(
            "merger",
            "routing",
            || {
                GraphNode::new(
                    "merger",
                    vec![audio_in("a"), audio_in("b"), audio_in("c"), audio_in("d")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Merger),
            Vec::new,
            |_, _| Box::new(Merger),
        ),
        NodeDescriptor::new(
            "selector",
            "routing",
            || {
                GraphNode::new(
                    "selector",
                    vec![audio_in("a"), audio_in("b"), audio_in("c"), audio_in("d")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Selector::new(0)),
            || vec![Parameter::new("input", 0.0, 3.0, 0.0)],
            |_, v| Box::new(Selector::new(arg(v, 0, 0.0).round().max(0.0) as usize)),
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
            "voice-in",
            "input",
            || {
                GraphNode::new(
                    "voice-in",
                    vec![],
                    vec![
                        Port::output("gate", SignalType::Control),
                        Port::output("pitch_hz", SignalType::Control),
                        Port::output("velocity", SignalType::Control),
                    ],
                )
            },
            || Box::new(PolyVoiceIn::new(crate::poly::PolyNoteSource::new(), 0)),
            || vec![Parameter::new("voice", 0.0, (MAX_VOICES - 1) as f32, 0.0)],
            |ctx, v| {
                let voice = (arg(v, 0, 0.0).round().max(0.0) as usize).min(MAX_VOICES - 1);
                Box::new(PolyVoiceIn::new(ctx.poly.clone(), voice))
            },
        ),
        NodeDescriptor::new(
            "key-gate",
            "input",
            || {
                GraphNode::new(
                    "key-gate",
                    vec![],
                    vec![Port::output("gate", SignalType::Control)],
                )
            },
            || Box::new(KeyGate::new(crate::poly::PolyNoteSource::new(), 36)),
            || vec![Parameter::new("note", 0.0, 127.0, 36.0)],
            |ctx, v| {
                let note = (arg(v, 0, 36.0).round().clamp(0.0, 127.0)) as u8;
                Box::new(KeyGate::new(ctx.poly.clone(), note))
            },
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
        NodeDescriptor::new(
            "audio-in",
            "input",
            || GraphNode::new("audio-in", vec![], vec![audio_out("out")]),
            // No host bus yet: a silent default source.
            || Box::new(AudioInput::new(crate::io::AudioBus::default(), 0)),
            || {
                vec![Parameter::new(
                    "channel",
                    0.0,
                    (MAX_IO_CHANNELS - 1) as f32,
                    0.0,
                )]
            },
            |ctx, v| {
                let channel = io_channel(arg(v, 0, 0.0));
                Box::new(AudioInput::new(ctx.io.clone(), channel))
            },
        ),
        NodeDescriptor::new(
            "audio-out",
            "output",
            || GraphNode::new("audio-out", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(AudioOutput::new(crate::io::AudioBus::default(), 0)),
            || {
                vec![Parameter::new(
                    "channel",
                    0.0,
                    (MAX_IO_CHANNELS - 1) as f32,
                    0.0,
                )]
            },
            |ctx, v| {
                let channel = io_channel(arg(v, 0, 0.0));
                Box::new(AudioOutput::new(ctx.io.clone(), channel))
            },
        ),
        NodeDescriptor::new(
            "vca",
            "dynamics",
            || {
                GraphNode::new(
                    "vca",
                    vec![audio_in("in"), Port::input("cv", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Vca::default()),
            || {
                vec![
                    Parameter::new("bias", 0.0, 2.0, 1.0),
                    Parameter::new("amount", 0.0, 2.0, 1.0),
                ]
            },
            |_, v| Box::new(Vca::new(arg(v, 0, 1.0), arg(v, 1, 1.0))),
        ),
        NodeDescriptor::new(
            "gate",
            "dynamics",
            || GraphNode::new("gate", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Gate::new(-40.0, 1.0, 50.0)),
            || {
                vec![
                    Parameter::new("threshold_db", -80.0, 0.0, -40.0),
                    Parameter::new("attack_ms", 0.1, 100.0, 1.0),
                    Parameter::new("release_ms", 1.0, 1000.0, 50.0),
                ]
            },
            |_, v| Box::new(Gate::new(arg(v, 0, -40.0), arg(v, 1, 1.0), arg(v, 2, 50.0))),
        ),
        NodeDescriptor::new(
            "limiter",
            "dynamics",
            || GraphNode::new("limiter", vec![audio_in("in")], vec![audio_out("out")]),
            || Box::new(Limiter::new(0.0, 50.0)),
            || {
                vec![
                    Parameter::new("ceiling_db", -24.0, 0.0, 0.0),
                    Parameter::new("release_ms", 1.0, 1000.0, 50.0),
                ]
            },
            |_, v| Box::new(Limiter::new(arg(v, 0, 0.0), arg(v, 1, 50.0))),
        ),
        NodeDescriptor::new(
            "vcf",
            "filter",
            || {
                GraphNode::new(
                    "vcf",
                    vec![audio_in("in"), Port::input("cutoff", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Vcf::new(FilterKind::LowPass, 1_000.0, 0.707, 4_000.0)),
            || {
                vec![
                    Parameter::new("kind", 0.0, 3.0, 0.0),
                    Parameter::new("base_hz", 20.0, 20_000.0, 1_000.0),
                    Parameter::new("q", 0.1, 10.0, 0.707),
                    Parameter::new("amount_hz", -20_000.0, 20_000.0, 4_000.0),
                ]
            },
            |_, v| {
                Box::new(Vcf::new(
                    filter_kind_from(arg(v, 0, 0.0)),
                    arg(v, 1, 1_000.0),
                    arg(v, 2, 0.707),
                    arg(v, 3, 4_000.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "ladder",
            "filter",
            || {
                GraphNode::new(
                    "ladder",
                    vec![audio_in("in"), Port::input("cutoff", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Ladder::new(1_000.0, 0.3, 4_000.0)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 1_000.0),
                    Parameter::new("resonance", 0.0, 1.0, 0.3),
                    Parameter::new("amount_hz", -20_000.0, 20_000.0, 4_000.0),
                ]
            },
            |_, v| {
                Box::new(Ladder::new(
                    arg(v, 0, 1_000.0),
                    arg(v, 1, 0.3),
                    arg(v, 2, 4_000.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "vco",
            "generator",
            || {
                GraphNode::new(
                    "vco",
                    vec![Port::input("pitch", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Vco::new(Waveform::Sine, 440.0, 1.0)),
            || {
                vec![
                    Parameter::new("waveform", 0.0, 3.0, 0.0),
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("amount_oct", -4.0, 4.0, 1.0),
                ]
            },
            |_, v| {
                Box::new(Vco::new(
                    waveform_from(arg(v, 0, 0.0)),
                    arg(v, 1, 440.0),
                    arg(v, 2, 1.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "glide",
            "modulation",
            || {
                GraphNode::new(
                    "glide",
                    vec![Port::input("in", SignalType::Control)],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(Glide::new(50.0)),
            || vec![Parameter::new("glide_ms", 0.0, 5_000.0, 50.0)],
            |_, v| Box::new(Glide::new(arg(v, 0, 50.0))),
        ),
        NodeDescriptor::new(
            "sample-hold",
            "modulation",
            || {
                GraphNode::new(
                    "sample-hold",
                    vec![
                        Port::input("in", SignalType::Control),
                        Port::input("trig", SignalType::Control),
                    ],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(SampleHold::new()),
            Vec::new,
            |_, _| Box::new(SampleHold::new()),
        ),
        NodeDescriptor::new(
            "slew",
            "modulation",
            || {
                GraphNode::new(
                    "slew",
                    vec![Port::input("in", SignalType::Control)],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(Slew::new(0.05, 0.05)),
            || {
                vec![
                    Parameter::new("rise_s", 0.0, 5.0, 0.05),
                    Parameter::new("fall_s", 0.0, 5.0, 0.05),
                ]
            },
            |_, v| Box::new(Slew::new(arg(v, 0, 0.05), arg(v, 1, 0.05))),
        ),
        NodeDescriptor::new(
            "quantizer",
            "modulation",
            || {
                GraphNode::new(
                    "quantizer",
                    vec![Port::input("in", SignalType::Control)],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(Quantizer::new(440.0, 12.0)),
            || {
                vec![
                    Parameter::new("ref_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("divisions", 1.0, 48.0, 12.0),
                ]
            },
            |_, v| Box::new(Quantizer::new(arg(v, 0, 440.0), arg(v, 1, 12.0))),
        ),
        NodeDescriptor::new(
            "comparator",
            "modulation",
            || {
                GraphNode::new(
                    "comparator",
                    vec![
                        Port::input("a", SignalType::Control),
                        Port::input("b", SignalType::Control),
                    ],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(Comparator::new(0.0)),
            || vec![Parameter::new("hysteresis", 0.0, 1.0, 0.0)],
            |_, v| Box::new(Comparator::new(arg(v, 0, 0.0))),
        ),
        NodeDescriptor::new(
            "clock",
            "sequencing",
            || {
                GraphNode::new(
                    "clock",
                    vec![],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(Clock::new(120.0, 0.5)),
            || {
                vec![
                    Parameter::new("bpm", 20.0, 300.0, 120.0),
                    Parameter::new("pulse_width", 0.01, 0.99, 0.5),
                ]
            },
            |_, v| Box::new(Clock::new(arg(v, 0, 120.0), arg(v, 1, 0.5))),
        ),
        NodeDescriptor::new(
            "clock-divider",
            "sequencing",
            || {
                GraphNode::new(
                    "clock-divider",
                    vec![Port::input("in", SignalType::Control)],
                    vec![Port::output("out", SignalType::Control)],
                )
            },
            || Box::new(ClockDivider::new(2)),
            || vec![Parameter::new("division", 1.0, 16.0, 2.0)],
            |_, v| Box::new(ClockDivider::new(arg(v, 0, 2.0).round().max(1.0) as u32)),
        ),
        NodeDescriptor::new(
            "step-sequencer",
            "sequencing",
            || {
                GraphNode::new(
                    "step-sequencer",
                    vec![Port::input("clock", SignalType::Control)],
                    vec![Port::output("cv", SignalType::Control)],
                )
            },
            || Box::new(StepSequencer::new(8, [0.0; crate::step_sequencer::STEPS])),
            || {
                let mut params = vec![Parameter::new("steps", 1.0, 8.0, 8.0)];
                for i in 0..crate::step_sequencer::STEPS {
                    params.push(Parameter::new(format!("step{i}"), -1.0, 1.0, 0.0));
                }
                params
            },
            |_, v| {
                let mut values = [0.0_f32; crate::step_sequencer::STEPS];
                for (i, slot) in values.iter_mut().enumerate() {
                    *slot = arg(v, i + 1, 0.0);
                }
                Box::new(StepSequencer::new(
                    arg(v, 0, 8.0).round().max(1.0) as u32,
                    values,
                ))
            },
        ),
        NodeDescriptor::new(
            "euclid",
            "sequencing",
            || {
                GraphNode::new(
                    "euclid",
                    vec![Port::input("clock", SignalType::Control)],
                    vec![Port::output("gate", SignalType::Control)],
                )
            },
            || Box::new(Euclid::new(4, 8)),
            || {
                vec![
                    Parameter::new("pulses", 0.0, 32.0, 4.0),
                    Parameter::new("steps", 1.0, 32.0, 8.0),
                ]
            },
            |_, v| {
                Box::new(Euclid::new(
                    arg(v, 0, 4.0).round().max(0.0) as u32,
                    arg(v, 1, 8.0).round().max(1.0) as u32,
                ))
            },
        ),
        NodeDescriptor::new(
            "pitch-osc",
            "generator",
            || {
                GraphNode::new(
                    "pitch-osc",
                    vec![Port::input("pitch", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(PitchOsc::new(Waveform::Saw, 440.0, 0.0)),
            || {
                vec![
                    Parameter::new("waveform", 0.0, 3.0, 1.0),
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("detune_semitones", -24.0, 24.0, 0.0),
                ]
            },
            |_, v| {
                Box::new(PitchOsc::new(
                    waveform_from(arg(v, 0, 1.0)),
                    arg(v, 1, 440.0),
                    arg(v, 2, 0.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "wavetable",
            "generator",
            || {
                GraphNode::new(
                    "wavetable",
                    vec![
                        Port::input("pitch", SignalType::Control),
                        Port::input("position", SignalType::Control),
                    ],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Wavetable::new(440.0, 0.5)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("position", 0.0, 1.0, 0.5),
                ]
            },
            |_, v| Box::new(Wavetable::new(arg(v, 0, 440.0), arg(v, 1, 0.5))),
        ),
        NodeDescriptor::new(
            "karplus",
            "generator",
            || {
                GraphNode::new(
                    "karplus",
                    vec![
                        Port::input("gate", SignalType::Control),
                        Port::input("pitch", SignalType::Control),
                    ],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Karplus::new(220.0, 0.996, 0.5)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 220.0),
                    Parameter::new("damping", 0.0, 0.9999, 0.996),
                    Parameter::new("tone", 0.0, 1.0, 0.5),
                ]
            },
            |_, v| {
                Box::new(Karplus::new(
                    arg(v, 0, 220.0),
                    arg(v, 1, 0.996),
                    arg(v, 2, 0.5),
                ))
            },
        ),
        NodeDescriptor::new(
            "fm-op",
            "generator",
            || {
                GraphNode::new(
                    "fm-op",
                    vec![Port::input("pitch", SignalType::Control), audio_in("mod")],
                    vec![audio_out("out")],
                )
            },
            || Box::new(FmOp::new(440.0, 1.0, 0.0)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("ratio", 0.0, 16.0, 1.0),
                    Parameter::new("index", 0.0, 24.0, 0.0),
                ]
            },
            |_, v| Box::new(FmOp::new(arg(v, 0, 440.0), arg(v, 1, 1.0), arg(v, 2, 0.0))),
        ),
        NodeDescriptor::new(
            "supersaw",
            "generator",
            || {
                GraphNode::new(
                    "supersaw",
                    vec![Port::input("pitch", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Supersaw::new(440.0, 0.3, 7)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("detune_semitones", 0.0, 2.0, 0.3),
                    Parameter::new("voices", 1.0, 9.0, 7.0),
                ]
            },
            |_, v| {
                Box::new(Supersaw::new(
                    arg(v, 0, 440.0),
                    arg(v, 1, 0.3),
                    arg(v, 2, 7.0).round().max(1.0) as usize,
                ))
            },
        ),
        NodeDescriptor::new(
            "additive",
            "generator",
            || {
                GraphNode::new(
                    "additive",
                    vec![Port::input("pitch", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Additive::new(440.0, 8, 1.0)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("partials", 1.0, 32.0, 8.0),
                    Parameter::new("tilt", 0.0, 2.0, 1.0),
                ]
            },
            |_, v| {
                Box::new(Additive::new(
                    arg(v, 0, 440.0),
                    arg(v, 1, 8.0).round().max(1.0) as u32,
                    arg(v, 2, 1.0),
                ))
            },
        ),
        NodeDescriptor::new(
            "sub-osc",
            "generator",
            || {
                GraphNode::new(
                    "sub-osc",
                    vec![Port::input("pitch", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(SubOsc::new(440.0, 1)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("octaves", 1.0, 2.0, 1.0),
                ]
            },
            |_, v| {
                Box::new(SubOsc::new(
                    arg(v, 0, 440.0),
                    arg(v, 1, 1.0).round().clamp(1.0, 2.0) as u32,
                ))
            },
        ),
        NodeDescriptor::new(
            "pwm",
            "generator",
            || {
                GraphNode::new(
                    "pwm",
                    vec![Port::input("pitch", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(Pwm::new(440.0, 0.5)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("width", 0.01, 0.99, 0.5),
                ]
            },
            |_, v| Box::new(Pwm::new(arg(v, 0, 440.0), arg(v, 1, 0.5))),
        ),
        NodeDescriptor::new(
            "hard-sync",
            "generator",
            || {
                GraphNode::new(
                    "hard-sync",
                    vec![Port::input("pitch", SignalType::Control)],
                    vec![audio_out("out")],
                )
            },
            || Box::new(HardSync::new(440.0, 2.0)),
            || {
                vec![
                    Parameter::new("base_hz", 20.0, 20_000.0, 440.0),
                    Parameter::new("sync_ratio", 1.0, 8.0, 2.0),
                ]
            },
            |_, v| Box::new(HardSync::new(arg(v, 0, 440.0), arg(v, 1, 2.0))),
        ),
    ]
}

/// Clamps a raw parameter value to a valid I/O channel index.
fn io_channel(raw: f32) -> usize {
    (raw.round().max(0.0) as usize).min(MAX_IO_CHANNELS - 1)
}

#[cfg(test)]
mod tests {
    use super::Registry;
    use crate::midi::BuildContext;
    use std::collections::HashMap;
    use vb_core::{Engine, Graph, PortId, ProcessContext, Processor};

    #[test]
    fn cv_math_nodes_are_control_typed_and_sum_control_signals() {
        let r = Registry::with_builtins();
        for kind in ["cv-add", "cv-attenuvert"] {
            let n = r.graph_node(kind).expect("registered");
            assert!(
                n.inputs
                    .iter()
                    .all(|p| p.signal == vb_core::SignalType::Control),
                "{kind} inputs are Control"
            );
            assert!(
                n.outputs
                    .iter()
                    .all(|p| p.signal == vb_core::SignalType::Control),
                "{kind} outputs are Control"
            );
            assert!(r.processor(kind).is_some(), "{kind} builds a processor");
        }
        // A control source → cv-add → a control sink connects — the audio `add` would be a
        // signal mismatch here (this is why the CV twins exist).
        let mut g = Graph::new();
        let lfo = g.add_node(r.graph_node("lfo").unwrap());
        let cv_add = g.add_node(r.graph_node("cv-add").unwrap());
        let ladder = g.add_node(r.graph_node("ladder").unwrap());
        assert!(
            g.connect(lfo, PortId(0), cv_add, PortId(0)).is_ok(),
            "lfo → cv-add.a"
        );
        assert!(
            g.connect(cv_add, PortId(0), ladder, PortId(1)).is_ok(),
            "cv-add → ladder.cutoff"
        );
    }

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
    fn vco_is_registered_as_a_pitch_modulated_generator() {
        let r = Registry::with_builtins();
        let desc = r.get("vco").expect("vco is registered");
        assert_eq!(desc.category(), "generator");

        // One control input (the pitch CV) and one audio output.
        let node = r.graph_node("vco").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs.len(), 1);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);

        // Built with waveform / base_hz / amount_oct params.
        let descriptor_params = desc.params();
        let params: Vec<&str> = descriptor_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(params, vec!["waveform", "base_hz", "amount_oct"]);
        assert!(r.processor("vco").is_some(), "a processor can be built");
    }

    #[test]
    fn math_utility_nodes_are_registered() {
        let r = Registry::with_builtins();
        // Binary difference.
        let sub = r.graph_node("sub").expect("sub registered");
        assert_eq!(sub.inputs.len(), 2);
        assert_eq!(r.get("sub").unwrap().category(), "math");
        assert!(r.get("sub").unwrap().params().is_empty());
        // Unary rectify.
        let rect = r.graph_node("rectify").expect("rectify registered");
        assert_eq!(rect.inputs.len(), 1);
        assert!(r.get("rectify").unwrap().params().is_empty());
        // Attenuverter with scale/offset.
        assert_eq!(r.get("attenuvert").unwrap().category(), "math");
        let att = r.get("attenuvert").unwrap().params();
        assert_eq!(
            att.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["scale", "offset"]
        );
        assert!(r.processor("attenuvert").is_some());
        // Binary min/max (signal utility and CV logic AND/OR).
        for kind in ["min", "max"] {
            let node = r
                .graph_node(kind)
                .unwrap_or_else(|| panic!("{kind} registered"));
            assert_eq!(node.inputs.len(), 2);
            assert_eq!(r.get(kind).unwrap().category(), "math");
            assert!(r.get(kind).unwrap().params().is_empty());
            assert!(r.processor(kind).is_some());
        }
    }

    #[test]
    fn tremolo_and_chorus_are_registered_as_effects() {
        let r = Registry::with_builtins();
        for kind in ["tremolo", "chorus"] {
            let d = r.get(kind).unwrap_or_else(|| panic!("{kind} registered"));
            assert_eq!(d.category(), "effect");
            let node = r.graph_node(kind).unwrap();
            assert_eq!(node.inputs.len(), 1);
            assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
            assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
            assert!(r.processor(kind).is_some());
        }
        let tr = r.get("tremolo").unwrap().params();
        assert_eq!(
            tr.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["rate_hz", "depth"]
        );
        let ch = r.get("chorus").unwrap().params();
        assert_eq!(
            ch.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["rate_hz", "depth_ms", "mix"]
        );
    }

    #[test]
    fn compressor_and_phaser_are_registered_as_effects() {
        let r = Registry::with_builtins();
        for kind in ["compressor", "phaser"] {
            let d = r.get(kind).unwrap_or_else(|| panic!("{kind} registered"));
            assert_eq!(d.category(), "effect");
            let node = r.graph_node(kind).unwrap();
            assert_eq!(node.inputs.len(), 1);
            assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
            assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
            assert!(r.processor(kind).is_some());
        }
        let comp = r.get("compressor").unwrap().params();
        assert_eq!(
            comp.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec![
                "threshold_db",
                "ratio",
                "attack_ms",
                "release_ms",
                "makeup_db"
            ]
        );
        let ph = r.get("phaser").unwrap().params();
        assert_eq!(
            ph.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["rate_hz", "depth", "feedback", "mix"]
        );
    }

    #[test]
    fn ring_mod_and_bitcrusher_are_registered_as_effects() {
        let r = Registry::with_builtins();
        for kind in ["ring-mod", "bitcrusher"] {
            let d = r.get(kind).unwrap_or_else(|| panic!("{kind} registered"));
            assert_eq!(d.category(), "effect");
            let node = r.graph_node(kind).unwrap();
            assert_eq!(node.inputs.len(), 1);
            assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
            assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
            assert!(r.processor(kind).is_some());
        }
        let rm = r.get("ring-mod").unwrap().params();
        assert_eq!(
            rm.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["freq_hz", "mix"]
        );
        let bc = r.get("bitcrusher").unwrap().params();
        assert_eq!(
            bc.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["bits", "downsample", "mix"]
        );
    }

    #[test]
    fn sample_hold_and_slew_are_registered_as_modulation() {
        let r = Registry::with_builtins();

        // Sample & hold: signal + trigger control inputs → control output; no params.
        let sh = r.get("sample-hold").expect("sample-hold is registered");
        assert_eq!(sh.category(), "modulation");
        let node = r.graph_node("sample-hold").unwrap();
        assert_eq!(node.inputs.len(), 2);
        assert!(node
            .inputs
            .iter()
            .all(|p| p.signal == vb_core::SignalType::Control));
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert!(sh.params().is_empty());
        assert!(r.processor("sample-hold").is_some());

        // Slew: one control input → control output; rise/fall params.
        let slew = r.get("slew").expect("slew is registered");
        assert_eq!(slew.category(), "modulation");
        let node = r.graph_node("slew").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        let slew_desc_params = slew.params();
        let slew_params: Vec<&str> = slew_desc_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(slew_params, vec!["rise_s", "fall_s"]);
        assert!(r.processor("slew").is_some());
    }

    #[test]
    fn quantizer_and_comparator_are_registered_as_modulation() {
        let r = Registry::with_builtins();

        // Quantizer: one control input → control output; ref/divisions params.
        let q = r.get("quantizer").expect("quantizer is registered");
        assert_eq!(q.category(), "modulation");
        let node = r.graph_node("quantizer").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(
            q.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["ref_hz", "divisions"]
        );
        assert!(r.processor("quantizer").is_some());

        // Comparator: two control inputs → control gate; hysteresis param.
        let c = r.get("comparator").expect("comparator is registered");
        assert_eq!(c.category(), "modulation");
        let node = r.graph_node("comparator").unwrap();
        assert_eq!(node.inputs.len(), 2);
        assert!(node
            .inputs
            .iter()
            .all(|p| p.signal == vb_core::SignalType::Control));
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(
            c.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hysteresis"]
        );
        assert!(r.processor("comparator").is_some());
    }

    #[test]
    fn clock_and_clock_divider_are_registered_as_sequencing() {
        let r = Registry::with_builtins();

        // Clock: no inputs → control gate output; bpm/pulse_width params.
        let c = r.get("clock").expect("clock is registered");
        assert_eq!(c.category(), "sequencing");
        let node = r.graph_node("clock").unwrap();
        assert!(node.inputs.is_empty());
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(
            c.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["bpm", "pulse_width"]
        );
        assert!(r.processor("clock").is_some());

        // Clock divider: one control gate in → control gate out; division param.
        let d = r.get("clock-divider").expect("clock-divider is registered");
        assert_eq!(d.category(), "sequencing");
        let node = r.graph_node("clock-divider").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(
            d.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["division"]
        );
        assert!(r.processor("clock-divider").is_some());
    }

    #[test]
    fn step_sequencer_and_euclid_are_registered_as_sequencing() {
        let r = Registry::with_builtins();

        // Step sequencer: one control clock in → control cv out; steps + 8 step values.
        let s = r
            .get("step-sequencer")
            .expect("step-sequencer is registered");
        assert_eq!(s.category(), "sequencing");
        let node = r.graph_node("step-sequencer").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        let params = s.params();
        let names: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names[0], "steps");
        assert_eq!(names.len(), 1 + crate::step_sequencer::STEPS);
        assert_eq!(names[1], "step0");
        assert!(r.processor("step-sequencer").is_some());

        // Euclid: one control clock in → control gate out; pulses + steps params.
        let e = r.get("euclid").expect("euclid is registered");
        assert_eq!(e.category(), "sequencing");
        let node = r.graph_node("euclid").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(
            e.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["pulses", "steps"]
        );
        assert!(r.processor("euclid").is_some());
    }

    #[test]
    fn dc_blocker_and_tone_are_registered_as_filters() {
        let r = Registry::with_builtins();

        // DC blocker: audio in → audio out; single pole-radius param.
        let d = r.get("dc-blocker").expect("dc-blocker is registered");
        assert_eq!(d.category(), "filter");
        let node = r.graph_node("dc-blocker").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(
            d.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["r"]
        );
        assert!(r.processor("dc-blocker").is_some());

        // Tone: audio in → audio out; tilt + pivot params.
        let t = r.get("tone").expect("tone is registered");
        assert_eq!(t.category(), "filter");
        let node = r.graph_node("tone").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(
            t.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["tilt", "pivot_hz"]
        );
        assert!(r.processor("tone").is_some());
    }

    #[test]
    fn gate_and_limiter_are_registered_as_dynamics() {
        let r = Registry::with_builtins();

        // Gate: audio in → audio out; threshold + attack/release params.
        let g = r.get("gate").expect("gate is registered");
        assert_eq!(g.category(), "dynamics");
        let node = r.graph_node("gate").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(
            g.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["threshold_db", "attack_ms", "release_ms"]
        );
        assert!(r.processor("gate").is_some());

        // Limiter: audio in → audio out; ceiling + release params.
        let l = r.get("limiter").expect("limiter is registered");
        assert_eq!(l.category(), "dynamics");
        let node = r.graph_node("limiter").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(
            l.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["ceiling_db", "release_ms"]
        );
        assert!(r.processor("limiter").is_some());
    }

    #[test]
    fn wavefolder_and_xfade_are_registered() {
        let r = Registry::with_builtins();

        // Wavefolder: audio in → audio out; drive/symmetry/level params; effect category.
        let wf = r.get("wavefolder").expect("wavefolder is registered");
        assert_eq!(wf.category(), "effect");
        let node = r.graph_node("wavefolder").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(
            wf.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["drive", "symmetry", "level"]
        );
        assert!(r.processor("wavefolder").is_some());

        // Xfade: two audio inputs + a control mix input → audio out; math category.
        let xf = r.get("xfade").expect("xfade is registered");
        assert_eq!(xf.category(), "math");
        let node = r.graph_node("xfade").unwrap();
        assert_eq!(node.inputs.len(), 3);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.inputs[1].signal, vb_core::SignalType::Audio);
        assert_eq!(node.inputs[2].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert!(xf.params().is_empty(), "xfade has no params");
        assert!(r.processor("xfade").is_some());
    }

    #[test]
    fn pan_is_registered_with_a_stereo_output() {
        let r = Registry::with_builtins();
        let p = r.get("pan").expect("pan is registered");
        assert_eq!(p.category(), "routing");
        let node = r.graph_node("pan").unwrap();
        // Audio in + control pan in.
        assert_eq!(node.inputs.len(), 2);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.inputs[1].signal, vb_core::SignalType::Control);
        // Two audio outputs — the defining stereo (two-channel) node.
        assert_eq!(node.outputs.len(), 2);
        assert_eq!(node.outputs[0].name, "left");
        assert_eq!(node.outputs[1].name, "right");
        assert!(node
            .outputs
            .iter()
            .all(|o| o.signal == vb_core::SignalType::Audio));
        assert_eq!(
            p.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["pan"]
        );
        assert!(r.processor("pan").is_some());
    }

    #[test]
    fn ping_pong_and_width_are_registered_as_stereo_effects() {
        let r = Registry::with_builtins();

        // Ping-pong: mono audio in → stereo left/right out; delay/feedback/mix params.
        let pp = r.get("ping-pong").expect("ping-pong is registered");
        assert_eq!(pp.category(), "effect");
        let node = r.graph_node("ping-pong").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.outputs.len(), 2);
        assert_eq!(node.outputs[0].name, "left");
        assert_eq!(node.outputs[1].name, "right");
        assert!(node
            .outputs
            .iter()
            .all(|o| o.signal == vb_core::SignalType::Audio));
        assert_eq!(
            pp.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["delay_s", "feedback", "mix"]
        );
        assert!(r.processor("ping-pong").is_some());

        // Width: stereo left/right in → stereo left/right out; single width param.
        let w = r.get("width").expect("width is registered");
        assert_eq!(w.category(), "effect");
        let node = r.graph_node("width").unwrap();
        assert_eq!(node.inputs.len(), 2);
        assert_eq!(node.outputs.len(), 2);
        assert!(node
            .inputs
            .iter()
            .chain(node.outputs.iter())
            .all(|p| p.signal == vb_core::SignalType::Audio));
        assert_eq!(
            w.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["width"]
        );
        assert!(r.processor("width").is_some());
    }

    #[test]
    fn ad_and_env_follower_are_registered() {
        let r = Registry::with_builtins();

        // AD: gate control input → level control output; envelope category.
        let ad = r.get("ad").expect("ad is registered");
        assert_eq!(ad.category(), "envelope");
        let node = r.graph_node("ad").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        let ad_desc_params = ad.params();
        let ad_params: Vec<&str> = ad_desc_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(ad_params, vec!["attack_s", "decay_s"]);
        assert!(r.processor("ad").is_some());

        // Envelope follower: audio input → control output; modulation category.
        let ef = r.get("env-follower").expect("env-follower is registered");
        assert_eq!(ef.category(), "modulation");
        let node = r.graph_node("env-follower").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        let ef_desc_params = ef.params();
        let ef_params: Vec<&str> = ef_desc_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(ef_params, vec!["attack_s", "release_s"]);
        assert!(r.processor("env-follower").is_some());
    }

    #[test]
    fn ar_and_ahr_are_registered_as_envelopes() {
        let r = Registry::with_builtins();

        // AR: gate control in → level control out; attack/release params.
        let ar = r.get("ar").expect("ar is registered");
        assert_eq!(ar.category(), "envelope");
        let node = r.graph_node("ar").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(
            ar.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["attack_s", "release_s"]
        );
        assert!(r.processor("ar").is_some());

        // AHR: gate control in → level control out; attack/hold/release params.
        let ahr = r.get("ahr").expect("ahr is registered");
        assert_eq!(ahr.category(), "envelope");
        let node = r.graph_node("ahr").unwrap();
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(
            ahr.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["attack_s", "hold_s", "release_s"]
        );
        assert!(r.processor("ahr").is_some());
    }

    #[test]
    fn pwm_and_hard_sync_are_registered_as_generators() {
        let r = Registry::with_builtins();
        for kind in ["pwm", "hard-sync"] {
            let d = r.get(kind).unwrap_or_else(|| panic!("{kind} registered"));
            assert_eq!(d.category(), "generator");
            let node = r.graph_node(kind).unwrap();
            assert_eq!(node.inputs.len(), 1);
            assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
            assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
            assert!(r.processor(kind).is_some());
        }
        let pwm = r.get("pwm").unwrap().params();
        assert_eq!(
            pwm.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["base_hz", "width"]
        );
        let hs = r.get("hard-sync").unwrap().params();
        assert_eq!(
            hs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["base_hz", "sync_ratio"]
        );
    }

    #[test]
    fn fm_op_and_supersaw_are_registered_as_generators() {
        let r = Registry::with_builtins();

        // The FM operator: pitch CV + an audio modulator input, one audio output.
        let fm = r.get("fm-op").expect("fm-op is registered");
        assert_eq!(fm.category(), "generator");
        let node = r.graph_node("fm-op").unwrap();
        assert_eq!(node.inputs.len(), 2);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control); // pitch
        assert_eq!(node.inputs[1].signal, vb_core::SignalType::Audio); // mod
        assert_eq!(node.outputs.len(), 1);
        let fm_desc_params = fm.params();
        let fm_params: Vec<&str> = fm_desc_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(fm_params, vec!["base_hz", "ratio", "index"]);
        assert!(r.processor("fm-op").is_some());

        // The supersaw: a single pitch CV input, one audio output.
        let ss = r.get("supersaw").expect("supersaw is registered");
        assert_eq!(ss.category(), "generator");
        let node = r.graph_node("supersaw").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        let ss_desc_params = ss.params();
        let ss_params: Vec<&str> = ss_desc_params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(ss_params, vec!["base_hz", "detune_semitones", "voices"]);
        assert!(r.processor("supersaw").is_some());
    }

    #[test]
    fn additive_and_sub_osc_are_registered_as_generators() {
        let r = Registry::with_builtins();

        // Additive: one pitch CV input → one audio output; base/partials/tilt params.
        let a = r.get("additive").expect("additive is registered");
        assert_eq!(a.category(), "generator");
        let node = r.graph_node("additive").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(
            a.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["base_hz", "partials", "tilt"]
        );
        assert!(r.processor("additive").is_some());

        // Sub oscillator: one pitch CV input → one audio output; base/octaves params.
        let s = r.get("sub-osc").expect("sub-osc is registered");
        assert_eq!(s.category(), "generator");
        let node = r.graph_node("sub-osc").unwrap();
        assert_eq!(node.inputs.len(), 1);
        assert_eq!(node.inputs[0].signal, vb_core::SignalType::Control);
        assert_eq!(node.outputs[0].signal, vb_core::SignalType::Audio);
        assert_eq!(
            s.params()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["base_hz", "octaves"]
        );
        assert!(r.processor("sub-osc").is_some());
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
    fn audio_io_nodes_are_registered_with_the_right_shape() {
        let r = Registry::with_builtins();
        let kinds: Vec<&str> = r.kinds().collect();
        assert!(kinds.contains(&"audio-in") && kinds.contains(&"audio-out"));

        let ain = r.graph_node("audio-in").unwrap();
        assert_eq!((ain.inputs.len(), ain.outputs.len()), (0, 1), "source");
        let aout = r.graph_node("audio-out").unwrap();
        assert_eq!(
            (aout.inputs.len(), aout.outputs.len()),
            (1, 1),
            "sink+passthru"
        );

        assert_eq!(r.get("audio-in").unwrap().category(), "input");
        assert_eq!(r.get("audio-out").unwrap().category(), "output");
        assert_eq!(r.get("audio-in").unwrap().params()[0].name, "channel");
    }

    #[test]
    fn registry_audio_io_carries_host_audio_through_the_graph() {
        // audio-in(ch 0) -> audio-out(ch 1): the host feeds channel 0 and reads its
        // signal back on channel 1, exercising the shared bus end to end.
        let r = Registry::with_builtins();
        let bus = crate::io::AudioBus::new(64);
        let ctx = BuildContext {
            io: bus.clone(),
            ..Default::default()
        };

        let mut g = Graph::new();
        let ain = g.add_node(r.graph_node("audio-in").unwrap());
        let aout = g.add_node(r.graph_node("audio-out").unwrap());
        g.connect(ain, PortId(0), aout, PortId(0)).unwrap();

        let mut procs: HashMap<_, Box<dyn Processor>> = HashMap::new();
        procs.insert(ain, r.get("audio-in").unwrap().processor_with(&ctx, &[0.0]));
        procs.insert(
            aout,
            r.get("audio-out").unwrap().processor_with(&ctx, &[1.0]),
        );

        let block = [0.1_f32, 0.2, 0.3, 0.4];
        bus.publish_input(&[&block], 4);

        let mut engine = Engine::compile(&g, procs, 4).unwrap();
        engine.process(&ProcessContext {
            sample_rate: 48_000.0,
            frames: 4,
        });

        // Pass-through reaches the audio-out node's output buffer...
        let tapped = engine.node_output(aout).unwrap().channel(0).unwrap();
        assert_eq!(tapped, block);
        // ...and the host drains the same signal from bus output channel 1.
        let (mut l, mut r_ch) = ([0.0; 4], [0.0; 4]);
        bus.collect_output(&mut [&mut l, &mut r_ch]);
        assert_eq!(l, [0.0; 4]);
        assert_eq!(r_ch, block);
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
