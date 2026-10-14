//! NL → patch: turn a description into a validated [`ProjectDoc`].
//!
//! This is the core assistant loop. It assembles the grounded system prompt
//! ([`build_system_prompt`]), asks an [`AiProvider`] for a patch, extracts the RON
//! `ProjectDoc` from the reply, and puts it through **mandatory validation** — every
//! node kind must exist in the registry (grounding) and the graph must pass
//! [`ProjectDoc::validate`] (ports, signal types, single-source inputs, acyclicity).
//!
//! Validation is a hard gate, never a suggestion: an invalid proposal never escapes this
//! function. When it fails, the validator's issues are fed back to the model and it is
//! asked to fix them, up to a bounded number of attempts, so a near-miss converges
//! instead of being rejected outright.

use vb_nodes::Registry;
use vb_project::ProjectDoc;

use crate::converge::{converge, ConvergeError, Converged, Verdict};
use crate::diff::PatchDiff;
use crate::prompt::build_system_prompt;
use crate::provider::{AiError, AiProvider, ProviderConfig};

/// A successfully generated, validated patch.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateOutcome {
    /// The validated document, ready to load onto the canvas.
    pub doc: ProjectDoc,
    /// The assistant's plain-language explanation (the prose before the RON block).
    pub explanation: String,
    /// How many model round-trips it took (1 = first try).
    pub attempts: u32,
}

/// Why generation failed.
#[derive(Debug, Clone, PartialEq)]
pub enum GenerateError {
    /// The provider call itself failed.
    Provider(AiError),
    /// The model never produced a valid patch within the attempt budget; carries the
    /// issues from the final attempt (validation messages, a parse error, or a missing
    /// code block) and how many attempts were spent.
    NotConverged {
        /// Human-readable problems from the last attempt.
        issues: Vec<String>,
        /// Attempts spent.
        attempts: u32,
    },
}

impl std::fmt::Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GenerateError::Provider(e) => write!(f, "provider error: {e}"),
            GenerateError::NotConverged { issues, attempts } => write!(
                f,
                "no valid patch after {attempts} attempt(s); last issues: {}",
                issues.join("; ")
            ),
        }
    }
}

impl std::error::Error for GenerateError {}

/// Generates a validated patch from `description`, retrying with validator feedback.
///
/// `current` is the patch being edited (shown to the model as context), or `None` when
/// starting from a blank canvas. `max_attempts` bounds the model round-trips (>= 1).
///
/// # Errors
/// [`GenerateError::Provider`] if a provider call fails; [`GenerateError::NotConverged`]
/// if no valid patch is produced within `max_attempts`.
pub fn generate_patch(
    provider: &dyn AiProvider,
    registry: &Registry,
    config: &ProviderConfig,
    description: &str,
    current: Option<&ProjectDoc>,
    max_attempts: u32,
) -> Result<GenerateOutcome, GenerateError> {
    let system = build_system_prompt(registry, current);
    // The propose → validate → repair loop is the shared `converge` primitive; the gate
    // here is "extract a RON ProjectDoc and pass mandatory validation".
    let result = converge(
        provider,
        config,
        &system,
        description.to_owned(),
        max_attempts,
        |content| match evaluate_reply(content, registry) {
            Ok(accepted) => Verdict::Accept(accepted),
            Err(issues) => Verdict::Revise(issues),
        },
        retry_prompt,
    );
    match result {
        Ok(Converged {
            value: (doc, explanation),
            attempts,
        }) => Ok(GenerateOutcome {
            doc,
            explanation,
            attempts,
        }),
        Err(ConvergeError::Provider(e)) => Err(GenerateError::Provider(e)),
        Err(ConvergeError::NotConverged { issues, attempts }) => {
            Err(GenerateError::NotConverged { issues, attempts })
        }
    }
}

/// A successfully generated edit: the new patch plus the diff from the current one.
#[derive(Debug, Clone, PartialEq)]
pub struct EditOutcome {
    /// The validated, edited document.
    pub doc: ProjectDoc,
    /// What changed from the current patch — for previewing before applying.
    pub diff: PatchDiff,
    /// The assistant's plain-language explanation.
    pub explanation: String,
    /// Model round-trips it took.
    pub attempts: u32,
}

