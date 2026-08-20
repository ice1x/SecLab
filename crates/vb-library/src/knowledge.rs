//! Docs → knowledge graph (backlog 000127): turn the live node registry into a typed
//! drevo ontology the assistant can query.
//!
//! [`Library::build_registry_ontology`] walks [`vb_nodes::Registry`] and writes, for every
//! node kind, a [`crate::kind::NODE_KIND`] entity linked to its [`crate::kind::CATEGORY`]
//! (`in_category`), its [`crate::kind::PORT`]s (`has_port`) and its
//! [`crate::kind::PARAMETER`]s (`has_parameter`). It then adds `compatible_with` edges
//! from every output port to every input port of the same signal type — the port
//! compatibility rules, as a graph. Authored knowledge ([`crate::kind::CONCEPT`],
//! [`crate::kind::HEURISTIC`], [`crate::kind::EXAMPLE_PATCH`]) attaches with `explains` /
//! `uses` via the helper methods; the registry pass populates the structural core.
//!
//! Node titles must be unique in drevo, so every entity's title is namespaced by type
//! (`nodekind:…`, `category:…`, `port:…`, `param:…`); the plain name lives in the body for
//! search.

use std::collections::HashMap;

use drevo::error::Result;
use drevo::model::{Direction, Edge, NewNode, Node, Properties};
use vb_nodes::Registry;

use crate::library::{kind, new_edge, rel, Library};

fn nodekind_title(k: &str) -> String {
    format!("nodekind:{k}")
}
fn category_title(c: &str) -> String {
    format!("category:{c}")
}
fn port_title(node_kind: &str, dir: &str, name: &str) -> String {
    format!("port:{node_kind}/{dir}/{name}")
}
fn param_title(node_kind: &str, name: &str) -> String {
    format!("param:{node_kind}/{name}")
}

/// Builds a knowledge-graph node with a body and properties.
fn kg_node(
    entity_kind: &str,
    title: String,
    body: String,
    props: HashMap<String, serde_json::Value>,
) -> NewNode {
    NewNode {
        kind: entity_kind.to_owned(),
        title,
        body,
        body_html: String::new(),
        properties: Properties(props),
    }
}

/// Counts of what a registry ontology build produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OntologyReport {
    /// Number of `node-kind` entities written.
    pub node_kinds: usize,
    /// Number of distinct `category` entities.
    pub categories: usize,
    /// Number of `port` entities.
    pub ports: usize,
    /// Number of `parameter` entities.
    pub parameters: usize,
    /// Number of `compatible_with` edges (output → input, same signal type).
    pub compatibility_edges: usize,
}

impl Library {
    /// Populates the store with the structural ontology derived from `registry`: a
    /// `node-kind` entity per kind, linked to its category, ports and parameters, plus
    /// `compatible_with` edges between signal-compatible ports. Returns the counts.
    ///
    /// Intended for a fresh store; run once. Re-running would hit duplicate titles.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn build_registry_ontology(&self, registry: &Registry) -> Result<OntologyReport> {
        let mut report = OntologyReport::default();
        let mut categories: HashMap<String, u64> = HashMap::new();
        // (port node id, signal type string) collected across all kinds.
        let mut outputs: Vec<(u64, String)> = Vec::new();
        let mut inputs: Vec<(u64, String)> = Vec::new();

