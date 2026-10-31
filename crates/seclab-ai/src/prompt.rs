//! Assembling the assistant's system prompt from versioned assets plus live context.
//!
//! The static pieces — the role/contract/refusal pre-prompt and a concrete valid
//! example — are versioned files under `assets/prompts/`, embedded at build time. The
//! live pieces — which nodes exist, and the patch being edited — are generated from the
//! registry and the project so they can never go stale.

use seclab_nodes::{node_reference, Registry};
use seclab_project::ProjectDoc;

/// Version of the prompt asset set. Bump when the pre-prompt or contract changes.
///
/// v2 adds the "Designing the interface" contract: a node entry may carry `values`,
/// `widgets`, `panel_pos`, `panel_color`, and `panel_hidden`, so the assistant authors
/// the control panel, not just the signal graph.
pub const PROMPT_VERSION: u32 = 2;

/// The versioned system pre-prompt: role, grounding rules, output contract (graph **and**
/// interface), refusals.
pub const SYSTEM_PREAMBLE: &str = include_str!("../assets/prompts/system_v2.md");

/// A concrete, valid `ProjectDoc` in RON — the shape the assistant must emit. Embedded
/// so the example shown to the model is verified (a test parses it) rather than a
/// hand-typed snippet that can rot.
pub const EXAMPLE_PATCH_RON: &str = include_str!("../assets/prompts/example_patch.ron");

/// Section heading for the injected node reference.
const NODES_HEADING: &str = "# Available nodes";
/// Section heading for the injected current patch.
const PATCH_HEADING: &str = "# Current patch";
/// Section heading for the injected example output.
const EXAMPLE_HEADING: &str = "# Example output (RON ProjectDoc)";
/// Shown in place of a patch when starting from a blank canvas.
const EMPTY_PATCH: &str = "(empty — starting from a blank canvas)";

/// Builds the full system prompt: the versioned pre-prompt, then the live node
/// reference, the example output, and the current patch (or an empty-canvas marker).
///
/// `patch` is the document being edited, or `None` when starting fresh.
#[must_use]
pub fn build_system_prompt(registry: &Registry, patch: Option<&ProjectDoc>) -> String {
    let patch_text = patch.map_or_else(|| EMPTY_PATCH.to_owned(), ProjectDoc::to_prompt);
    format!(
        "{preamble}\n\n{nodes_h}\n\n{nodes}\n{example_h}\n\n```ron\n{example}```\n\n{patch_h}\n\n{patch}\n",
        preamble = SYSTEM_PREAMBLE.trim_end(),
        nodes_h = NODES_HEADING,
        nodes = node_reference(registry),
        example_h = EXAMPLE_HEADING,
        example = EXAMPLE_PATCH_RON,
        patch_h = PATCH_HEADING,
        patch = patch_text.trim_end(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use seclab_core::{Graph, GraphNode, Port, PortId, SignalType};

    fn sample_patch() -> ProjectDoc {
        let mut g = Graph::new();
        let osc = g.add_node(GraphNode::new(
            "oscillator",
            vec![],
            vec![Port::output("out", SignalType::Audio)],
        ));
        let amp = g.add_node(GraphNode::new(
            "gain",
            vec![Port::input("in", SignalType::Audio)],
            vec![Port::output("out", SignalType::Audio)],
        ));
        g.connect(osc, PortId(0), amp, PortId(0)).unwrap();
        ProjectDoc::from_graph(&g)
    }

    #[test]
    fn the_preamble_states_role_contract_and_refusals() {
        let p = SYSTEM_PREAMBLE;
        assert!(!p.trim().is_empty());
        assert!(p.contains("SecLab patch assistant"), "role");
        assert!(p.contains("ProjectDoc"), "output contract names the doc");
        assert!(p.to_lowercase().contains("refus"), "refusal rules");
        assert!(
            p.contains("only") && p.to_lowercase().contains("node reference"),
            "grounding rule references the node list"
        );
        // v2: the contract also covers the control panel, so the model authors the interface.
        assert!(
            p.contains("Designing the interface"),
            "interface section present"
        );
        for field in [
            "values",
            "widgets",
            "panel_pos",
            "panel_color",
            "panel_hidden",
        ] {
            assert!(p.contains(field), "interface field '{field}' documented");
        }
        for widget in [
            "\"knob\"",
            "\"slider\"",
            "\"fader\"",
            "\"switch\"",
            "\"xy-pad\"",
        ] {
            assert!(p.contains(widget), "valid widget label {widget} listed");
        }
    }

    #[test]
    fn the_embedded_example_is_a_valid_project_doc() {
        // The example the model is told to mirror must actually parse and rebuild a graph
        // — otherwise the contract drifts from what the app accepts.
        let doc = seclab_project::from_ron(EXAMPLE_PATCH_RON).expect("example parses");
        let graph = doc.to_graph().expect("example builds a graph");
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edges().len(), 1);
    }

    #[test]
    fn the_embedded_example_demonstrates_the_interface_contract() {
        // v2: the example must actually SHOW the panel fields, or the model has nothing to
        // mirror for the interface half of the contract.
        let doc = seclab_project::from_ron(EXAMPLE_PATCH_RON).expect("example parses");
        assert!(
            doc.nodes.iter().all(|n| !n.values.is_empty()),
            "every node sets its parameter values"
        );
        assert!(
            doc.nodes.iter().all(|n| !n.widgets.is_empty()),
            "every node assigns widgets"
        );
        assert!(
            doc.nodes.iter().any(|n| n.panel_pos.is_some()),
            "at least one module is placed on the panel"
        );
        assert!(
            doc.nodes.iter().any(|n| n.panel_color.is_some()),
            "the example demonstrates a panel accent colour"
        );
        // The widget labels in the example are the ones the contract declares valid.
        let valid = ["knob", "slider", "fader", "switch", "xy-pad"];
        assert!(
            doc.nodes
                .iter()
                .flat_map(|n| &n.widgets)
                .all(|w| valid.contains(&w.as_str())),
            "example widgets use only valid labels"
        );
    }

    #[test]
    fn the_assembled_prompt_injects_live_nodes_and_the_patch() {
        let registry = Registry::with_builtins();
        let prompt = build_system_prompt(&registry, Some(&sample_patch()));

        // Pre-prompt first, then the live node reference.
        assert!(prompt.starts_with("# SecLab patch assistant"));
        assert!(prompt.contains(NODES_HEADING));
        assert!(
            prompt.contains("oscillator [generator]"),
            "the live node reference is injected"
        );
        // The example and the current patch are both present.
        assert!(prompt.contains(EXAMPLE_HEADING));
        assert!(
            prompt.contains("node 0 oscillator"),
            "current patch in prompt form"
        );
        assert!(prompt.contains("edge 0.out -> 1.in"));
    }

    #[test]
    fn a_blank_canvas_is_marked_not_omitted() {
        let registry = Registry::with_builtins();
        let prompt = build_system_prompt(&registry, None);
        assert!(prompt.contains(PATCH_HEADING));
        assert!(prompt.contains(EMPTY_PATCH), "empty patch is explicit");
        // The node reference is still there so the model knows the palette.
        assert!(prompt.contains("gain [math]"));
    }
}
