//! Context compression: keep the assistant's context small as a patch grows and a
//! conversation runs long.
//!
//! Two deterministic tools, no model call:
//! - [`graph_summary`] renders a patch as a one-line structural digest (kind counts,
//!   edges, signal types) — a cheap stand-in for the full prompt form when the graph is
//!   large or only its shape matters.
//! - [`distill_history`] trims a long message list to the opening goal plus the most
//!   recent turns, collapsing the omitted middle into a single marker, so token use per
//!   exchange stays bounded instead of growing without limit.

use std::collections::BTreeMap;

use seclab_core::SignalType;
use seclab_project::ProjectDoc;

use crate::provider::Message;

/// The tag for a signal type, matching the prompt/reference vocabulary.
fn signal_tag(signal: SignalType) -> &'static str {
    match signal {
        SignalType::Audio => "audio",
        SignalType::Control => "control",
        SignalType::Event => "event",
    }
}

/// A compact one-line structural digest of `doc`: node kinds with counts, the edge
/// count, and the distinct signal types its ports carry.
///
/// Example: `3 nodes (biquad, gain, oscillator), 2 edges; signals: audio, control`.
#[must_use]
pub fn graph_summary(doc: &ProjectDoc) -> String {
    if doc.nodes.is_empty() {
        return "empty patch (0 nodes)".to_owned();
    }

    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut signals: BTreeMap<&'static str, ()> = BTreeMap::new();
    for entry in &doc.nodes {
        *kinds.entry(entry.node.kind.as_str()).or_insert(0) += 1;
        for port in entry.node.inputs.iter().chain(&entry.node.outputs) {
            signals.insert(signal_tag(port.signal), ());
        }
    }

    let kind_list = kinds
        .iter()
        .map(|(kind, n)| {
            if *n > 1 {
                format!("{kind}×{n}")
            } else {
                (*kind).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let signal_list = signals.keys().copied().collect::<Vec<_>>().join(", ");

    format!(
        "{} nodes ({kind_list}), {} edges; signals: {signal_list}",
        doc.nodes.len(),
        doc.edges.len()
    )
}

/// Trims `messages` to the first turn (the goal) plus the most recent `keep_recent`,
/// replacing any omitted middle with a single marker turn so the total stays bounded.
///
/// A history already at or under `keep_recent + 1` turns is returned unchanged.
#[must_use]
pub fn distill_history(messages: &[Message], keep_recent: usize) -> Vec<Message> {
    if messages.len() <= keep_recent + 1 {
        return messages.to_vec();
    }
    let omitted = messages.len() - 1 - keep_recent;
    let mut out = Vec::with_capacity(keep_recent + 2);
    out.push(messages[0].clone());
    out.push(Message::user(format!(
        "[{omitted} earlier turn(s) omitted to save context]"
    )));
    out.extend_from_slice(&messages[messages.len() - keep_recent..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use seclab_core::{Graph, GraphNode, Port, PortId, SignalType};
    use seclab_project::ProjectDoc;

    fn osc_biquad_gain() -> ProjectDoc {
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
    fn graph_summary_digests_kinds_edges_and_signals() {
        let summary = graph_summary(&osc_biquad_gain());
        assert_eq!(
            summary,
            "3 nodes (biquad, gain, oscillator), 2 edges; signals: audio, control"
        );
        // It is far shorter than the full prompt form.
        assert!(summary.len() < osc_biquad_gain().to_prompt().len());
    }

    #[test]
    fn graph_summary_counts_repeated_kinds() {
        let mut g = Graph::new();
        let a = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let b = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(a, PortId(0), b, PortId(0)).unwrap();
        assert_eq!(
            graph_summary(&ProjectDoc::from_graph(&g)),
            "2 nodes (gain×2), 1 edges; signals: audio"
        );
    }

    #[test]
    fn graph_summary_handles_an_empty_patch() {
        assert_eq!(
            graph_summary(&ProjectDoc::from_graph(&Graph::new())),
            "empty patch (0 nodes)"
        );
    }

    #[test]
    fn distill_keeps_the_goal_and_recent_turns() {
        // goal + 5 turns, keep the most recent 2.
        let messages = vec![
            Message::user("design a bass"),
            Message::assistant("a1"),
            Message::user("u2"),
            Message::assistant("a3"),
            Message::user("u4"),
            Message::assistant("a5"),
        ];
        let distilled = distill_history(&messages, 2);
        // goal + marker + last 2 = 4.
        assert_eq!(distilled.len(), 4);
        assert_eq!(distilled[0].content, "design a bass", "goal kept");
        assert!(distilled[1].content.contains("3 earlier turn(s) omitted"));
        assert_eq!(distilled[2].content, "u4");
        assert_eq!(distilled[3].content, "a5");
    }

    #[test]
    fn distill_leaves_a_short_history_untouched() {
        let messages = vec![
            Message::user("goal"),
            Message::assistant("a1"),
            Message::user("u2"),
        ];
        // 3 turns, keep_recent 2 → 2 + 1 = 3 ≥ len → unchanged.
        assert_eq!(distill_history(&messages, 2), messages);
    }
}
