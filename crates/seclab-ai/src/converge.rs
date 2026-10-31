//! The propose → validate → repair loop, as a reusable primitive.
//!
//! Every assistant flow that must return something *valid* shares the same shape: ask the
//! model, gate the reply, and — if it fails the gate — feed the exact problems back and
//! ask again, up to a retry cap. [`converge`] is that loop, parameterized over the gate
//! (an `evaluate` function returning [`Verdict`]) and the feedback wording, so patch
//! generation, editing, and repair all run one implementation instead of three copies.
//!
//! The gate is a hard wall: `converge` only ever returns a value the evaluator *accepted*,
//! so an invalid proposal can never escape.

use crate::provider::{AiError, AiProvider, ChatRequest, Message, ProviderConfig};

/// The evaluator's ruling on one reply.
pub enum Verdict<T> {
    /// The reply passed the gate; carry the extracted value out.
    Accept(T),
    /// The reply failed; these problems are fed back to the model for another try.
    Revise(Vec<String>),
}

/// A converged result: the accepted value and how many attempts it took.
#[derive(Debug, Clone, PartialEq)]
pub struct Converged<T> {
    /// The accepted value.
    pub value: T,
    /// Model round-trips it took (1 = first try).
    pub attempts: u32,
}

/// Why the loop did not produce an accepted value.
#[derive(Debug, Clone, PartialEq)]
pub enum ConvergeError {
    /// A provider call failed.
    Provider(AiError),
    /// The retry cap was reached without acceptance; carries the final attempt's issues.
    NotConverged {
        /// Problems from the last attempt.
        issues: Vec<String>,
        /// Attempts spent.
        attempts: u32,
    },
}

/// Runs the propose → validate → repair loop under `system`, starting from `first_user`.
///
/// Each attempt sends the conversation, runs `evaluate` on the reply, and either returns
/// the accepted value or appends the reply plus `feedback(issues)` and tries again, up to
/// `max_attempts` (clamped to at least 1).
///
/// # Errors
/// [`ConvergeError::Provider`] if a provider call fails, or [`ConvergeError::NotConverged`]
/// if the cap is reached without an accepted reply.
pub fn converge<T>(
    provider: &dyn AiProvider,
    config: &ProviderConfig,
    system: &str,
    first_user: String,
    max_attempts: u32,
    evaluate: impl Fn(&str) -> Verdict<T>,
    feedback: impl Fn(&[String]) -> String,
) -> Result<Converged<T>, ConvergeError> {
    let budget = max_attempts.max(1);
    let mut messages = vec![Message::user(first_user)];
    let mut last_issues = vec!["no attempts were made".to_owned()];

    for attempt in 1..=budget {
        let request = ChatRequest::new(config, system, messages.clone());
        let reply = provider
            .complete(&request)
            .map_err(ConvergeError::Provider)?;

        match evaluate(&reply.content) {
            Verdict::Accept(value) => {
                return Ok(Converged {
                    value,
                    attempts: attempt,
                })
            }
            Verdict::Revise(issues) => {
                last_issues = issues;
                messages.push(Message::assistant(reply.content));
                messages.push(Message::user(feedback(&last_issues)));
            }
        }
    }

    Err(ConvergeError::NotConverged {
        issues: last_issues,
        attempts: budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ScriptedProvider;

    fn config() -> ProviderConfig {
        ProviderConfig::default()
    }

    /// Accept a reply that says "ok", otherwise revise with the reply as the issue.
    fn accept_ok(content: &str) -> Verdict<String> {
        if content.trim() == "ok" {
            Verdict::Accept(content.trim().to_owned())
        } else {
            Verdict::Revise(vec![format!("not ok: {}", content.trim())])
        }
    }

    fn feedback(issues: &[String]) -> String {
        format!("fix: {}", issues.join(", "))
    }

    #[test]
    fn accepts_on_the_first_valid_reply() {
        let provider = ScriptedProvider::replies(&["ok"]);
        let out = converge(
            &provider,
            &config(),
            "sys",
            "go".to_owned(),
            3,
            accept_ok,
            feedback,
        )
        .unwrap();
        assert_eq!(out.attempts, 1);
        assert_eq!(out.value, "ok");
    }

    #[test]
    fn revises_then_accepts_and_feeds_the_issue_back() {
        let provider = ScriptedProvider::replies(&["nope", "ok"]);
        let out = converge(
            &provider,
            &config(),
            "sys",
            "go".to_owned(),
            3,
            accept_ok,
            feedback,
        )
        .unwrap();
        assert_eq!(out.attempts, 2);

        // The second request carried the assistant's wrong answer and the feedback.
        let second = &provider.requests()[1];
        assert_eq!(second.messages.len(), 3, "user, assistant, user(feedback)");
        assert_eq!(second.messages[1].content, "nope");
        assert!(second.messages[2].content.contains("fix: not ok: nope"));
    }

    #[test]
    fn gives_up_after_the_cap_with_the_last_issues() {
        let provider = ScriptedProvider::replies(&["bad", "worse"]);
        let err = converge(
            &provider,
            &config(),
            "sys",
            "go".to_owned(),
            2,
            accept_ok,
            feedback,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ConvergeError::NotConverged {
                issues: vec!["not ok: worse".to_owned()],
                attempts: 2
            }
        );
    }

    #[test]
    fn a_provider_error_propagates() {
        let provider = ScriptedProvider::new(vec![Err(AiError::MissingKey)]);
        let err = converge(
            &provider,
            &config(),
            "sys",
            "go".to_owned(),
            3,
            accept_ok,
            feedback,
        )
        .unwrap_err();
        assert_eq!(err, ConvergeError::Provider(AiError::MissingKey));
    }

    #[test]
    fn max_attempts_is_clamped_to_at_least_one() {
        let provider = ScriptedProvider::replies(&["ok"]);
        let out = converge(
            &provider,
            &config(),
            "sys",
            "go".to_owned(),
            0,
            accept_ok,
            feedback,
        )
        .unwrap();
        assert_eq!(out.attempts, 1);
    }
}