/// Applies a natural-language edit to `current`, returning the validated new patch and a
/// [`PatchDiff`] to preview before it touches the canvas.
///
/// The model is asked to return the *full* updated `ProjectDoc`, keeping existing node
/// ids stable so a small edit reads as a small diff; the result runs through the same
/// mandatory validation as [`generate_patch`]. Applying the diff as an undoable command
/// is a separate step (000105) — this only proposes and previews.
///
/// # Errors
/// Same as [`generate_patch`]: [`GenerateError::Provider`] or [`GenerateError::NotConverged`].
pub fn edit_patch(
    provider: &dyn AiProvider,
    registry: &Registry,
    config: &ProviderConfig,
    current: &ProjectDoc,
    instruction: &str,
    max_attempts: u32,
) -> Result<EditOutcome, GenerateError> {
    let framed = format!(
        "Edit the current patch per this instruction. Keep existing node ids stable for \
         nodes you do not change, and output the FULL updated ProjectDoc.\n\nInstruction: {instruction}"
    );
    let out = generate_patch(
        provider,
        registry,
        config,
        &framed,
        Some(current),
        max_attempts,
    )?;
    let diff = PatchDiff::between(current, &out.doc);
    Ok(EditOutcome {
        doc: out.doc,
        diff,
        explanation: out.explanation,
        attempts: out.attempts,
    })
}

/// Extracts and validates a reply. `Ok((doc, explanation))` on success, `Err(issues)`
/// with the problems to feed back otherwise.
pub(crate) fn evaluate_reply(
    content: &str,
    registry: &Registry,
) -> Result<(ProjectDoc, String), Vec<String>> {
    let Some((explanation, ron)) = extract_ron_block(content) else {
        return Err(vec![
            "no RON code block found — output the patch in a fenced ```ron block".to_owned(),
        ]);
    };

    let doc = match vb_project::from_ron(&ron) {
        Ok(doc) => doc,
        Err(e) => return Err(vec![format!("the RON did not parse: {e}")]),
    };

    let issues = patch_issues(&doc, registry);
    if issues.is_empty() {
        Ok((doc, explanation))
    } else {
        Err(issues)
    }
}

/// Mandatory validation: unknown node kinds (grounding), every structural problem
/// [`ProjectDoc::validate`] reports, and panel-coherence (the authored interface must
/// match the node's real parameters). Shared with the repair skill (000118).
pub(crate) fn patch_issues(doc: &ProjectDoc, registry: &Registry) -> Vec<String> {
    let mut issues: Vec<String> = doc
        .nodes
        .iter()
        .filter(|entry| registry.get(&entry.node.kind).is_none())
        .map(|entry| {
            format!(
                "node {} uses unknown kind '{}' — use only kinds from the reference",
                entry.id.0, entry.node.kind
            )
        })
        .collect();
    issues.extend(doc.validate().iter().map(ToString::to_string));
    issues.extend(panel_issues(doc, registry));
    issues
}

