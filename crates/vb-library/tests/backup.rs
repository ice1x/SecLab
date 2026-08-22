//! Backup/restore tests (backlog 000100): the library exports to a JSON snapshot and a
//! fresh library imports it, recovering nodes, links and searchability; re-import is
//! idempotent.
//!
//! Compiles only under the `library` feature: `cargo test -p vb-library --features library`.
#![cfg(feature = "library")]

use vb_core::{GraphNode, NodeId, Port, SignalType};
use vb_library::{import_patch, Library};
use vb_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

fn tiny_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![NodeEntry::new(
            NodeId(0),
            GraphNode::new(
                "oscillator",
                vec![],
                vec![Port::output("out", SignalType::Audio)],
            ),
        )],
        edges: vec![],
    }
}

/// Builds a small library: a patch with a preset in a bank.
fn seed(lib: &Library) {
    let patch = import_patch(lib, &tiny_doc(), "Fat Bass", "warm analog low end").unwrap();
    let preset = lib
        .add_preset_for(patch.id, "Fat Bass — Bright", &[("gain", 0.8)])
        .unwrap();
    let bank = lib.create_bank("Factory").unwrap();
    lib.add_to_bank(bank.id, preset.id).unwrap();
}

#[test]
fn a_library_round_trips_through_a_json_backup() {
    let source = Library::open_in_memory().unwrap();
    seed(&source);
    let backup = source.export_json().unwrap();
    assert!(!backup.is_empty(), "the backup has content");

    // Restore into a fresh, empty library.
    let restored = Library::open_in_memory().unwrap();
    let report = restored.import_json(&backup).unwrap();
    assert!(report.nodes_imported > 0, "nodes were restored");
    assert!(report.edges_imported > 0, "links were restored");

    // The patch is back and searchable, with its preset and bank links intact.
    let hits = restored.search_patches("bass", 10).unwrap();
    assert_eq!(hits.len(), 1);
    let patch = &hits[0].node;
    assert_eq!(patch.title, "Fat Bass");

    let presets = restored.presets_for_patch(patch.id).unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].title, "Fat Bass — Bright");
    // The preset still belongs to its patch.
    assert_eq!(
        restored.patch_of_preset(presets[0].id).unwrap().unwrap().id,
        patch.id
    );
}

#[test]
fn re_importing_a_backup_is_idempotent() {
    let source = Library::open_in_memory().unwrap();
    seed(&source);
    let backup = source.export_json().unwrap();

    let restored = Library::open_in_memory().unwrap();
    let first = restored.import_json(&backup).unwrap();
    assert!(first.nodes_imported > 0);

    // Second import of the same snapshot adds nothing and skips the byte-equal records.
    let second = restored.import_json(&backup).unwrap();
    assert_eq!(second.nodes_imported, 0, "nothing new on re-import");
    assert!(
        second.nodes_skipped >= first.nodes_imported,
        "the already-present nodes were skipped"
    );
}
