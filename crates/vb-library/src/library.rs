//! The library's storage schema over drevo (backlog 000094): a typed layer that stores
//! **patches**, **macro-nodes**, **presets** and **tags** as graph nodes, linked by
//! **uses**, **derived-from** and **tagged** edges.
//!
//! drevo itself is schema-less — nodes and edges carry a free-form `kind` string. This
//! module pins down the small vocabulary the library uses ([`kind`] / [`rel`]) and wraps
//! drevo's generic CRUD in intention-revealing methods ([`Library::add_patch`],
//! [`Library::link_uses`], …) so the rest of VST_bomb never hand-writes kind strings. The
//! RON project files remain the source of truth for audio structure; this graph stores
//! only metadata and the links between library items.

use std::collections::HashMap;

use drevo::error::Result;
use drevo::model::{Direction, Edge, NewEdge, NewNode, Node, Properties, ScoredNode};
use drevo::native_service::NativeService;

/// Node `kind` values the library stores.
pub mod kind {
    /// A saved patch (a whole instrument/effect graph). The RON file is the source of
    /// truth; this node holds its metadata.
    pub const PATCH: &str = "patch";
    /// A reusable macro-node (a nested subgraph packaged as one node).
    pub const MACRO_NODE: &str = "macro-node";
    /// A preset (a set of parameter values) for a patch.
    pub const PRESET: &str = "preset";
    /// A named collection of presets.
    pub const PRESET_BANK: &str = "preset-bank";
    /// A free-form tag used to organise the library.
    pub const TAG: &str = "tag";

    // --- Knowledge-graph ontology (000127) ---

    /// A node kind in the DSP library (e.g. `oscillator`), as a knowledge-graph entity.
    pub const NODE_KIND: &str = "node-kind";
    /// A port on a node kind (an input or output, with a signal type).
    pub const PORT: &str = "port";
    /// A parameter on a node kind (name, range, default).
    pub const PARAMETER: &str = "parameter";
    /// A UI category grouping node kinds (e.g. `generator`, `filter`).
    pub const CATEGORY: &str = "category";
    /// An authored DSP concept the assistant can cite.
    pub const CONCEPT: &str = "concept";
    /// An authored DSP heuristic ("when X, prefer Y").
    pub const HEURISTIC: &str = "heuristic";
    /// An example patch, referenced from the knowledge graph.
    pub const EXAMPLE_PATCH: &str = "example-patch";
}

/// Edge `kind` values the library stores.
pub mod rel {
    /// `patch --uses--> macro-node`: the patch embeds that macro-node.
    pub const USES: &str = "uses";
    /// `derivative --derived-from--> original`: one item was created from another
    /// (a preset from a patch, a patch forked from a patch).
    pub const DERIVED_FROM: &str = "derived-from";
    /// `item --tagged--> tag`: the item carries that tag.
    pub const TAGGED: &str = "tagged";
    /// `preset-bank --contains--> preset`: the bank groups that preset.
    pub const CONTAINS: &str = "contains";

    // --- Knowledge-graph ontology (000127) ---

    /// `node-kind --has_port--> port`.
    pub const HAS_PORT: &str = "has_port";
    /// `node-kind --has_parameter--> parameter`.
    pub const HAS_PARAMETER: &str = "has_parameter";
    /// `node-kind --in_category--> category`.
    pub const IN_CATEGORY: &str = "in_category";
    /// `output-port --compatible_with--> input-port` (same signal type).
    pub const COMPATIBLE_WITH: &str = "compatible_with";
    /// `concept|heuristic --explains--> node-kind` (authored knowledge).
    pub const EXPLAINS: &str = "explains";
}

/// Builds a library node with the given kind/title (empty body and no properties).
pub(crate) fn new_node(node_kind: &str, title: &str) -> NewNode {
    NewNode {
        kind: node_kind.to_owned(),
        title: title.to_owned(),
        body: String::new(),
        body_html: String::new(),
        properties: Properties(HashMap::new()),
    }
}

/// Builds a library edge of the given kind at unit weight (no properties).
pub(crate) fn new_edge(from_id: u64, to_id: u64, edge_kind: &str) -> NewEdge {
    NewEdge {
        from_id,
        to_id,
        kind: edge_kind.to_owned(),
        weight: 1.0,
        properties: Properties(HashMap::new()),
    }
}

