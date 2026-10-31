//! Stale-graph sync check (backlog 000130): the knowledge graph is a derived artifact, so
//! CI asserts a freshly built ontology covers every registry kind, and that the diff check
//! actually detects an out-of-date graph.
//!
//! Compiles only under the `library` feature: `cargo test -p seclab-library --features library`.
#![cfg(feature = "library")]

use seclab_library::Library;
use seclab_nodes::Registry;

#[test]
fn a_built_ontology_is_in_sync_with_the_registry() {
    let registry = Registry::with_builtins();
    let lib = Library::open_in_memory().unwrap();

    // Before building, everything is "missing" — the check detects a stale/absent graph.
    let before = lib.missing_ontology_kinds(&registry).unwrap();
    let kind_count = registry.kinds().count();
    assert_eq!(
        before.len(),
        kind_count,
        "an empty store reports every registry kind as missing"
    );

    // After building, nothing is missing — the ontology is in sync.
    let report = lib.build_registry_ontology(&registry).unwrap();
    assert_eq!(
        report.node_kinds, kind_count,
        "one entity per registry kind"
    );
    assert!(
        lib.missing_ontology_kinds(&registry).unwrap().is_empty(),
        "a freshly built ontology covers every registry kind"
    );
}

#[test]
fn the_diff_names_a_kind_the_graph_is_missing() {
    // A store built from an *empty* registry is missing every kind the real registry has —
    // this is the signal a regenerate-on-release step would act on.
    let full = Registry::with_builtins();
    let lib = Library::open_in_memory().unwrap();
    lib.build_registry_ontology(&Registry::new()).unwrap();

    let missing = lib.missing_ontology_kinds(&full).unwrap();
    assert!(
        missing.iter().any(|k| k == "oscillator"),
        "the diff names kinds absent from the graph: {missing:?}"
    );
}
