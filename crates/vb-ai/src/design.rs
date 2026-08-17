//! The "design a synth/effect from a description" skill: a guided session where the
//! assistant may ask clarifying questions before drafting a validated patch.
//!
//! Unlike a one-shot [`generate_patch`](crate::generate::generate_patch), designing is a
//! conversation. Each [`advance`](DesignSession::advance) runs one model turn; the reply
//! is either a clarifying **question** (prose, which the UI answers with
//! [`answer`](DesignSession::answer) before advancing again) or a **draft** — a full RON
//! `ProjectDoc` put through the same mandatory validation. An invalid draft is repaired
//! inline (issues fed straight back) rather than bothering the user, up to a fix cap.

use vb_nodes::Registry;

use crate::generate::{evaluate_reply, extract_ron_block, retry_prompt, GenerateOutcome};
use crate::prompt::build_system_prompt;
use crate::provider::{AiError, AiProvider, ChatRequest, Message, ProviderConfig};

/// One step of a design session.
#[derive(Debug, Clone, PartialEq)]
pub enum DesignStep {
    /// The assistant asked a clarifying question; call [`DesignSession::answer`] with the
    /// user's reply, then [`DesignSession::advance`] again.
    Question(String),
    /// The assistant produced a validated draft patch.
    Draft(Box<GenerateOutcome>),
}

/// Why a design step failed.
#[derive(Debug, Clone, PartialEq)]
pub enum DesignError {
    /// A provider call failed.
    Provider(AiError),
    /// The assistant kept emitting invalid drafts past the fix cap; carries the last
    /// issues.
    NotConverged(Vec<String>),
}

impl std::fmt::Display for DesignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesignError::Provider(e) => write!(f, "provider error: {e}"),
            DesignError::NotConverged(issues) => {
                write!(f, "the draft never validated: {}", issues.join("; "))
            }
        }
    }
}

impl std::error::Error for DesignError {}

/// A running design conversation: the grounded system prompt plus the turns so far.
pub struct DesignSession {
    system: String,
    messages: Vec<Message>,
    config: ProviderConfig,
}

impl DesignSession {
    /// Starts a session for `description`. The opening turn asks the assistant to ask a
    /// clarifying question when the request is ambiguous, and otherwise to draft.
    #[must_use]
    pub fn new(registry: &Registry, config: &ProviderConfig, description: &str) -> Self {
        let brief = format!(
            "Design a VST_bomb patch for this request. If the request is ambiguous, ask ONE \
             short clarifying question in prose (no code) and wait for the answer. When you \
             have enough to proceed, output the full ProjectDoc per the contract.\n\n\
             Request: {description}"
        );
        Self {
            system: build_system_prompt(registry, None),
            messages: vec![Message::user(brief)],
            config: config.clone(),
        }
    }

    /// Records the user's answer to the last question.
    pub fn answer(&mut self, text: &str) {
        self.messages.push(Message::user(text.to_owned()));
    }

