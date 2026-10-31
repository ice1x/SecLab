//! The "repair an existing patch" skill: feed a broken patch's validator issues to the
//! assistant and get a corrected, re-validated patch back.
//!
//! This is the mirror of [`crate::generate::generate_patch`] pointed at a patch that is
//! already invalid. It gathers exactly what the validator complains about — unknown node
//! kinds, port/signal problems, double-driven inputs, cycles — and hands those to the
//! model as the repair brief, then runs the reply through the same mandatory validation.
//! The result carries the diff (so the fix can be previewed and applied as an undoable
//! command) and the list of issues that prompted it.

use seclab_nodes::Registry;
use seclab_project::ProjectDoc;

use crate::diff::PatchDiff;
use crate::generate::{generate_patch, patch_issues, GenerateError};
use crate::provider::{AiProvider, ProviderConfig};

/// A successful repair: the corrected patch, what changed, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct RepairOutcome {
    /// The validated, corrected document.
    pub doc: ProjectDoc,
    /// The change from the broken patch — for preview and undoable apply.
    pub diff: PatchDiff,
    /// The validator issues that prompted the repair.
    pub fixed_issues: Vec<String>,
    /// The assistant's plain-language explanation.
    pub explanation: String,
    /// Model round-trips it took.
    pub attempts: u32,
}

/// Why a repair could not be produced.
#[derive(Debug, Clone, PartialEq)]
pub enum RepairError {
    /// The patch is already valid — there is nothing to repair (use explain instead).
    AlreadyValid,
    /// The underlying generation loop failed (provider error or no valid result).
    Generate(GenerateError),
}

impl std::fmt::Display for RepairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepairError::AlreadyValid => write!(f, "the patch is already valid"),
            RepairError::Generate(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RepairError {}

/// Repairs `broken`, driving the assistant with the validator's own issue list.
///
/// Returns [`RepairError::AlreadyValid`] (without calling the provider) if the patch has
/// no issues. Otherwise it frames the issues as the repair brief, runs the grounded,
/// mandatory-validation loop (so the fix is itself validated), and returns the corrected
/// patch with a [`PatchDiff`] from the original.
///
/// # Errors
/// [`RepairError::AlreadyValid`] or [`RepairError::Generate`].
pub fn repair_patch(
    provider: &dyn AiProvider,
    registry: &Registry,
    config: &ProviderConfig,
    broken: &ProjectDoc,
    max_attempts: u32,
) -> Result<RepairOutcome, RepairError> {
    let issues = patch_issues(broken, registry);
    if issues.is_empty() {
        return Err(RepairError::AlreadyValid);
    }

    let listed = issues
        .iter()
        .map(|i| format!("- {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let brief = format!(
        "This patch is invalid. The validator reports:\n{listed}\n\nRepair it while preserving \
         the intended sound. Keep existing node ids stable where you can, and output the FULL \
         corrected ProjectDoc."
    );

    let out = generate_patch(
        provider,
        registry,
        config,
        &brief,
        Some(broken),
        max_attempts,
    )
    .map_err(RepairError::Generate)?;
    let diff = PatchDiff::between(broken, &out.doc);
    Ok(RepairOutcome {
        doc: out.doc,
        diff,
        fixed_issues: issues,
        explanation: out.explanation,
        attempts: out.attempts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ScriptedProvider;
    use seclab_core::{Edge, GraphNode, NodeId, Port, PortId, SignalType};
    use seclab_project::{NodeEntry, ProjectDoc, FORMAT_VERSION};

    fn config() -> ProviderConfig {
        ProviderConfig::default()
    }

    fn reply_with(ron: &str) -> String {
        format!("Fixed the wiring.\n\n```ron\n{ron}\n```\n")
    }

    /// A broken patch: an audio output wired into a control input (signal mismatch).
    fn broken() -> ProjectDoc {
        ProjectDoc {
            version: FORMAT_VERSION,
            nodes: vec![
                NodeEntry::new(
                    NodeId(0),
                    GraphNode::new(
                        "oscillator",
                        vec![],
                        vec![Port::output("out", SignalType::Audio)],
                    ),
                ),
                NodeEntry::new(
                    NodeId(1),
                    GraphNode::new(
                        "adsr",
                        vec![Port::input("gate", SignalType::Control)],
                        vec![Port::output("env", SignalType::Control)],
                    ),
                ),
            ],
            edges: vec![Edge {
                from_node: NodeId(0),
                from_port: PortId(0),
                to_node: NodeId(1),
                to_port: PortId(0),
            }],
        }
    }

    /// A valid osc → gain repair the model returns.
    fn repaired_ron() -> String {
        let mut g = seclab_core::Graph::new();
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
        seclab_project::to_ron(&ProjectDoc::from_graph(&g)).unwrap()
    }

    #[test]
    fn repair_feeds_the_validator_issues_to_the_model_and_returns_a_fix() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&[&reply_with(&repaired_ron())]);

        let out = repair_patch(&provider, &registry, &config(), &broken(), 3).expect("repaired");
        assert_eq!(out.attempts, 1);
        assert!(out.doc.is_valid(), "the repair is valid");
        assert!(
            out.fixed_issues
                .iter()
                .any(|i| i.contains("incompatible signal")),
            "the signal mismatch is among the fixed issues"
        );
        assert!(!out.diff.is_empty(), "something changed");

        // The brief handed to the model listed the validator's issue.
        let brief = &provider.requests()[0].messages[0].content;
        assert!(brief.contains("The validator reports"));
        assert!(brief.contains("incompatible signal types"));
    }

    #[test]
    fn a_valid_patch_needs_no_repair_and_calls_no_provider() {
        let registry = Registry::with_builtins();
        let provider = ScriptedProvider::replies(&["unused"]);
        // Build a valid osc → gain patch.
        let valid = seclab_project::from_ron(&repaired_ron()).unwrap();
        let err = repair_patch(&provider, &registry, &config(), &valid, 3).unwrap_err();
        assert_eq!(err, RepairError::AlreadyValid);
        assert!(provider.requests().is_empty(), "no wasted call");
    }

    #[test]
    fn a_generation_failure_propagates() {
        let registry = Registry::with_builtins();
        // The model never produces a valid fix within the budget.
        let provider = ScriptedProvider::replies(&["no code block here"]);
        let err = repair_patch(&provider, &registry, &config(), &broken(), 1).unwrap_err();
        assert!(matches!(
            err,
            RepairError::Generate(GenerateError::NotConverged { .. })
        ));
    }
}
