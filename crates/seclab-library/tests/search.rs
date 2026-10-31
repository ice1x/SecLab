//! Full-text search over the library (backlog 000096): drevo's BM25 FTS ranks patches by
//! name and description, and `search_patches` filters to real patches.
//!
//! Compiles only under the `library` feature: `cargo test -p seclab-library --features library`.
#![cfg(feature = "library")]

use seclab_core::{GraphNode, NodeId, Port, SignalType};
use seclab_library::{import_patch, kind, Library};
use seclab_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

fn audio_out(name: &str) -> Port {
    Port::output(name, SignalType::Audio)
}

/// A minimal one-node doc (an oscillator) — enough to import; the search is over the
/// patch's name and description, not its structure.
fn tiny_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![NodeEntry::new(
            NodeId(0),
            GraphNode::new("oscillator", vec![], vec![audio_out("out")]),
        )],
        edges: vec![],
    }
}

fn titles(hits: &[seclab_library::ScoredNode]) -> Vec<String> {
    hits.iter().map(|h| h.node.title.clone()).collect()
}

#[test]
fn search_finds_patches_by_name() {
    let lib = Library::open_in_memory().unwrap();
    import_patch(&lib, &tiny_doc(), "Fat Bass", "warm analog low end").unwrap();
    import_patch(&lib, &tiny_doc(), "Bright Lead", "screaming saw solo").unwrap();

    let hits = lib.search_patches("bass", 10).unwrap();
    assert_eq!(titles(&hits), vec!["Fat Bass"], "name match");
    assert!(hits[0].score > 0.0, "a relevance score is attached");
}

#[test]
fn search_finds_patches_by_description() {
    let lib = Library::open_in_memory().unwrap();
    import_patch(&lib, &tiny_doc(), "Hall", "a long cathedral reverb space").unwrap();
    import_patch(&lib, &tiny_doc(), "Pluck", "short percussive stab").unwrap();

    // "cathedral" appears only in the description, so FTS must index the body.
    let hits = lib.search_patches("cathedral", 10).unwrap();
    assert_eq!(titles(&hits), vec!["Hall"], "description match");
}

#[test]
fn search_patches_filters_out_non_patch_nodes() {
    let lib = Library::open_in_memory().unwrap();
    // Importing creates a `kind:oscillator` tag node behind the scenes, and the patch's
    // stored `kinds` metadata also contains "oscillator". A raw search for "oscillator"
    // therefore matches both the tag node and the patch; search_patches must drop the tag.
    import_patch(&lib, &tiny_doc(), "Some Pad", "lush evolving texture").unwrap();

    let raw = lib.search("oscillator", 10).unwrap();
    assert!(
        raw.iter().any(|h| h.node.kind == kind::TAG),
        "the raw search sees the kind:oscillator tag node"
    );

    let patches = lib.search_patches("oscillator", 10).unwrap();
    assert!(
        patches.iter().all(|h| h.node.kind == kind::PATCH),
        "search_patches returns only patches"
    );
    assert!(
        raw.len() > patches.len(),
        "the non-patch (tag) hit was filtered out: raw {} vs patches {}",
        raw.len(),
        patches.len()
    );
}

#[test]
fn a_query_matching_nothing_returns_empty() {
    let lib = Library::open_in_memory().unwrap();
    import_patch(&lib, &tiny_doc(), "Only Patch", "nothing relevant here").unwrap();
    assert!(lib.search_patches("zzzqqq", 10).unwrap().is_empty());
}
