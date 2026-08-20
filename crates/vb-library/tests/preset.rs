//! Preset + preset-bank tests (backlog 000098): presets link to their patch, carry their
//! values, and group into banks.
//!
//! Compiles only under the `library` feature: `cargo test -p vb-library --features library`.
#![cfg(feature = "library")]

use std::collections::HashSet;

use vb_core::{GraphNode, NodeId, Port, SignalType};
use vb_library::{import_patch, kind, Library};
use vb_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

fn tiny_doc() -> ProjectDoc {
    ProjectDoc {
        version: FORMAT_VERSION,
        nodes: vec![NodeEntry {
            id: NodeId(0),
            node: GraphNode::new(
                "oscillator",
                vec![],
                vec![Port::output("out", SignalType::Audio)],
            ),
        }],
        edges: vec![],
    }
}

#[test]
fn a_preset_links_to_its_patch_and_stores_values() {
    let lib = Library::open_in_memory().unwrap();
    let patch = import_patch(&lib, &tiny_doc(), "Bass", "warm").unwrap();

    let preset = lib
        .add_preset_for(
            patch.id,
            "Bass — Bright",
            &[("gain", 0.8), ("cutoff", 1200.0)],
        )
        .unwrap();
    assert_eq!(preset.kind, kind::PRESET);

    // preset → patch link.
    let owner = lib
        .patch_of_preset(preset.id)
        .unwrap()
        .expect("has a patch");
    assert_eq!(owner.id, patch.id);
    assert_eq!(owner.title, "Bass");

    // Values stored in properties (as f32, so compare with a tolerance).
    let values = &preset.properties.0["values"];
    assert!((values["gain"].as_f64().unwrap() - 0.8).abs() < 1e-6);
    assert!((values["cutoff"].as_f64().unwrap() - 1200.0).abs() < 1e-3);
}

#[test]
fn presets_for_patch_lists_only_its_presets() {
    let lib = Library::open_in_memory().unwrap();
    let patch = import_patch(&lib, &tiny_doc(), "Lead", "bright").unwrap();
    let other = import_patch(&lib, &tiny_doc(), "Pad", "soft").unwrap();

    lib.add_preset_for(patch.id, "Lead — Soft", &[("gain", 0.5)])
        .unwrap();
    lib.add_preset_for(patch.id, "Lead — Hard", &[("gain", 1.0)])
        .unwrap();
    lib.add_preset_for(other.id, "Pad — Warm", &[("gain", 0.3)])
        .unwrap();

    let names: HashSet<String> = lib
        .presets_for_patch(patch.id)
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert_eq!(
        names,
        HashSet::from(["Lead — Soft".to_owned(), "Lead — Hard".to_owned()])
    );
    // The other patch's preset is not included.
    assert_eq!(lib.presets_for_patch(other.id).unwrap().len(), 1);
}

#[test]
fn a_bank_groups_presets() {
    let lib = Library::open_in_memory().unwrap();
    let patch = import_patch(&lib, &tiny_doc(), "Keys", "electric").unwrap();
    let p1 = lib
        .add_preset_for(patch.id, "Keys — Dark", &[("gain", 0.4)])
        .unwrap();
    let p2 = lib
        .add_preset_for(patch.id, "Keys — Air", &[("gain", 0.9)])
        .unwrap();

    let bank = lib.create_bank("Factory").unwrap();
    assert_eq!(bank.kind, kind::PRESET_BANK);
    lib.add_to_bank(bank.id, p1.id).unwrap();
    lib.add_to_bank(bank.id, p2.id).unwrap();

    let in_bank: HashSet<u64> = lib
        .presets_in_bank(bank.id)
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(in_bank, HashSet::from([p1.id, p2.id]));

    // An empty bank groups nothing.
    let empty = lib.create_bank("Empty").unwrap();
    assert!(lib.presets_in_bank(empty.id).unwrap().is_empty());
}
