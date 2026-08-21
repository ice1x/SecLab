//! The plugin's in-window editor (backlog 000067).
//!
//! A simplified **run-view**: the window shows which patch the plugin is playing (node
//! counts by kind, wiring) and the current value of every host-automatable parameter. It
//! is read-only for now — a first step toward embedding the full node editor — so a user
//! who loads the plugin in a DAW can see what it is doing instead of a black box.
//!
//! The view-model ([`PatchSummary`]) is a pure function of the project document, always
//! compiled and unit-tested. The egui glue lives behind the `gui` Cargo feature, so the
//! default build ships no editor and a headless clap-validator never opens a window.

use std::collections::BTreeMap;

use vb_project::ProjectDoc;

/// A compact, display-ready summary of the loaded patch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PatchSummary {
    /// Number of nodes in the patch.
    pub node_count: usize,
    /// Number of edges (connections).
    pub edge_count: usize,
    /// Node kinds and how many of each, sorted by kind name.
    pub kinds: Vec<(String, usize)>,
}

/// Summarises `doc` for the run-view: node/edge counts and a per-kind tally.
#[must_use]
pub fn patch_summary(doc: &ProjectDoc) -> PatchSummary {
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &doc.nodes {
        *kinds.entry(entry.node.kind.clone()).or_default() += 1;
    }
    PatchSummary {
        node_count: doc.nodes.len(),
        edge_count: doc.edges.len(),
        kinds: kinds.into_iter().collect(),
    }
}

/// Builds the egui run-view editor for the plugin window.
#[cfg(feature = "gui")]
pub(crate) fn create(
    egui_state: std::sync::Arc<nih_plug_egui::EguiState>,
    doc: std::sync::Arc<std::sync::Mutex<std::sync::Arc<ProjectDoc>>>,
    params: std::sync::Arc<crate::VstBombParams>,
) -> Option<Box<dyn nih_plug::prelude::Editor>> {
    use nih_plug_egui::{create_egui_editor, egui};

    create_egui_editor(
        egui_state,
        (),
        |_ctx, _state| {},
        move |ctx, _setter, _state| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("VST_bomb");

                let summary = match doc.lock() {
                    Ok(guard) => patch_summary(&guard),
                    Err(_) => PatchSummary::default(),
                };
                ui.label(format!(
                    "Patch: {} nodes, {} edges",
                    summary.node_count, summary.edge_count
                ));

                ui.separator();
                ui.label("Nodes:");
                for (kind, count) in &summary.kinds {
                    ui.label(format!("   {count}× {kind}"));
                }

                ui.separator();
                ui.label("Parameters:");
                let overview = params.overview();
                if overview.is_empty() {
                    ui.label("   (none)");
                } else {
                    for (name, value) in overview {
                        ui.label(format!("   {name}: {value:.3}"));
                    }
                }
            });
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vb_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};
    use vb_project::{NodeEntry, FORMAT_VERSION};

    fn node(id: u64, kind: &str) -> NodeEntry {
        NodeEntry {
            id: NodeId(id),
            node: GraphNode::new(
                kind,
                vec![Port::input("in", SignalType::Audio)],
                vec![Port::output("out", SignalType::Audio)],
            ),
        }
    }

    #[test]
    fn summary_counts_nodes_edges_and_kinds() {
        let doc = ProjectDoc {
            version: FORMAT_VERSION,
            nodes: vec![node(0, "oscillator"), node(1, "gain"), node(2, "gain")],
            edges: vec![
                Edge {
                    from_node: NodeId(0),
                    from_port: PortId(0),
                    to_node: NodeId(1),
                    to_port: PortId(0),
                },
                Edge {
                    from_node: NodeId(1),
                    from_port: PortId(0),
                    to_node: NodeId(2),
                    to_port: PortId(0),
                },
            ],
        };
        let summary = patch_summary(&doc);
        assert_eq!(summary.node_count, 3);
        assert_eq!(summary.edge_count, 2);
        // Sorted by kind, gain tallied to 2.
        assert_eq!(
            summary.kinds,
            vec![("gain".to_owned(), 2), ("oscillator".to_owned(), 1)]
        );
    }

    #[test]
    fn an_empty_patch_summarises_to_zeroes() {
        let doc = ProjectDoc {
            version: FORMAT_VERSION,
            nodes: vec![],
            edges: vec![],
        };
        let summary = patch_summary(&doc);
        assert_eq!(summary, PatchSummary::default());
    }
}