/// Checks each node's **presentation** fields against the node's real parameters: widget
/// labels must be ones the panel knows, the per-parameter `widgets`/`values` lists must
/// not be longer than the kind's parameter count, every value must sit inside its stated
/// range, hidden-control indices must address a parameter that exists, and an accent
/// colour must be a `#rrggbb` hex string. The system prompt instructs all of this; this is
/// the hard gate that enforces it, so a panel that lies about the graph is fed back for
/// repair instead of reaching the canvas.
///
/// Nodes of an unknown kind are skipped here — the grounding check already reports them,
/// and without a descriptor there is nothing to check the panel against.
fn panel_issues(doc: &ProjectDoc, registry: &Registry) -> Vec<String> {
    let mut issues = Vec::new();
    for entry in &doc.nodes {
        let Some(descriptor) = registry.get(&entry.node.kind) else {
            continue;
        };
        let params = descriptor.params();
        let n = params.len();
        let id = entry.id.0;
        let kind = &entry.node.kind;

        for label in &entry.widgets {
            if !vb_project::VALID_WIDGET_LABELS.contains(&label.as_str()) {
                issues.push(format!(
                    "node {id} ({kind}) uses unknown widget '{label}' — use one of: {}",
                    vb_project::VALID_WIDGET_LABELS.join(", ")
                ));
            }
        }
        if entry.widgets.len() > n {
            issues.push(format!(
                "node {id} ({kind}) lists {} widgets but the kind has {n} parameter(s)",
                entry.widgets.len()
            ));
        }
        if entry.values.len() > n {
            issues.push(format!(
                "node {id} ({kind}) sets {} values but the kind has {n} parameter(s)",
                entry.values.len()
            ));
        }
        for (value, param) in entry.values.iter().zip(&params) {
            if *value < param.min || *value > param.max {
                issues.push(format!(
                    "node {id} ({kind}) parameter '{}' value {value} is outside its range [{}, {}]",
                    param.name, param.min, param.max
                ));
            }
        }
        for idx in &entry.panel_hidden {
            if *idx >= n {
                issues.push(format!(
                    "node {id} ({kind}) hides parameter index {idx} but the kind has only {n} parameter(s)"
                ));
            }
        }
        if let Some(color) = &entry.panel_color {
            if !is_hex_color(color) {
                issues.push(format!(
                    "node {id} ({kind}) panel_color '{color}' is not a #rrggbb hex colour"
                ));
            }
        }
    }
    issues
}

