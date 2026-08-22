//! A compact, line-oriented text rendering of a patch for LLM context — and a parser
//! back to a [`ProjectDoc`].
//!
//! The AI assistant (epic 12) needs the graph in the model's context window, both to
//! explain a patch and to let the model emit edits. RON round-trips perfectly but is
//! punctuation-heavy and spends tokens on structure; this format is deliberately terse —
//! one statement per line, port types inline — so a whole graph is cheap to include and
//! easy for a model to read and produce. It still round-trips exactly: `to_prompt` then
//! [`ProjectDoc::from_prompt`] reconstructs the same document.
//!
//! Grammar (blank lines and `#` comments ignored):
//!
//! ```text
//! node <id> <kind> in=<name:type,...> out=<name:type,...>
//! edge <from_id>.<port> -> <to_id>.<port>
//! ```
//!
//! where `type` is `audio` | `control` | `event`, and edge endpoints name a port rather
//! than an index so the text stays legible.

use vb_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};

use crate::doc::{NodeEntry, ProjectDoc, FORMAT_VERSION};
use crate::error::ProjectError;

/// First line of every rendering; also the version marker.
const HEADER: &str = "# VST_bomb patch prompt v1";

/// Short tag for a signal type used in the prompt text.
fn type_tag(signal: SignalType) -> &'static str {
    match signal {
        SignalType::Audio => "audio",
        SignalType::Control => "control",
        SignalType::Event => "event",
    }
}

/// Parses a signal-type tag back, or `None` if unrecognized.
fn parse_tag(tag: &str) -> Option<SignalType> {
    match tag {
        "audio" => Some(SignalType::Audio),
        "control" => Some(SignalType::Control),
        "event" => Some(SignalType::Event),
        _ => None,
    }
}

/// Renders a port list as `name:type,name:type` (empty string for no ports).
fn ports_to_str(ports: &[Port]) -> String {
    ports
        .iter()
        .map(|p| format!("{}:{}", p.name, type_tag(p.signal)))
        .collect::<Vec<_>>()
        .join(",")
}

fn deserialize_err(msg: impl Into<String>) -> ProjectError {
    ProjectError::Deserialize(msg.into())
}

/// Parses a `name:type,...` segment into ports of the given direction.
fn parse_ports(segment: &str, input: bool) -> Result<Vec<Port>, ProjectError> {
    if segment.is_empty() {
        return Ok(Vec::new());
    }
    segment
        .split(',')
        .map(|item| {
            let (name, tag) = item
                .split_once(':')
                .ok_or_else(|| deserialize_err(format!("port '{item}' is missing a :type")))?;
            let signal = parse_tag(tag)
                .ok_or_else(|| deserialize_err(format!("unknown signal type '{tag}'")))?;
            Ok(if input {
                Port::input(name, signal)
            } else {
                Port::output(name, signal)
            })
        })
        .collect()
}

/// Strips a `key=` prefix, erroring if `token` does not start with it.
fn strip_kv<'a>(token: &'a str, key: &str) -> Result<&'a str, ProjectError> {
    token
        .strip_prefix(key)
        .ok_or_else(|| deserialize_err(format!("expected `{key}...`, found '{token}'")))
}

/// Splits an `id.port` endpoint into its node id and port name.
fn split_endpoint(token: &str) -> Result<(NodeId, &str), ProjectError> {
    let (id, port) = token
        .split_once('.')
        .ok_or_else(|| deserialize_err(format!("edge endpoint '{token}' must be <id>.<port>")))?;
    let id = id
        .parse::<u64>()
        .map_err(|_| deserialize_err(format!("edge endpoint '{token}' has a non-numeric id")))?;
    Ok((NodeId(id), port))
}

/// Finds the [`PortId`] of the named port on `id`, looking at inputs or outputs.
fn resolve_port(
    nodes: &[NodeEntry],
    id: NodeId,
    port_name: &str,
    input: bool,
) -> Result<PortId, ProjectError> {
    let node = nodes
        .iter()
        .find(|n| n.id == id)
        .ok_or_else(|| deserialize_err(format!("edge references unknown node {}", id.0)))?;
    let ports = if input {
        &node.node.inputs
    } else {
        &node.node.outputs
    };
    ports
        .iter()
        .position(|p| p.name == port_name)
        .map(PortId)
        .ok_or_else(|| {
            let side = if input { "input" } else { "output" };
            deserialize_err(format!("node {} has no {side} port '{port_name}'", id.0))
        })
}

