//! Importing a patch into the library (backlog 000095): read a [`vb_project::ProjectDoc`]
//! and write its **metadata** and **links** into drevo, while the RON file stays the
//! source of truth for the audio structure.
//!
//! A patch becomes one [`crate::kind::PATCH`] node whose properties summarise it
//! (node/edge counts, the distinct node kinds it contains, the format version). Each
//! distinct node kind is also linked as a facet: the patch is [`crate::rel::TAGGED`] to a
//! `kind:<name>` tag node, so the library can answer "which patches contain a reverb?"
//! ([`patches_with_node_kind`]) without re-parsing every RON.

use std::collections::BTreeSet;
use std::collections::HashMap;

use drevo::error::Result;
use drevo::model::{NewNode, Node, Properties};
use vb_project::ProjectDoc;

use crate::library::{kind, Library};

/// Title of the tag node standing for a structural node kind (namespaced so it never
/// collides with a user-facing patch/tag title, which drevo requires to be unique).
fn kind_tag_title(node_kind: &str) -> String {
    format!("kind:{node_kind}")
}

/// The distinct node kinds a patch contains, sorted.
fn distinct_kinds(doc: &ProjectDoc) -> Vec<String> {
    doc.nodes
        .iter()
        .map(|entry| entry.node.kind.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Gets or creates the `kind:<name>` tag node for a structural node kind.
fn kind_tag(lib: &Library, node_kind: &str) -> Result<Node> {
    let title = kind_tag_title(node_kind);
    match lib.db().get_node_by_title(&title)? {
        Some(existing) if existing.kind == kind::TAG => Ok(existing),
        _ => lib.add_tag(&title),
    }
}

/// Imports `doc` as a patch named `title`: creates the patch node with summary metadata
/// and tags it with each structural node kind it contains.
///
/// The `title` is the library key and must be unique (drevo enforces unique titles);
/// re-importing under an existing title returns [`drevo::error::DrevoError::DuplicateTitle`].
///
/// # Errors
/// Propagates any drevo error (including a duplicate title).
pub fn import_patch(lib: &Library, doc: &ProjectDoc, title: &str) -> Result<Node> {
    let kinds = distinct_kinds(doc);

    let mut props = HashMap::new();
    props.insert("node_count".to_owned(), serde_json::json!(doc.nodes.len()));
    props.insert("edge_count".to_owned(), serde_json::json!(doc.edges.len()));
    props.insert("format_version".to_owned(), serde_json::json!(doc.version));
    props.insert("kinds".to_owned(), serde_json::json!(kinds));

    let patch = lib.db().create_node(NewNode {
        kind: kind::PATCH.to_owned(),
        title: title.to_owned(),
        body: String::new(),
        body_html: String::new(),
        properties: Properties(props),
    })?;

    for node_kind in &kinds {
        let tag = kind_tag(lib, node_kind)?;
        lib.tag(patch.id, tag.id)?;
    }

    Ok(patch)
}

/// The patches that contain a given structural node kind (e.g. `"reverb"`), via the
/// `kind:<name>` tag. Empty if no patch uses that kind.
///
/// # Errors
/// Propagates any drevo error.
pub fn patches_with_node_kind(lib: &Library, node_kind: &str) -> Result<Vec<Node>> {
    let title = kind_tag_title(node_kind);
    let Some(tag) = lib.db().get_node_by_title(&title)? else {
        return Ok(Vec::new());
    };
    let mut patches = lib.items_with_tag(tag.id)?;
    patches.retain(|node| node.kind == kind::PATCH);
    Ok(patches)
}
