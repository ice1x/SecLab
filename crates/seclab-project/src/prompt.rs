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
//! node <id> <kind> in=<name:type,...> out=<name:type,...> [values=..] [widgets=..] [pos=x,y] [color=#rrggbb] [hidden=..]
//! edge <from_id>.<port> -> <to_id>.<port>
//! ```
//!
//! where `type` is `audio` | `control` | `event`, and edge endpoints name a port rather
//! than an index so the text stays legible.
//!
//! A node line may carry the **presentation** fields (the control panel the assistant
//! authors), each emitted only when it departs from the default so a structure-only patch
//! renders exactly as before:
//!
//! - `values=<f32>,<f32>,...` — parameter values in registry `params()` order;
//! - `widgets=<label>,<label>,...` — per-parameter widget label (see
//!   [`crate::VALID_WIDGET_LABELS`]);
//! - `pos=<x>,<y>` — the module's panel position;
//! - `color=#rrggbb` — the module's panel accent colour;
//! - `hidden=<idx>,<idx>,...` — parameter indices hidden from the panel.
//!
//! These round-trip through [`ProjectDoc::from_prompt`] alongside the structure, so an
//! edit shown to the model as prompt text keeps the authored interface instead of
//! silently dropping it.

use seclab_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};

use crate::doc::{NodeEntry, ProjectDoc, FORMAT_VERSION};
use crate::error::ProjectError;

/// First line of every rendering; also the version marker. v2 adds the optional
/// presentation fields (`values`/`widgets`/`pos`/`color`/`hidden`) to a node line.
const HEADER: &str = "# SecLab patch prompt v2";

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

