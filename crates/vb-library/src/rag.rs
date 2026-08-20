//! Retrieval over the library for the assistant (backlog 000109): given a request, fetch
//! the most relevant example patches and render them as compact prompt context (RAG).
//!
//! The assistant grounds a "make me a …" request in real patches the user already has:
//! [`Library::retrieve_patches`] ranks them with the library's FTS, and
//! [`Library::retrieve_context`] formats the top matches — name, description, and the node
//! kinds each contains — into a short block ready to prepend to the model's system prompt.
//! Wiring that block into the live assistant loop is a separate step; this produces the
//! text.

use drevo::error::Result;
use drevo::model::Node;

use crate::library::{kind, Library};

/// Pulls the node kinds a patch was imported with out of its `kinds` property.
fn patch_kinds(node: &Node) -> Vec<String> {
    node.properties
        .0
        .get("kinds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

impl Library {
    /// The `limit` most relevant patches for `query`, ranked by the library's full-text
    /// search. Empty when nothing matches.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn retrieve_patches(&self, query: &str, limit: usize) -> Result<Vec<Node>> {
        Ok(self
            .search_patches(query, limit)?
            .into_iter()
            .map(|scored| scored.node)
            .collect())
    }

    /// Renders the top `limit` matching patches for `query` as a compact context block for
    /// the assistant — one line per patch (`name — description [kinds]`). Returns an empty
    /// string when nothing matches, so the caller can skip injecting an empty section.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn retrieve_context(&self, query: &str, limit: usize) -> Result<String> {
        let patches = self.retrieve_patches(query, limit)?;
        if patches.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("Relevant example patches from the library:\n");
        for node in &patches {
            debug_assert_eq!(node.kind, kind::PATCH);
            out.push_str("- ");
            out.push_str(&node.title);
            if !node.body.is_empty() {
                out.push_str(" — ");
                out.push_str(&node.body);
            }
            let kinds = patch_kinds(node);
            if !kinds.is_empty() {
                out.push_str(" [");
                out.push_str(&kinds.join(", "));
                out.push(']');
            }
            out.push('\n');
        }
        Ok(out)
    }
}
