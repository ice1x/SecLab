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
fn the_example_set_is_the_expected_three() {
    // A guard so the committed set and this manifest stay in step.
    let names: Vec<&str> = EXAMPLES.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, vec!["synth", "delay", "unison", "tremolo"]);
    // No example is empty.
    assert!(EXAMPLES.iter().all(|(_, ron)| !ron.trim().is_empty()));
}