impl ProjectDoc {
    /// Renders this document as compact prompt text (see the [module docs](self)).
    #[must_use]
    pub fn to_prompt(&self) -> String {
        let mut out = String::from(HEADER);
        out.push('\n');
        for entry in &self.nodes {
            out.push_str(&format!(
                "node {} {} in={} out={}\n",
                entry.id.0,
                entry.node.kind,
                ports_to_str(&entry.node.inputs),
                ports_to_str(&entry.node.outputs),
            ));
        }
        for edge in &self.edges {
            // Endpoints name a port; a dangling id/index would have failed validation
            // before serialization, so a miss here falls back to the raw index.
            let from = self.port_label(edge.from_node, edge.from_port, false);
            let to = self.port_label(edge.to_node, edge.to_port, true);
            out.push_str(&format!(
                "edge {}.{} -> {}.{}\n",
                edge.from_node.0, from, edge.to_node.0, to
            ));
        }
        out
    }

    /// The name of a port on `id`, or the raw index as a string if it cannot be found.
    fn port_label(&self, id: NodeId, port: PortId, input: bool) -> String {
        self.node(id)
            .and_then(|n| {
                let ports = if input { &n.inputs } else { &n.outputs };
                ports.get(port.0).map(|p| p.name.clone())
            })
            .unwrap_or_else(|| port.0.to_string())
    }

    /// Parses prompt text back into a document.
    ///
    /// # Errors
    /// [`ProjectError::Deserialize`] for any malformed statement, unknown signal type,
    /// or an edge naming a node/port that is not present.
    pub fn from_prompt(text: &str) -> Result<Self, ProjectError> {
        let mut nodes: Vec<NodeEntry> = Vec::new();
        let mut edge_lines: Vec<String> = Vec::new();

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut tokens = line.split_whitespace();
            match tokens.next() {
                Some("node") => nodes.push(parse_node_line(&mut tokens)?),
                Some("edge") => edge_lines.push(line.to_owned()),
                Some(other) => return Err(deserialize_err(format!("unknown statement '{other}'"))),
                None => unreachable!("split_whitespace yielded no first token on a non-empty line"),
            }
        }

        let mut edges = Vec::with_capacity(edge_lines.len());
        for line in &edge_lines {
            edges.push(parse_edge_line(line, &nodes)?);
        }

        Ok(ProjectDoc {
            version: FORMAT_VERSION,
            nodes,
            edges,
        })
    }
}

/// Parses the tokens after `node`: `<id> <kind> in=... out=...`.
fn parse_node_line<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<NodeEntry, ProjectError> {
    let id = tokens
        .next()
        .ok_or_else(|| deserialize_err("node statement is missing an id"))?
        .parse::<u64>()
        .map_err(|_| deserialize_err("node id must be numeric"))?;
    let kind = tokens
        .next()
        .ok_or_else(|| deserialize_err("node statement is missing a kind"))?;
    let in_seg = strip_kv(
        tokens
            .next()
            .ok_or_else(|| deserialize_err("node statement is missing in=..."))?,
        "in=",
    )?;
    let out_seg = strip_kv(
        tokens
            .next()
            .ok_or_else(|| deserialize_err("node statement is missing out=..."))?,
        "out=",
    )?;
    let node = GraphNode::new(
        kind,
        parse_ports(in_seg, true)?,
        parse_ports(out_seg, false)?,
    );
    Ok(NodeEntry::new(NodeId(id), node))
}

