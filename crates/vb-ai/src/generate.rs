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

use crate::prompt::build_system_prompt;
use crate::provider::{AiError, AiProvider, ChatRequest, Message, ProviderConfig};

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
    let attempts_budget = max_attempts.max(1);
    let system = build_system_prompt(registry, current);
    let mut messages = vec![Message::user(description)];
    let mut last_issues = vec!["no attempts were made".to_owned()];

    for attempt in 1..=attempts_budget {
        let request = ChatRequest::new(config, system.clone(), messages.clone());
        let reply = provider
            .complete(&request)
            .map_err(GenerateError::Provider)?;

        match evaluate_reply(&reply.content, registry) {
            Ok((doc, explanation)) => {
                return Ok(GenerateOutcome {
                    doc,
                    explanation,
                    attempts: attempt,
                })
            }
            Err(issues) => {
                last_issues = issues;
                // Feed the wrong answer and the problems back for another try.
                messages.push(Message::assistant(reply.content));
                messages.push(Message::user(retry_prompt(&last_issues)));
            }
        }
    }

    Err(GenerateError::NotConverged {
        issues: last_issues,
        attempts: attempts_budget,
    })
}

/// Extracts and validates a reply. `Ok((doc, explanation))` on success, `Err(issues)`
/// with the problems to feed back otherwise.
fn evaluate_reply(content: &str, registry: &Registry) -> Result<(ProjectDoc, String), Vec<String>> {
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

/// Mandatory validation: unknown node kinds (grounding) plus every structural problem
/// [`ProjectDoc::validate`] reports.
fn patch_issues(doc: &ProjectDoc, registry: &Registry) -> Vec<String> {
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
    issues
}

/// The follow-up message listing the problems to fix.
fn retry_prompt(issues: &[String]) -> String {
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
fn extract_ron_block(text: &str) -> Option<(String, String)> {
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
    fn extract_handles_bare_and_tagged_fences() {
        let (_e, code) = extract_ron_block("pre\n```ron\nBODY\n```\npost").unwrap();
        assert_eq!(code, "BODY");
        let (expl, code) = extract_ron_block("why\n```\nBODY2\n```").unwrap();
        assert_eq!(code, "BODY2");
        assert_eq!(expl, "why");
        assert!(extract_ron_block("no fence here").is_none());
    }
}