/// Whether `s` is a `#rrggbb` hex colour: a `#` and exactly six hex digits.
fn is_hex_color(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The follow-up message listing the problems to fix.
pub(crate) fn retry_prompt(issues: &[String]) -> String {
    let mut msg =
        String::from("That patch is not valid. Fix these and re-emit the full RON ProjectDoc:\n");
    for issue in issues {
        msg.push_str("- ");
        msg.push_str(issue);
        msg.push('\n');
    }
    msg
}

/// Pulls the RON out of a fenced code block, returning `(prose_before, ron)`.
/// Accepts a ```ron fence or a bare ``` fence; `None` if there is no closed block.
pub(crate) fn extract_ron_block(text: &str) -> Option<(String, String)> {
    let (open_at, after) = if let Some(pos) = text.find("```ron") {
        (pos, &text[pos + "```ron".len()..])
    } else {
        let pos = text.find("```")?;
        (pos, &text[pos + 3..])
    };
    let end = after.find("```")?;
    let code = after[..end].trim().to_owned();
    let explanation = text[..open_at].trim().to_owned();
    Some((explanation, code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AiError, ScriptedProvider};
    use crate::EXAMPLE_PATCH_RON;

    fn reply_with(ron: &str) -> String {
        format!("Here is a simple patch.\n\n```ron\n{ron}\n```\n")
    }

    /// A valid doc using real kinds but a node the registry does not know.
    const UNKNOWN_KIND_RON: &str = r#"(
        version: 1,
        nodes: [ (id: (0), node: (kind: "wobbletron", inputs: [], outputs: [
            (name: "out", direction: Output, signal: Audio) ], feedback: false)) ],
        edges: [],
    )"#;

    fn setup() -> (Registry, ProviderConfig) {
        (Registry::with_builtins(), ProviderConfig::default())
    }

    #[test]
    fn generates_a_valid_patch_on_the_first_try() {
        let (registry, config) = setup();
        let provider = ScriptedProvider::replies(&[&reply_with(EXAMPLE_PATCH_RON)]);

        let out = generate_patch(&provider, &registry, &config, "a simple tone", None, 3)
            .expect("valid patch");
        assert_eq!(out.attempts, 1);
        assert_eq!(out.doc.to_graph().unwrap().node_count(), 2);
        assert!(
            out.explanation.contains("simple patch"),
            "prose is captured"
        );
        // The system prompt the model saw was grounded with the node reference.
        assert!(provider.requests()[0]
            .system
            .contains("oscillator [generator]"));
    }

    #[test]
    fn the_authored_interface_survives_the_pipeline() {
        // v2 contract: a reply may carry panel data (values / widgets / panel_pos / colour).
        // The pipeline must pass it through untouched so the generated instrument arrives
        // with its interface, not just its graph. The embedded example already carries it.
        let (registry, config) = setup();
        let provider = ScriptedProvider::replies(&[&reply_with(EXAMPLE_PATCH_RON)]);

        let out = generate_patch(
            &provider,
            &registry,
            &config,
            "a tone with a panel",
            None,
            3,
        )
        .expect("valid patch");
        let osc = &out.doc.nodes[0];
        assert_eq!(
            osc.values,
            vec![2.0, 110.0],
            "parameter values carried through"
        );
        assert_eq!(
            osc.widgets,
            vec!["switch".to_owned(), "knob".to_owned()],
            "per-parameter widgets carried through"
        );
        assert_eq!(
            osc.panel_pos,
            Some((0.0, 0.0)),
            "panel placement carried through"
        );
        let amp = &out.doc.nodes[1];
        assert_eq!(
            amp.panel_color.as_deref(),
            Some("#88ccff"),
            "panel accent colour carried through"
        );
    }

    #[test]
    fn recovers_after_the_validator_rejects_the_first_answer() {
        let (registry, config) = setup();
        // First: an unknown kind (rejected); second: the valid example.
        let provider = ScriptedProvider::replies(&[
            &reply_with(UNKNOWN_KIND_RON),
            &reply_with(EXAMPLE_PATCH_RON),
        ]);

        let out =
            generate_patch(&provider, &registry, &config, "make it", None, 3).expect("recovers");
        assert_eq!(out.attempts, 2, "second attempt succeeded");

        // The retry carried the validator's complaint back to the model.
        let second = &provider.requests()[1];
        let fed_back = &second.messages.last().unwrap().content;
        assert!(fed_back.contains("unknown kind 'wobbletron'"));
    }

    #[test]
    fn gives_up_with_issues_after_exhausting_attempts() {
        let (registry, config) = setup();
        let provider = ScriptedProvider::replies(&[
            &reply_with(UNKNOWN_KIND_RON),
            &reply_with(UNKNOWN_KIND_RON),
        ]);

        let err = generate_patch(&provider, &registry, &config, "x", None, 2).unwrap_err();
        match err {
            GenerateError::NotConverged { issues, attempts } => {
                assert_eq!(attempts, 2);
                assert!(issues.iter().any(|i| i.contains("wobbletron")));
            }
            other => panic!("expected NotConverged, got {other:?}"),
        }
    }

    #[test]
    fn a_reply_without_a_code_block_is_an_issue_not_a_crash() {
        let (registry, config) = setup();
        let provider =
            ScriptedProvider::replies(&["I think you want a saw wave, but here's prose only."]);
        let err = generate_patch(&provider, &registry, &config, "x", None, 1).unwrap_err();
        match err {
            GenerateError::NotConverged { issues, .. } => {
                assert!(issues[0].contains("no RON code block"));
            }
            other => panic!("expected NotConverged, got {other:?}"),
        }
    }

    #[test]
    fn malformed_ron_is_reported_as_a_parse_issue() {
        let (registry, config) = setup();
        let provider = ScriptedProvider::replies(&["```ron\nnot actually ron\n```"]);
        let err = generate_patch(&provider, &registry, &config, "x", None, 1).unwrap_err();
        match err {
            GenerateError::NotConverged { issues, .. } => {
                assert!(issues[0].contains("did not parse"));
            }
            other => panic!("expected NotConverged, got {other:?}"),
        }
    }

    #[test]
    fn a_provider_failure_propagates() {
        let (registry, config) = setup();
        let provider = ScriptedProvider::new(vec![Err(AiError::MissingKey)]);
        let err = generate_patch(&provider, &registry, &config, "x", None, 3).unwrap_err();
        assert_eq!(err, GenerateError::Provider(AiError::MissingKey));
    }

    #[test]
    fn edit_patch_returns_a_validated_doc_and_a_preview_diff() {
        use vb_core::{Graph, GraphNode, Port, PortId, SignalType};

        let (registry, config) = setup();
        // Current patch: osc(0) -> gain(1).
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
        let current = ProjectDoc::from_graph(&g);

        // The model returns a full edited doc that inserts a biquad between them.
        let edited = ProjectDoc {
            version: vb_project::FORMAT_VERSION,
            nodes: vec![
                vb_project::NodeEntry::new(
                    vb_core::NodeId(0),
                    GraphNode::new(
                        "oscillator",
                        vec![],
                        vec![Port::output("out", SignalType::Audio)],
                    ),
                ),
                vb_project::NodeEntry::new(
                    vb_core::NodeId(1),
                    GraphNode::new(
                        "gain",
                        vec![Port::input("in", SignalType::Audio)],
                        vec![Port::output("out", SignalType::Audio)],
                    ),
                ),
                vb_project::NodeEntry::new(
                    vb_core::NodeId(2),
                    GraphNode::new(
                        "biquad",
                        vec![
                            Port::input("in", SignalType::Audio),
                            Port::input("cutoff", SignalType::Control),
                        ],
                        vec![Port::output("out", SignalType::Audio)],
                    ),
                ),
            ],
            edges: vec![
                vb_core::Edge {
                    from_node: vb_core::NodeId(0),
                    from_port: PortId(0),
                    to_node: vb_core::NodeId(2),
                    to_port: PortId(0),
                },
                vb_core::Edge {
                    from_node: vb_core::NodeId(2),
                    from_port: PortId(0),
                    to_node: vb_core::NodeId(1),
                    to_port: PortId(0),
                },
            ],
        };
        let ron = vb_project::to_ron(&edited).unwrap();
        let provider = ScriptedProvider::replies(&[&reply_with(&ron)]);

        let out = edit_patch(
            &provider,
            &registry,
            &config,
            &current,
            "add a lowpass filter",
            2,
        )
        .expect("edit succeeds");
        assert_eq!(out.attempts, 1);
        assert_eq!(
            out.diff.added_nodes,
            vec![(vb_core::NodeId(2), "biquad".to_owned())]
        );
        assert!(
            out.diff.removed_edges.len() == 1,
            "old osc->gain edge dropped"
        );
        assert!(!out.diff.is_empty());

        // The instruction was framed as an edit and reached the model.
        assert!(provider.requests()[0].messages[0]
            .content
            .contains("add a lowpass filter"));
        assert!(provider.requests()[0].messages[0]
            .content
            .contains("Keep existing node ids stable"));
    }

    /// A single-oscillator doc (params: waveform 0..3, freq_hz 20..20000) for panel checks.
    fn osc_doc() -> ProjectDoc {
        use vb_core::{GraphNode, NodeId, Port, SignalType};
        ProjectDoc {
            version: vb_project::FORMAT_VERSION,
            nodes: vec![vb_project::NodeEntry::new(
                NodeId(0),
                GraphNode::new(
                    "oscillator",
                    vec![],
                    vec![Port::output("out", SignalType::Audio)],
                ),
            )],
            edges: vec![],
        }
    }

    #[test]
    fn the_embedded_example_has_no_panel_issues() {
        // The example the model is told to mirror must itself pass the panel gate, or the
        // contract contradicts the validator.
        let (registry, _) = setup();
        let doc = vb_project::from_ron(EXAMPLE_PATCH_RON).expect("parses");
        assert!(
            patch_issues(&doc, &registry).is_empty(),
            "example should be fully coherent: {:?}",
            patch_issues(&doc, &registry)
        );
    }

    #[test]
    fn an_unknown_widget_label_is_a_panel_issue() {
        let (registry, _) = setup();
        let mut doc = osc_doc();
        doc.nodes[0].widgets = vec!["knob".to_owned(), "dial".to_owned()];
        let issues = patch_issues(&doc, &registry);
        assert!(
            issues.iter().any(|i| i.contains("unknown widget 'dial'")),
            "got {issues:?}"
        );
    }

    #[test]
    fn more_widgets_than_parameters_is_a_panel_issue() {
        let (registry, _) = setup();
        let mut doc = osc_doc();
        // Oscillator has 2 params; three widgets is incoherent.
        doc.nodes[0].widgets = vec!["knob".to_owned(), "knob".to_owned(), "knob".to_owned()];
        let issues = patch_issues(&doc, &registry);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("lists 3 widgets but the kind has 2 parameter")),
            "got {issues:?}"
        );
    }

    #[test]
    fn a_value_out_of_range_is_a_panel_issue() {
        let (registry, _) = setup();
        let mut doc = osc_doc();
        // freq_hz max is 20000; 48000 is out of range.
        doc.nodes[0].values = vec![0.0, 48_000.0];
        let issues = patch_issues(&doc, &registry);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("freq_hz") && i.contains("outside its range")),
            "got {issues:?}"
        );
    }

    #[test]
    fn too_many_values_is_a_panel_issue() {
        let (registry, _) = setup();
        let mut doc = osc_doc();
        doc.nodes[0].values = vec![0.0, 440.0, 0.5];
        let issues = patch_issues(&doc, &registry);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("sets 3 values but the kind has 2 parameter")),
            "got {issues:?}"
        );
    }

    #[test]
    fn a_hidden_index_out_of_bounds_is_a_panel_issue() {
        let (registry, _) = setup();
        let mut doc = osc_doc();
        doc.nodes[0].panel_hidden = vec![5];
        let issues = patch_issues(&doc, &registry);
        assert!(
            issues.iter().any(|i| i.contains("hides parameter index 5")),
            "got {issues:?}"
        );
    }

    #[test]
    fn a_malformed_panel_colour_is_a_panel_issue() {
        let (registry, _) = setup();
        let mut doc = osc_doc();
        doc.nodes[0].panel_color = Some("blue".to_owned());
        let issues = patch_issues(&doc, &registry);
        assert!(
            issues
                .iter()
                .any(|i| i.contains("panel_color 'blue' is not a #rrggbb")),
            "got {issues:?}"
        );
        // A well-formed colour passes.
        doc.nodes[0].panel_color = Some("#1a2b3c".to_owned());
        assert!(patch_issues(&doc, &registry)
            .iter()
            .all(|i| !i.contains("panel_color")));
    }

    #[test]
    fn a_within_range_panel_is_accepted() {
        let (registry, _) = setup();
        let mut doc = osc_doc();
        doc.nodes[0].values = vec![2.0, 110.0];
        doc.nodes[0].widgets = vec!["switch".to_owned(), "knob".to_owned()];
        doc.nodes[0].panel_pos = Some((0.0, 0.0));
        doc.nodes[0].panel_color = Some("#88ccff".to_owned());
        assert!(
            patch_issues(&doc, &registry).is_empty(),
            "a coherent panel must pass: {:?}",
            patch_issues(&doc, &registry)
        );
    }

    #[test]
    fn the_retry_loop_rejects_an_incoherent_panel_then_recovers() {
        // A reply whose panel lies about the graph (a bogus widget) is rejected and fed
        // back; a corrected reply then converges — the panel gate behaves like the
        // structural gate.
        let (registry, config) = setup();
        let bad = r#"(
            version: 1,
            nodes: [ (id: (0), node: (kind: "oscillator", inputs: [], outputs: [
                (name: "out", direction: Output, signal: Audio) ], feedback: false),
                widgets: ["wobble"]) ],
            edges: [],
        )"#;
        let provider =
            ScriptedProvider::replies(&[&reply_with(bad), &reply_with(EXAMPLE_PATCH_RON)]);
        let out =
            generate_patch(&provider, &registry, &config, "a tone", None, 3).expect("recovers");
        assert_eq!(out.attempts, 2);
        let second = &provider.requests()[1];
        let fed_back = &second.messages.last().unwrap().content;
        assert!(
            fed_back.contains("unknown widget 'wobble'"),
            "the panel issue was fed back: {fed_back}"
        );
    }

    #[test]
    fn extract_handles_bare_and_tagged_fences() {
        let (_e, code) = extract_ron_block("pre\n```ron\nBODY\n```\npost").unwrap();
        assert_eq!(code, "BODY");
        let (expl, code) = extract_ron_block("why\n```\nBODY2\n```").unwrap();
        assert_eq!(code, "BODY2");
        assert_eq!(expl, "why");
        assert!(extract_ron_block("no fence here").is_none());
    }
}
