//! Starter example projects: each committed patch under `examples/` must load, validate,
//! and build a working engine with an audio sink. These are the "first patch" content a
//! user opens, so a regression that breaks one (a renamed node, a changed port) fails CI.

use vb_nodes::{BuildContext, Registry};
use vb_plugin::engine_builder::engine_from_doc;

/// (name, committed RON) for every shipped example.
const EXAMPLES: &[(&str, &str)] = &[
    ("synth", include_str!("../../../examples/synth.ron")),
    ("delay", include_str!("../../../examples/delay.ron")),
    ("unison", include_str!("../../../examples/unison.ron")),
    ("tremolo", include_str!("../../../examples/tremolo.ron")),
    (
        "filter_sweep",
        include_str!("../../../examples/filter_sweep.ron"),
    ),
    ("vibrato", include_str!("../../../examples/vibrato.ron")),
    (
        "first_synth",
        include_str!("../../../examples/first_synth.ron"),
    ),
    (
        "ladder_bass",
        include_str!("../../../examples/ladder_bass.ron"),
    ),
    ("mono_lead", include_str!("../../../examples/mono_lead.ron")),
];

#[test]
fn every_example_loads_validates_and_builds_an_engine() {
    let registry = Registry::with_builtins();
    let ctx = BuildContext::default();

    for (name, ron) in EXAMPLES {
        // Loads and validates (open_ron runs the structural validator).
        let doc = vb_project::open_ron(ron)
            .unwrap_or_else(|e| panic!("example '{name}' failed to load/validate: {e}"));
        assert!(doc.is_valid(), "example '{name}' is not valid");

        // Builds a real engine at a typical block size, with an audio sink to play.
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 256)
            .unwrap_or_else(|e| panic!("example '{name}' failed to build an engine: {e:?}"));
        assert!(sink.is_some(), "example '{name}' has no audio sink");

        // And processes a block without panicking or producing non-finite samples.
        engine.process(&vb_core::ProcessContext {
            sample_rate: 48_000.0,
            frames: 256,
        });
        let out = engine
            .node_output(sink.unwrap())
            .and_then(|b| b.channel(0))
            .expect("sink output");
        assert!(
            out.iter().all(|s| s.is_finite()),
            "example '{name}' produced non-finite audio"
        );
    }
}

#[test]
fn ladder_bass_is_silent_at_rest_and_sings_on_a_note() {
    // The ladder-bass factory patch: an envelope-swept resonant ladder gated by a note.
    // Its stored VCA bias = 0 keeps it silent until a note opens the envelope.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/ladder_bass.ron");
    let doc = vb_project::open_ron(ron).expect("ladder_bass loads");

    let peak = |ctx: &BuildContext| {
        let (mut engine, sink) = engine_from_doc(&doc, &registry, ctx, 256).expect("builds");
        let sink = sink.expect("has a sink");
        // A few blocks so the filter/amp envelope has time to open.
        let mut p = 0.0_f32;
        for _ in 0..8 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 256,
            });
            let out = engine.node_output(sink).and_then(|b| b.channel(0)).unwrap();
            p = p.max(out.iter().fold(0.0, |m, s| m.max(s.abs())));
        }
        p
    };

    let at_rest = peak(&BuildContext::default());
    assert!(
        at_rest < 1e-6,
        "ladder_bass must be silent at rest: {at_rest}"
    );

    let ctx = BuildContext::default();
    ctx.notes.note_on(45, 100); // ~110 Hz
    let on_note = peak(&ctx);
    assert!(
        on_note > 0.01,
        "ladder_bass must sound on a note: {on_note}"
    );
}

#[test]
fn mono_lead_tracks_the_played_note_pitch() {
    // The mono-lead factory patch is playable: its pitch-tracking oscillator follows
    // midi-in.pitch_hz, so different notes come out at different pitches. Assert the
    // whole engine — midi → pitch-osc → ladder → vca — produces a higher output
    // frequency for a higher note.
    let registry = Registry::with_builtins();
    let ron = include_str!("../../../examples/mono_lead.ron");
    let doc = vb_project::open_ron(ron).expect("mono_lead loads");

    // Zero crossings over a rendered tail — proportional to the output frequency.
    let crossings_for = |note: u8| {
        let ctx = BuildContext::default();
        ctx.notes.note_on(note, 110);
        let (mut engine, sink) = engine_from_doc(&doc, &registry, &ctx, 512).expect("builds");
        let sink = sink.expect("has a sink");
        // Let the envelope open, then count crossings over a settled block.
        for _ in 0..6 {
            engine.process(&vb_core::ProcessContext {
                sample_rate: 48_000.0,
                frames: 512,
            });
        }
        let out = engine
            .node_output(sink)
            .and_then(|b| b.channel(0))
            .unwrap()
            .to_vec();
        out.windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count()
    };

    let low = crossings_for(45); // ~110 Hz
    let high = crossings_for(69); // 440 Hz, two octaves up
    assert!(
        high > low * 2,
        "a higher note must sound higher: {low} vs {high} crossings"
    );
}

#[test]
fn the_example_set_is_the_expected_manifest() {
    // A guard so the committed set and this manifest stay in step.
    let names: Vec<&str> = EXAMPLES.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        vec![
            "synth",
            "delay",
            "unison",
            "tremolo",
            "filter_sweep",
            "vibrato",
            "first_synth",
            "ladder_bass",
            "mono_lead"
        ]
    );
    // No example is empty.
    assert!(EXAMPLES.iter().all(|(_, ron)| !ron.trim().is_empty()));
}