/// Joins floats with commas via their shortest round-trippable `Display` form.
fn floats_to_str(values: &[f32]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Joins indices with commas.
fn usizes_to_str(values: &[usize]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Parses a comma-separated float list (empty string → empty vec).
fn parse_floats(segment: &str) -> Result<Vec<f32>, ProjectError> {
    if segment.is_empty() {
        return Ok(Vec::new());
    }
    segment
        .split(',')
        .map(|p| {
            p.parse::<f32>()
                .map_err(|_| deserialize_err(format!("'{p}' is not a number")))
        })
        .collect()
}

/// Parses a comma-separated index list (empty string → empty vec).
fn parse_usizes(segment: &str) -> Result<Vec<usize>, ProjectError> {
    if segment.is_empty() {
        return Ok(Vec::new());
    }
    segment
        .split(',')
        .map(|p| {
            p.parse::<usize>()
                .map_err(|_| deserialize_err(format!("'{p}' is not a non-negative index")))
        })
        .collect()
}

/// Parses a `x,y` panel position.
fn parse_pos(segment: &str) -> Result<(f32, f32), ProjectError> {
    let (x, y) = segment
        .split_once(',')
        .ok_or_else(|| deserialize_err(format!("pos '{segment}' must be <x>,<y>")))?;
    let x = x
        .parse::<f32>()
        .map_err(|_| deserialize_err(format!("pos x '{x}' is not a number")))?;
    let y = y
        .parse::<f32>()
        .map_err(|_| deserialize_err(format!("pos y '{y}' is not a number")))?;
    Ok((x, y))
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
            let mut line = format!(
                "node {} {} in={} out={}",
                entry.id.0,
                entry.node.kind,
                ports_to_str(&entry.node.inputs),
                ports_to_str(&entry.node.outputs),
            );
            // Presentation fields — emitted only when non-default, so a structure-only
            // patch renders exactly as the v1 grammar did.
            if !entry.values.is_empty() {
                line.push_str(&format!(" values={}", floats_to_str(&entry.values)));
            }
            if !entry.widgets.is_empty() {
                line.push_str(&format!(" widgets={}", entry.widgets.join(",")));
            }
            if let Some((x, y)) = entry.panel_pos {
                line.push_str(&format!(" pos={x},{y}"));
            }
            if let Some(color) = &entry.panel_color {
                line.push_str(&format!(" color={color}"));
            }
            if !entry.panel_hidden.is_empty() {
                line.push_str(&format!(" hidden={}", usizes_to_str(&entry.panel_hidden)));
            }
            line.push('\n');
            out.push_str(&line);
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
    let mut entry = NodeEntry::new(NodeId(id), node);
    // Remaining tokens are the optional presentation fields, in any order.
    for token in tokens {
        if let Some(v) = token.strip_prefix("values=") {
            entry.values = parse_floats(v)?;
        } else if let Some(v) = token.strip_prefix("widgets=") {
            entry.widgets = if v.is_empty() {
                Vec::new()
            } else {
                v.split(',').map(str::to_owned).collect()
            };
        } else if let Some(v) = token.strip_prefix("pos=") {
            entry.panel_pos = Some(parse_pos(v)?);
        } else if let Some(v) = token.strip_prefix("color=") {
            entry.panel_color = Some(v.to_owned());
        } else if let Some(v) = token.strip_prefix("hidden=") {
            entry.panel_hidden = parse_usizes(v)?;
        } else {
            return Err(deserialize_err(format!("unknown node field '{token}'")));
        }
    }
    Ok(entry)
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
    use seclab_core::Graph;

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

    /// A doc whose nodes carry the full set of presentation fields.
    fn panel_doc() -> ProjectDoc {
        let mut doc = sample_doc();
        // oscillator (id 0): values + widgets + placement.
        doc.nodes[0].values = vec![2.0, 110.0];
        doc.nodes[0].widgets = vec!["switch".to_owned(), "knob".to_owned()];
        doc.nodes[0].panel_pos = Some((0.0, 0.0));
        // biquad (id 1): values, accent colour, a hidden control.
        doc.nodes[1].values = vec![0.0, 1200.0, 0.707];
        doc.nodes[1].panel_color = Some("#88ccff".to_owned());
        doc.nodes[1].panel_hidden = vec![2];
        // gain (id 2): a single fader.
        doc.nodes[2].widgets = vec!["fader".to_owned()];
        doc.nodes[2].values = vec![0.8];
        doc
    }

    #[test]
    fn prompt_emits_the_presentation_fields_when_present() {
        let text = panel_doc().to_prompt();
        assert!(
            text.contains(
                "node 0 oscillator in= out=out:audio values=2,110 widgets=switch,knob pos=0,0"
            ),
            "oscillator line carries values, widgets and placement:\n{text}"
        );
        assert!(
            text.contains("color=#88ccff"),
            "the accent colour is emitted:\n{text}"
        );
        assert!(
            text.contains("hidden=2"),
            "the hidden-control index is emitted:\n{text}"
        );
        assert!(
            text.contains("widgets=fader"),
            "the gain's fader is emitted:\n{text}"
        );
    }

    #[test]
    fn structure_only_nodes_emit_no_presentation_tokens() {
        // A patch nobody has arranged renders exactly as the v1 grammar did — no trailing
        // tokens — so prompt text stays terse when there is no interface to carry.
        let text = sample_doc().to_prompt();
        assert!(text.contains("node 0 oscillator in= out=out:audio\n"));
        assert!(!text.contains("values="));
        assert!(!text.contains("widgets="));
        assert!(!text.contains("pos="));
        assert!(!text.contains("color="));
        assert!(!text.contains("hidden="));
    }

    #[test]
    fn panel_fields_round_trip_through_prompt_text() {
        let doc = panel_doc();
        let back = ProjectDoc::from_prompt(&doc.to_prompt()).expect("parse");
        assert_eq!(
            back, doc,
            "values, widgets, placement, colour and hidden set all survive"
        );
    }

    #[test]
    fn a_malformed_presentation_token_is_rejected() {
        // A non-numeric value.
        let text = "node 0 gain in=in:audio out=out:audio values=loud";
        assert!(ProjectDoc::from_prompt(text).is_err());
        // A bad position.
        let text = "node 0 gain in=in:audio out=out:audio pos=0";
        assert!(ProjectDoc::from_prompt(text).is_err());
        // An unknown field keyword.
        let text = "node 0 gain in=in:audio out=out:audio wibble=1";
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