/// The patch/preset library, backed by drevo's embedded [`NativeService`] handle.
///
/// drevo retired both its redb on-disk backend and its legacy in-memory KV `Drevo` handle
/// (drevo epic #444); `NativeService` is the WAL-native store of record it now exposes. This
/// binds to its in-memory mode ([`NativeService::in_memory`]). Durable persistence would use
/// [`NativeService::open`] with a path — not yet wired here.
pub struct Library {
    db: NativeService,
}

impl Library {
    /// Opens a transient in-memory library (no file, no server).
    ///
    /// # Errors
    /// Returns a drevo error type for signature stability; the in-memory open is infallible.
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            db: NativeService::in_memory(),
        })
    }

    /// The underlying drevo store, for queries this typed layer does not yet wrap.
    #[must_use]
    pub fn db(&self) -> &NativeService {
        &self.db
    }

    /// Adds a patch node with `title`.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn add_patch(&self, title: &str) -> Result<Node> {
        self.db.create_node(new_node(kind::PATCH, title))
    }

    /// Adds a macro-node node with `title`.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn add_macro_node(&self, title: &str) -> Result<Node> {
        self.db.create_node(new_node(kind::MACRO_NODE, title))
    }

    /// Adds a preset node with `title`.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn add_preset(&self, title: &str) -> Result<Node> {
        self.db.create_node(new_node(kind::PRESET, title))
    }

    /// Adds (or returns a fresh) tag node named `name`.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn add_tag(&self, name: &str) -> Result<Node> {
        self.db.create_node(new_node(kind::TAG, name))
    }

    /// Links `patch --uses--> macro_node`.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn link_uses(&self, patch_id: u64, macro_node_id: u64) -> Result<Edge> {
        self.db
            .create_edge(new_edge(patch_id, macro_node_id, rel::USES))
    }

    /// Links `derivative --derived-from--> original`.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn link_derived_from(&self, derivative_id: u64, original_id: u64) -> Result<Edge> {
        self.db
            .create_edge(new_edge(derivative_id, original_id, rel::DERIVED_FROM))
    }

    /// Links `item --tagged--> tag`.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn tag(&self, item_id: u64, tag_id: u64) -> Result<Edge> {
        self.db.create_edge(new_edge(item_id, tag_id, rel::TAGGED))
    }

    /// The macro-nodes a patch uses (`patch --uses--> …`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn macro_nodes_used_by(&self, patch_id: u64) -> Result<Vec<Node>> {
        Ok(self
            .db
            .neighbors(patch_id, Direction::Outgoing, Some(rel::USES)))
    }

    /// The items an item was derived from (`item --derived-from--> …`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn originals_of(&self, item_id: u64) -> Result<Vec<Node>> {
        Ok(self
            .db
            .neighbors(item_id, Direction::Outgoing, Some(rel::DERIVED_FROM)))
    }

    /// The tags on an item (`item --tagged--> …`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn tags_of(&self, item_id: u64) -> Result<Vec<Node>> {
        Ok(self
            .db
            .neighbors(item_id, Direction::Outgoing, Some(rel::TAGGED)))
    }

    /// The items carrying a given tag (`… --tagged--> tag`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn items_with_tag(&self, tag_id: u64) -> Result<Vec<Node>> {
        Ok(self
            .db
            .neighbors(tag_id, Direction::Incoming, Some(rel::TAGGED)))
    }

    /// Full-text search across every library node's name and description, ranked by
    /// relevance (drevo's BM25 FTS), returning up to `limit` scored hits.
    ///
    /// Note: drevo's FTS is trigram-based, so queries shorter than three characters match
    /// nothing.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<ScoredNode>> {
        Ok(self.db.search_fts(query, limit))
    }

    /// Like [`search`](Self::search) but returns only **patch** hits (dropping tag and
    /// other node kinds), still ranked, up to `limit`. Internally it over-fetches so the
    /// filter does not starve the result.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn search_patches(&self, query: &str, limit: usize) -> Result<Vec<ScoredNode>> {
        // Over-fetch (bounded) before filtering to patches, so tag hits don't crowd out
        // real patches within `limit`.
        let over = limit.saturating_mul(4).max(limit).min(1_000);
        let mut hits = self.db.search_fts(query, over);
        hits.retain(|scored| scored.node.kind == kind::PATCH);
        hits.truncate(limit);
        Ok(hits)
    }
}