        for k in registry.kinds() {
            let desc = registry.get(k).expect("kind from the registry exists");

            let nk = self.db().create_node(kg_node(
                kind::NODE_KIND,
                nodekind_title(k),
                k.to_owned(),
                HashMap::new(),
            ))?;
            report.node_kinds += 1;

            // Category (get-or-create; shared across kinds), then in_category.
            let cat = desc.category();
            let cat_id = match categories.get(cat) {
                Some(id) => *id,
                None => {
                    let node = self.db().create_node(kg_node(
                        kind::CATEGORY,
                        category_title(cat),
                        cat.to_owned(),
                        HashMap::new(),
                    ))?;
                    categories.insert(cat.to_owned(), node.id);
                    report.categories += 1;
                    node.id
                }
            };
            self.db()
                .create_edge(new_edge(nk.id, cat_id, rel::IN_CATEGORY))?;

            // Ports (has_port); remember each for the compatibility pass.
            let graph_node = desc.graph_node();
            for port in &graph_node.inputs {
                let signal = format!("{:?}", port.signal);
                let node = self.db().create_node(kg_node(
                    kind::PORT,
                    port_title(k, "in", &port.name),
                    port.name.clone(),
                    HashMap::from([
                        ("signal".to_owned(), serde_json::json!(signal)),
                        ("direction".to_owned(), serde_json::json!("input")),
                    ]),
                ))?;
                self.db()
                    .create_edge(new_edge(nk.id, node.id, rel::HAS_PORT))?;
                inputs.push((node.id, signal));
                report.ports += 1;
            }
            for port in &graph_node.outputs {
                let signal = format!("{:?}", port.signal);
                let node = self.db().create_node(kg_node(
                    kind::PORT,
                    port_title(k, "out", &port.name),
                    port.name.clone(),
                    HashMap::from([
                        ("signal".to_owned(), serde_json::json!(signal)),
                        ("direction".to_owned(), serde_json::json!("output")),
                    ]),
                ))?;
                self.db()
                    .create_edge(new_edge(nk.id, node.id, rel::HAS_PORT))?;
                outputs.push((node.id, signal));
                report.ports += 1;
            }

            // Parameters (has_parameter).
            for param in desc.params() {
                let node = self.db().create_node(kg_node(
                    kind::PARAMETER,
                    param_title(k, &param.name),
                    param.name.clone(),
                    HashMap::from([
                        ("min".to_owned(), serde_json::json!(param.min)),
                        ("max".to_owned(), serde_json::json!(param.max)),
                        ("default".to_owned(), serde_json::json!(param.default)),
                    ]),
                ))?;
                self.db()
                    .create_edge(new_edge(nk.id, node.id, rel::HAS_PARAMETER))?;
                report.parameters += 1;
            }
        }

        // Compatibility: an output port connects to any input port of the same signal type.
        for (out_id, out_signal) in &outputs {
            for (in_id, in_signal) in &inputs {
                if out_signal == in_signal {
                    self.db()
                        .create_edge(new_edge(*out_id, *in_id, rel::COMPATIBLE_WITH))?;
                    report.compatibility_edges += 1;
                }
            }
        }

        Ok(report)
    }

    /// The `node-kind` entity for a kind name (e.g. `"oscillator"`), if the ontology holds
    /// it.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn node_kind_entity(&self, kind_name: &str) -> Result<Option<Node>> {
        self.db().get_node_by_title(&nodekind_title(kind_name))
    }

    /// The ports of a node-kind entity (`node-kind --has_port--> port`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn ports_of_entity(&self, node_kind_id: u64) -> Result<Vec<Node>> {
        self.db()
            .neighbors(node_kind_id, Direction::Outgoing, Some(rel::HAS_PORT))
    }

    /// The category of a node-kind entity, if any.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn category_of_entity(&self, node_kind_id: u64) -> Result<Option<Node>> {
        Ok(self
            .db()
            .neighbors(node_kind_id, Direction::Outgoing, Some(rel::IN_CATEGORY))?
            .into_iter()
            .next())
    }

    /// The input ports an output port may connect to (`output --compatible_with--> input`).
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn compatible_inputs(&self, output_port_id: u64) -> Result<Vec<Node>> {
        self.db().neighbors(
            output_port_id,
            Direction::Outgoing,
            Some(rel::COMPATIBLE_WITH),
        )
    }

    /// Adds an authored concept the assistant can cite.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn add_concept(&self, name: &str, body: &str) -> Result<Node> {
        self.db().create_node(kg_node(
            kind::CONCEPT,
            name.to_owned(),
            body.to_owned(),
            HashMap::new(),
        ))
    }

    /// Adds an authored heuristic ("when X, prefer Y").
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn add_heuristic(&self, name: &str, body: &str) -> Result<Node> {
        self.db().create_node(kg_node(
            kind::HEURISTIC,
            name.to_owned(),
            body.to_owned(),
            HashMap::new(),
        ))
    }

    /// Links an authored concept/heuristic to the node-kind it explains.
    ///
    /// # Errors
    /// Propagates any drevo error.
    pub fn link_explains(&self, from_id: u64, node_kind_id: u64) -> Result<Edge> {
        self.db()
            .create_edge(new_edge(from_id, node_kind_id, rel::EXPLAINS))
    }
}
