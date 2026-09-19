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

    let mut engine =
        RtEngine::compile(&graph, procs, frames).map_err(|e| BuildError::Engine(e.to_string()))?;
    // Alias document ids so a host can live-update a node's parameters by the id it tracks
    // (its automatable parameters are keyed by document id) — see `RtEngine::set_doc_params`.
    engine.alias_doc_ids(&remap);
    Ok((engine, sink))
}

/// The patch the plugin ships with — **the Moog Minitaur mono-bass** (Epic 14 / 000164): a
/// recognisable, musical default so the instrument makes a real sound the moment it loads,
/// not a generic test tone. It is note-gated and silent until a note plays. See
/// [`minitaur_patch`] for the voice and `docs/synths/minitaur.md` for the design.
#[must_use]
pub fn default_patch(registry: &Registry) -> ProjectDoc {
    minitaur_patch(registry)
}

/// A faithful Moog Minitaur mono-bass voice (Epic 14 / 000163): **two VCOs** (VCO1 saw + VCO2
/// an octave-down square — no separate sub, the real Minitaur has exactly two oscillators)
/// through a **leveled** two-channel mixer (per-oscillator `attenuvert`, VCO1/VCO2 balance)
/// into a Moog **ladder** low-pass, with **separate filter and amp envelopes** and an **LFO**
/// summed into the cutoff (through the control-rate `cv-attenuvert`/`cv-add`; its depth is off
/// by default for a clean bass). Glide on the pitch; **silent until a note** (VCA biased shut).
/// See `docs/synths/minitaur.md`. This is the plugin's default patch (000164).
#[must_use]
pub fn minitaur_patch(registry: &Registry) -> ProjectDoc {
    use vb_core::PortId;
    let mut g = vb_core::Graph::new();
    // The closure borrows `g` mutably; the block scopes that borrow so `g.connect` is free
    // afterwards (no `drop` needed).
    let (
        midi,
        glide,
        osc1,
        osc2,
        lvl1,
        lvl2,
        mix,
        ladder,
        filt_eg,
        amp_eg,
        vca,
        lfo,
        cv_depth,
        cv_mix,
    ) = {
        let mut add = |kind: &str| g.add_node(registry.graph_node(kind).expect("builtin"));
        (
            add("midi-in"),
            add("glide"),
            add("pitch-osc"),
            add("pitch-osc"),
            add("attenuvert"),
            add("attenuvert"),
            add("add"),
            add("ladder"),
            add("adsr"),
            add("adsr"),
            add("vca"),
            add("lfo"),
            add("cv-attenuvert"),
            add("cv-add"),
        )
    };

    // Pitch: note → glide → each oscillator's pitch input.
    g.connect(midi, PortId(1), glide, PortId(0))
        .expect("midi.pitch → glide");
    for osc in [osc1, osc2] {
        g.connect(glide, PortId(0), osc, PortId(0))
            .expect("glide → osc.pitch");
    }
    // Two-channel VCO mixer: each VCO · its own level → summed (VCO1/VCO2 balance).
    g.connect(osc1, PortId(0), lvl1, PortId(0))
        .expect("osc1 → lvl1");
    g.connect(osc2, PortId(0), lvl2, PortId(0))
        .expect("osc2 → lvl2");
    g.connect(lvl1, PortId(0), mix, PortId(0))
        .expect("lvl1 → mix.a");
    g.connect(lvl2, PortId(0), mix, PortId(1))
        .expect("lvl2 → mix.b");
    // Audio path: mix → ladder low-pass → VCA.
    g.connect(mix, PortId(0), ladder, PortId(0))
        .expect("mix → ladder.in");
    g.connect(ladder, PortId(0), vca, PortId(0))
        .expect("ladder → vca.in");
    // Filter path: the ladder cutoff CV is (filter envelope + LFO·depth), summed through the
    // control-rate `cv-add` (the audio `add` can't carry CV). The filter EG is a SEPARATE
    // contour from the amp — the Minitaur has two envelopes; the ladder's `amount_oct` scales
    // the sweep in octaves. The LFO's depth is off by default (a clean bass); the routing lets
    // the user dial in a filter wobble.
    g.connect(midi, PortId(0), filt_eg, PortId(0))
        .expect("gate → filtEG");
    g.connect(filt_eg, PortId(0), cv_mix, PortId(0))
        .expect("filtEG → cv_mix.a");
    g.connect(lfo, PortId(0), cv_depth, PortId(0))
        .expect("lfo → cv_depth");
    g.connect(cv_depth, PortId(0), cv_mix, PortId(1))
        .expect("cv_depth → cv_mix.b");
    g.connect(cv_mix, PortId(0), ladder, PortId(1))
        .expect("cv_mix → ladder.cutoff");
    // Amp path: gate → amp envelope → VCA cv.
    g.connect(midi, PortId(0), amp_eg, PortId(0))
        .expect("gate → ampEG");
    g.connect(amp_eg, PortId(0), vca, PortId(1))
        .expect("ampEG → vca.cv");

    let mut doc = ProjectDoc::from_graph(&g);
    // Pinned voice (000134): a punchy mono bass, silent at rest. Two VCOs — VCO1 a saw at
    // unison, VCO2 an octave-down square for the classic fat Minitaur low end (VCO2 FREQ is
    // the user's to retune, ±1 octave / ±12 st on the real panel).
    doc.set_node_values(osc1, vec![1.0, 55.0, 0.0]); // VCO1: saw, unison
    doc.set_node_values(osc2, vec![2.0, 55.0, -12.0]); // VCO2: square, one octave down
    doc.set_node_values(lvl1, vec![0.9, 0.0]); // VCO1 level
    doc.set_node_values(lvl2, vec![0.8, 0.0]); // VCO2 level
    doc.set_node_values(ladder, vec![200.0, 0.6, 4.0]); // low, resonant, EG-swept 4 octaves
    doc.set_node_values(filt_eg, vec![0.005, 0.15, 0.3, 0.2]); // a "ping" that decays
    doc.set_node_values(amp_eg, vec![0.005, 0.10, 0.8, 0.2]); // sustained while held
    doc.set_node_values(lfo, vec![3.0, 0.0, 5.0, 120.0, 2.0]); // triangle ~5 Hz
    doc.set_node_values(cv_depth, vec![0.0, 0.0]); // LFO depth OFF by default (clean bass)
    doc.set_node_values(vca, vec![0.0, 1.0]); // bias 0 → silent at rest
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
    fn minitaur_patch_is_silent_at_rest() {
        let registry = Registry::with_builtins();
        let doc = minitaur_patch(&registry);
        let (mut engine, sink) =
            engine_from_doc(&doc, &registry, &BuildContext::default(), 256).expect("compiles");
        let sink = sink.expect("terminal vca is the sink");
        let mut peak = 0.0_f32;
        for _ in 0..8 {
            let out = process_once(&mut engine, sink);
            assert!(out.iter().all(|s| s.is_finite()));
            peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
        assert!(
            peak < 1e-6,
            "minitaur must be silent until a note plays: {peak}"
        );
    }

    #[test]
    fn minitaur_patch_opens_on_a_note_as_a_bass() {
        let registry = Registry::with_builtins();
        let doc = minitaur_patch(&registry);
        let ctx = BuildContext::default();
        ctx.notes.note_on(40, 110); // ~82 Hz, a low bass note
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256).expect("compiles");
        let sink = sink.expect("terminal vca is the sink");
        // Let both envelopes open across a few blocks.
        let mut peak = 0.0_f32;
        for _ in 0..8 {
            let out = process_once(&mut engine, sink);
            assert!(out.iter().all(|s| s.is_finite()));
            peak = peak.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
        assert!(
            peak > 0.01,
            "a held note must make the minitaur audible: {peak}"
        );
    }

    #[test]
    fn minitaur_patch_is_a_faithful_two_vco_voice() {
        // The real Minitaur is TWO VCOs (VCO1 + VCO2), each a saw or square — there is no
        // separate sub oscillator (confirmed against Moog's Minitaur manual / editor docs:
        // the mixer is a two-channel VCO1/VCO2 balance). The clone must therefore carry
        // exactly two `pitch-osc` nodes — one saw (waveform 1.0) and one square (2.0) — and a
        // single `glide` (mono portamento). See docs/synths/minitaur.md.
        let registry = Registry::with_builtins();
        let doc = minitaur_patch(&registry);

        let waveforms: Vec<f32> = doc
            .nodes
            .iter()
            .filter(|e| e.node.kind == "pitch-osc")
            .map(|e| doc.node_values(e.id)[0])
            .collect();
        assert_eq!(
            waveforms.len(),
            2,
            "Minitaur is a two-VCO voice, no separate sub oscillator"
        );
        assert!(
            waveforms.contains(&1.0) && waveforms.contains(&2.0),
            "VCO1 saw (1.0) and VCO2 square (2.0): got {waveforms:?}"
        );
        assert_eq!(
            doc.nodes.iter().filter(|e| e.node.kind == "glide").count(),
            1,
            "mono portamento (one glide)"
        );
    }

    #[test]
    fn minitaur_patch_tracks_the_played_note_pitch() {
        // The whole voice is pitch-tracking: a higher note produces a higher output
        // frequency (more zero crossings over a settled tail).
        let registry = Registry::with_builtins();
        let doc = minitaur_patch(&registry);
        let crossings_for = |note: u8| {
            let ctx = BuildContext::default();
            ctx.notes.note_on(note, 110);
            let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("compiles");
            let sink = sink.expect("sink");
            let mut last = Vec::new();
            for _ in 0..6 {
                last = engine
                    .node_output(sink)
                    .and_then(|b| b.channel(0))
                    .map(<[f32]>::to_vec)
                    .unwrap_or_default();
                engine.process(&ProcessContext {
                    sample_rate: 48_000.0,
                    frames: 512,
                });
            }
            last.windows(2)
                .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
                .count()
        };
        assert!(
            crossings_for(64) > crossings_for(40),
            "a higher note must ring at a higher pitch"
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

#[cfg(test)]
mod minitaur_example {
    /// `examples/minitaur.ron` is generated from `minitaur_patch()`; this keeps the committed
    /// file in lock-step with the builder (regenerate the file if this fails).
    #[test]
    fn the_committed_ron_matches_the_builder() {
        let reg = vb_nodes::Registry::with_builtins();
        let expected = vb_project::to_ron(&super::minitaur_patch(&reg)).expect("serialises");
        let committed = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/minitaur.ron"
        ));
        assert_eq!(
            committed, expected,
            "examples/minitaur.ron is out of sync with minitaur_patch() — regenerate it"
        );
    }
}
