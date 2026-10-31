//! Knowledge retrieval tests (backlog 000128): the assistant fetches only the ontology
//! slice a question needs, far smaller than the full node-card corpus.
//!
//! Compiles only under the `library` feature: `cargo test -p seclab-library --features library`.
#![cfg(feature = "library")]

use seclab_library::Library;
use seclab_nodes::{node_reference, Registry};

fn built() -> Library {
    let lib = Library::open_in_memory().unwrap();
    lib.build_registry_ontology(&Registry::with_builtins())
        .unwrap();
    lib
}

#[test]
fn a_slice_pulls_only_the_relevant_node_kinds() {
    let lib = built();

    // "filter" matches the filter category, which yields its node-kinds (biquad, svf, …).
    let slice = lib.knowledge_slice("filter", 5).unwrap();
    assert!(!slice.is_empty());
    assert!(
        slice.contains("biquad"),
        "a filter kind is included: {slice}"
    );
    assert!(
        !slice.contains("oscillator"),
        "an unrelated generator is not pulled in: {slice}"
    );
    // The card carries its ports and parameters.
    assert!(slice.contains("ports:"));
    assert!(slice.contains("params:"));
}

#[test]
fn a_slice_is_much_smaller_than_the_full_node_card_corpus() {
    let lib = built();
    let full = node_reference(&Registry::with_builtins());
    let slice = lib.knowledge_slice("oscillator", 1).unwrap();

    assert!(slice.contains("oscillator"));
    assert!(
        slice.len() * 3 < full.len(),
        "a one-kind slice ({} bytes) is a fraction of the full corpus ({} bytes)",
        slice.len(),
        full.len()
    );
}

#[test]
fn a_query_matching_a_parameter_retrieves_its_owning_kind() {
    let lib = built();
    // "cutoff" is a biquad parameter; retrieving it should surface the biquad card.
    let slice = lib.knowledge_slice("cutoff", 5).unwrap();
    assert!(
        slice.contains("biquad"),
        "matching a parameter pulls in its node-kind: {slice}"
    );
}

#[test]
fn no_match_yields_an_empty_slice() {
    let lib = built();
    assert_eq!(lib.knowledge_slice("zzzqqq", 5).unwrap(), "");
}
