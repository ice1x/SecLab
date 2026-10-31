//! Storage-schema tests for the drevo-backed library (backlog 000094): the typed
//! `Library` layer stores patch / macro-node / preset / tag nodes and links them with
//! uses / derived-from / tagged edges, and traversals read them back.
//!
//! Compiles only under the `library` feature: `cargo test -p seclab-library --features library`.
#![cfg(feature = "library")]

use seclab_library::{kind, rel, Library};

#[test]
fn nodes_carry_the_declared_library_kinds() {
    let lib = Library::open_in_memory().expect("open library");

    let patch = lib.add_patch("Fat Bass").unwrap();
    let macro_node = lib.add_macro_node("Unison Osc").unwrap();
    let preset = lib.add_preset("Bass — Bright").unwrap();
    let tag = lib.add_tag("bass").unwrap();

    assert_eq!(patch.kind, kind::PATCH);
    assert_eq!(macro_node.kind, kind::MACRO_NODE);
    assert_eq!(preset.kind, kind::PRESET);
    assert_eq!(tag.kind, kind::TAG);
    // Distinct ids were allocated.
    assert_eq!(
        [patch.id, macro_node.id, preset.id, tag.id]
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
}

#[test]
fn uses_derived_from_and_tagged_links_traverse() {
    let lib = Library::open_in_memory().expect("open library");

    // A patch that uses a macro-node, a preset derived from that patch, and a tag on the
    // patch.
    let patch = lib.add_patch("Lead").unwrap();
    let macro_node = lib.add_macro_node("Detune Stack").unwrap();
    let preset = lib.add_preset("Lead — Soft").unwrap();
    let tag = lib.add_tag("lead").unwrap();

    let use_edge = lib.link_uses(patch.id, macro_node.id).unwrap();
    assert_eq!(use_edge.kind, rel::USES);
    lib.link_derived_from(preset.id, patch.id).unwrap();
    lib.tag(patch.id, tag.id).unwrap();

    // uses: the patch reaches its macro-node.
    let used = lib.macro_nodes_used_by(patch.id).unwrap();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0].id, macro_node.id);
    assert_eq!(used[0].kind, kind::MACRO_NODE);

    // derived-from: the preset points back to the patch.
    let originals = lib.originals_of(preset.id).unwrap();
    assert_eq!(originals.len(), 1);
    assert_eq!(originals[0].id, patch.id);

    // tagged: forward (the patch's tags) and backward (items with the tag).
    let tags = lib.tags_of(patch.id).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].id, tag.id);

    let tagged_items = lib.items_with_tag(tag.id).unwrap();
    assert_eq!(tagged_items.len(), 1);
    assert_eq!(tagged_items[0].id, patch.id);
}

#[test]
fn traversals_are_kind_scoped() {
    // A patch both uses a macro-node and is tagged; each traversal returns only its own
    // edge kind, not every neighbour.
    let lib = Library::open_in_memory().expect("open library");
    let patch = lib.add_patch("Pad").unwrap();
    let macro_node = lib.add_macro_node("Chorus").unwrap();
    let tag = lib.add_tag("ambient").unwrap();

    lib.link_uses(patch.id, macro_node.id).unwrap();
    lib.tag(patch.id, tag.id).unwrap();

    // `uses` traversal sees only the macro-node, not the tag.
    let used = lib.macro_nodes_used_by(patch.id).unwrap();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0].id, macro_node.id);

    // `tagged` traversal sees only the tag, not the macro-node.
    let tags = lib.tags_of(patch.id).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].id, tag.id);
}