/// Parses an `edge <from>.<port> -> <to>.<port>` line against the parsed nodes.
fn parse_edge_line(line: &str, nodes: &[NodeEntry]) -> Result<Edge, ProjectError> {
    let mut tokens = line.split_whitespace();
    tokens.next(); // "edge"
    let from = tokens
        .next()
        .ok_or_else(|| deserialize_err("edge is missing its source endpoint"))?;
    let arrow = tokens
        .next()
        .ok_or_else(|| deserialize_err("edge is missing '->'"))?;
    if arrow != "->" {
        return Err(deserialize_err(format!("expected '->', found '{arrow}'")));
    }
    let to = tokens
        .next()
        .ok_or_else(|| deserialize_err("edge is missing its destination endpoint"))?;

    let (from_node, from_port) = split_endpoint(from)?;
    let (to_node, to_port) = split_endpoint(to)?;
    Ok(Edge {
        from_node,
        from_port: resolve_port(nodes, from_node, from_port, false)?,
        to_node,
        to_port: resolve_port(nodes, to_node, to_port, true)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_core::Graph;

    /// osc -> biquad(in, cutoff) -> gain, the shape the editor seeds.
    fn sample_doc() -> ProjectDoc {
        let mut g = Graph::new();
        let osc = g.add_node(GraphNode::new(
            "oscillator",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let flt = g.add_node(GraphNode::new(
            "biquad",
            vec![
                Port::input("in", SignalType::Audio),
                Port::input("cutoff", SignalType::Control),
            ],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let amp = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(osc, PortId(0), flt, PortId(0)).unwrap();
        g.connect(flt, PortId(0), amp, PortId(0)).unwrap();
        ProjectDoc::from_graph(&g)
    }

    #[test]
    fn prompt_is_compact_and_readable() {
        let text = sample_doc().to_prompt();
        assert!(text.starts_with(HEADER));
        assert!(text.contains("node 0 oscillator in= out=out:audio"));
        assert!(text.contains("node 1 biquad in=in:audio,cutoff:control out=out:audio"));
        // Edges name ports, not indices.
        assert!(text.contains("edge 0.out -> 1.in"));
        assert!(text.contains("edge 1.out -> 2.in"));
    }

    #[test]
    fn round_trips_through_prompt_text() {
        let doc = sample_doc();
        let back = ProjectDoc::from_prompt(&doc.to_prompt()).expect("parse");
        assert_eq!(back, doc, "structure, ids, ports and edges survive");
    }

    #[test]
    fn an_empty_graph_round_trips() {
        let doc = ProjectDoc::from_graph(&Graph::new());
        let text = doc.to_prompt();
        assert_eq!(text.trim(), HEADER, "just the header");
        assert_eq!(ProjectDoc::from_prompt(&text).unwrap(), doc);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let text = "# a comment\n\nnode 0 gain in=in:audio out=out:audio\n\n# trailing\n";
        let doc = ProjectDoc::from_prompt(text).unwrap();
        assert_eq!(doc.nodes.len(), 1);
        assert_eq!(doc.nodes[0].node.kind, "gain");
        assert!(doc.edges.is_empty());
    }

    #[test]
    fn malformed_statements_are_rejected() {
        // Unknown statement keyword.
        assert!(ProjectDoc::from_prompt("wobble 0 gain").is_err());
        // Bad signal type.
        assert!(ProjectDoc::from_prompt("node 0 gain in=in:laser out=").is_err());
        // Missing in=/out=.
        assert!(ProjectDoc::from_prompt("node 0 gain").is_err());
        // Edge to a port that does not exist.
        let text = "node 0 gain in=in:audio out=out:audio\nedge 0.out -> 0.nope";
        assert!(matches!(
            ProjectDoc::from_prompt(text),
            Err(ProjectError::Deserialize(_))
        ));
        // Edge to a missing node.
        let text = "node 0 gain in=in:audio out=out:audio\nedge 0.out -> 9.in";
        assert!(ProjectDoc::from_prompt(text).is_err());
    }

    #[test]
    fn parsed_prompt_rebuilds_a_runnable_graph() {
        // The parsed doc converts to a real graph with the same topology.
        let doc = ProjectDoc::from_prompt(&sample_doc().to_prompt()).unwrap();
        let graph = doc.to_graph().expect("valid graph");
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edges().len(), 2);
    }
}
