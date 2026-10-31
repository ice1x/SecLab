//! Integration assessment for the drevo-backed library (backlog 000093): open an embedded
//! store and prove basic CRUD works through drevo's direct Rust API — no Bolt server.
//!
//! The whole file compiles only under the `library` feature; run it with
//! `cargo test -p vb-library --features library`. Without the feature drevo is not a
//! dependency and this is empty.
#![cfg(feature = "library")]

use std::collections::HashMap;

use drevo::model::{NewNode, NodePatch, Properties};

fn patch_node(title: &str) -> NewNode {
    NewNode {
        kind: "patch".to_owned(),
        title: title.to_owned(),
        body: String::new(),
        body_html: String::new(),
        properties: Properties::from(HashMap::new()),
    }
}

#[test]
fn embedded_store_supports_create_read_update_delete() {
    let db = vb_library::open_in_memory().expect("open an in-memory drevo store");

    // Create.
    let created = db
        .create_node(patch_node("Fat Bass"))
        .expect("create a patch node");
    assert_eq!(created.kind, "patch");
    assert_eq!(created.title, "Fat Bass");
    let id = created.id;

    // Read.
    let fetched = db.get_node(id).expect("read the node");
    assert_eq!(fetched.id, id);
    assert_eq!(fetched.title, "Fat Bass");

    // Update (partial patch: only the title changes).
    let updated = db
        .update_node(
            id,
            NodePatch {
                title: Some("Screaming Lead".to_owned()),
                ..Default::default()
            },
        )
        .expect("update the node");
    assert_eq!(updated.title, "Screaming Lead");
    assert_eq!(updated.kind, "patch", "untouched fields are preserved");
    assert_eq!(
        db.get_node(id).unwrap().title,
        "Screaming Lead",
        "the change persisted"
    );

    // Delete. `NativeService::get_node` returns `NodeNotFound` (not `Ok(None)`) for a
    // missing id, so a read after deletion is an error.
    db.delete_node(id).expect("delete the node");
    assert!(db.get_node(id).is_err(), "the node is gone after deletion");
}

#[test]
fn stored_properties_round_trip() {
    let db = vb_library::open_in_memory().expect("open store");

    let mut props = HashMap::new();
    props.insert("tags".to_owned(), serde_json::json!(["bass", "analog"]));
    props.insert("author".to_owned(), serde_json::json!("ice"));
    let node = NewNode {
        kind: "patch".to_owned(),
        title: "Tagged".to_owned(),
        body: "notes".to_owned(),
        body_html: String::new(),
        properties: Properties::from(props),
    };

    let created = db.create_node(node).expect("create");
    let fetched = db.get_node(created.id).unwrap();
    assert_eq!(
        fetched.properties.get("author").unwrap(),
        &serde_json::json!("ice")
    );
    assert_eq!(fetched.body, "notes");
}
