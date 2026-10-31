//! Knowledge-graph tests (backlog 000127): the node registry becomes a typed drevo
//! ontology — node kinds linked to categories, ports and parameters, with signal-based
//! port compatibility.
//!
//! Compiles only under the `library` feature: `cargo test -p seclab-library --features library`.
#![cfg(feature = "library")]

use seclab_library::{kind, Library};
use seclab_nodes::Registry;

fn built() -> Library {
    let lib = Library::open_in_memory().unwrap();
    let report = lib
        .build_registry_ontology(&Registry::with_builtins())
        .unwrap();
    assert!(
        report.node_kinds > 10,
        "every builtin kind became an entity"
    );
    assert!(report.categories > 0);
    assert!(report.ports > 0);
    assert!(report.parameters > 0);
    assert!(report.compatibility_edges > 0);
    lib
}

#[test]
fn a_node_kind_links_to_its_category_and_ports() {
    let lib = built();

    let osc = lib
        .node_kind_entity("oscillator")
        .unwrap()
        .expect("oscillator entity");
    assert_eq!(osc.kind, kind::NODE_KIND);
    assert_eq!(osc.body, "oscillator", "plain name kept for search");

    // in_category → generator.
    let category = lib
        .category_of_entity(osc.id)
        .unwrap()
        .expect("has category");
    assert_eq!(category.kind, kind::CATEGORY);
    assert_eq!(category.body, "generator");

    // has_port → an output port named "out".
    let ports = lib.ports_of_entity(osc.id).unwrap();
    assert!(ports.iter().all(|p| p.kind == kind::PORT));
    assert!(
        ports.iter().any(|p| p.body == "out"),
        "the oscillator's output port is present"
    );
}

#[test]
fn ports_are_compatible_only_within_a_signal_type() {
    let lib = built();

    // The oscillator's audio output should be compatible with the biquad's audio input.
    let osc = lib.node_kind_entity("oscillator").unwrap().unwrap();
    let osc_out = lib
        .ports_of_entity(osc.id)
        .unwrap()
        .into_iter()
        .find(|p| p.body == "out")
        .expect("oscillator output");

    let compatible: Vec<String> = lib
        .compatible_inputs(osc_out.id)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    assert!(
        compatible.iter().any(|t| t == "port:biquad/in/in"),
        "audio out connects to the biquad's audio in: {compatible:?}"
    );

    // Every compatible target is an audio input — no control ports leak in. The oscillator
    // out is Audio, so a control input (e.g. an adsr gate) must not appear.
    assert!(
        !compatible.iter().any(|t| t == "port:adsr/in/gate"),
        "an audio output is not compatible with a control input"
    );
}

#[test]
fn authored_knowledge_attaches_with_explains() {
    let lib = built();
    let osc = lib.node_kind_entity("oscillator").unwrap().unwrap();

    let concept = lib
        .add_concept(
            "Band-limited synthesis",
            "Use PolyBLEP to suppress aliasing on saw/square oscillators.",
        )
        .unwrap();
    assert_eq!(concept.kind, kind::CONCEPT);
    let edge = lib.link_explains(concept.id, osc.id).unwrap();
    assert_eq!(edge.from_id, concept.id);
    assert_eq!(edge.to_id, osc.id);

    let heuristic = lib
        .add_heuristic(
            "Bright source for filters",
            "Start a subtractive patch with a saw.",
        )
        .unwrap();
    assert_eq!(heuristic.kind, kind::HEURISTIC);
    lib.link_explains(heuristic.id, osc.id).unwrap();
}