    /// The conversation so far (for inspection/preview).
    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Runs one model turn: returns a clarifying [`DesignStep::Question`] or a validated
    /// [`DesignStep::Draft`]. An invalid draft is repaired inline, feeding the validator's
    /// issues back up to `max_fix_attempts` times before giving up.
    ///
    /// # Errors
    /// [`DesignError::Provider`] on a provider failure, or [`DesignError::NotConverged`]
    /// if the draft never validates within the fix cap.
    pub fn advance(
        &mut self,
        provider: &dyn AiProvider,
        registry: &Registry,
        max_fix_attempts: u32,
    ) -> Result<DesignStep, DesignError> {
        let cap = max_fix_attempts.max(1);
        let mut last_issues = vec!["no attempts were made".to_owned()];

        for attempt in 1..=cap {
            let request =
                ChatRequest::new(&self.config, self.system.clone(), self.messages.clone());
            let reply = provider.complete(&request).map_err(DesignError::Provider)?;

            match evaluate_reply(&reply.content, registry) {
                Ok((doc, explanation)) => {
                    self.messages.push(Message::assistant(reply.content));
                    return Ok(DesignStep::Draft(Box::new(GenerateOutcome {
                        doc,
                        explanation,
                        attempts: attempt,
                    })));
                }
                Err(issues) => {
                    self.messages
                        .push(Message::assistant(reply.content.clone()));
                    if extract_ron_block(&reply.content).is_none() {
                        // No code block → this is a clarifying question, not a bad draft.
                        return Ok(DesignStep::Question(reply.content));
                    }
                    // A draft that failed validation: feed the issues back and retry.
                    last_issues = issues;
                    self.messages
                        .push(Message::user(retry_prompt(&last_issues)));
                }
            }
        }

        Err(DesignError::NotConverged(last_issues))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ScriptedProvider;
    use crate::EXAMPLE_PATCH_RON;

    fn setup() -> (Registry, ProviderConfig) {
        (Registry::with_builtins(), ProviderConfig::default())
    }

    fn fenced(ron: &str) -> String {
        format!("Here is a draft.\n\n```ron\n{ron}\n```\n")
    }

    #[test]
    fn the_opening_brief_invites_a_clarifying_question() {
        let (registry, config) = setup();
        let session = DesignSession::new(&registry, &config, "a bass");
        let brief = &session.messages()[0].content;
        assert!(brief.contains("ask ONE"));
        assert!(brief.contains("Request: a bass"));
        // The system prompt is the grounded contract.
        assert!(session.system.contains("oscillator [generator]"));
    }

    #[test]
    fn asks_a_question_then_drafts_after_the_answer() {
        let (registry, config) = setup();
        let provider = ScriptedProvider::replies(&[
            "What waveform do you want — saw or sine?",
            &fenced(EXAMPLE_PATCH_RON),
        ]);
        let mut session = DesignSession::new(&registry, &config, "a lead");

        // First turn: a clarifying question.
        match session.advance(&provider, &registry, 3).unwrap() {
            DesignStep::Question(q) => assert!(q.contains("waveform")),
            other => panic!("expected a question, got {other:?}"),
        }

        // Answer it, then advance: a validated draft.
        session.answer("saw");
        match session.advance(&provider, &registry, 3).unwrap() {
            DesignStep::Draft(out) => {
                assert!(out.doc.is_valid());
                assert_eq!(out.attempts, 1);
            }
            other => panic!("expected a draft, got {other:?}"),
        }

        // The second request carried the whole Q&A transcript.
        let second = &provider.requests()[1];
        assert!(second.messages.iter().any(|m| m.content == "saw"));
        assert!(second
            .messages
            .iter()
            .any(|m| m.content.contains("What waveform")));
    }

    #[test]
    fn a_confident_assistant_drafts_immediately() {
        let (registry, config) = setup();
        let provider = ScriptedProvider::replies(&[&fenced(EXAMPLE_PATCH_RON)]);
        let mut session = DesignSession::new(&registry, &config, "a sine tone");
        assert!(matches!(
            session.advance(&provider, &registry, 3).unwrap(),
            DesignStep::Draft(_)
        ));
    }

    #[test]
    fn an_invalid_draft_is_repaired_inline_without_a_user_turn() {
        let (registry, config) = setup();
        // First a draft with an unknown kind, then a valid one — the fix loop feeds the
        // issue back itself.
        let bad = r#"(version: 1, nodes: [ (id: (0), node: (kind: "wobble", inputs: [], outputs: [], feedback: false)) ], edges: [])"#;
        let provider = ScriptedProvider::replies(&[&fenced(bad), &fenced(EXAMPLE_PATCH_RON)]);
        let mut session = DesignSession::new(&registry, &config, "a pad");

        match session.advance(&provider, &registry, 3).unwrap() {
            DesignStep::Draft(out) => assert_eq!(out.attempts, 2, "fixed on the second try"),
            other => panic!("expected a draft, got {other:?}"),
        }
        // The second request fed the validator's complaint back.
        let second = &provider.requests()[1];
        assert!(second
            .messages
            .last()
            .unwrap()
            .content
            .contains("unknown kind 'wobble'"));
    }

    #[test]
    fn a_provider_error_propagates() {
        let (registry, config) = setup();
        let provider = ScriptedProvider::new(vec![Err(AiError::MissingKey)]);
        let mut session = DesignSession::new(&registry, &config, "x");
        assert_eq!(
            session.advance(&provider, &registry, 3),
            Err(DesignError::Provider(AiError::MissingKey))
        );
    }
}
