//! The assistant **chat panel** as a UI-agnostic view-model (backlog 000108).
//!
//! An editor's assistant panel is a small conversation loop: the user types a request, the
//! assistant proposes an edit (a validated new patch plus a [`PatchDiff`] to preview), and
//! the user either **applies** it — the proposal becomes the live patch — or **rejects** it,
//! leaving the canvas untouched. This type is that loop as pure state, with no egui or other
//! UI framework in sight, so it can be unit-tested against a [`ScriptedProvider`] and later
//! bound to whatever renders it.
//!
//! The live patch is never mutated by a proposal — only [`EditSession::apply`] swaps it in,
//! so the editor can wrap that single step as one undoable command.

use seclab_nodes::Registry;
use seclab_project::ProjectDoc;

use crate::generate::{edit_patch, EditOutcome, GenerateError};
use crate::provider::{AiProvider, ProviderConfig};

/// What the panel is currently waiting on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditState {
    /// No proposal pending — the live patch is settled.
    Idle,
    /// A proposal is awaiting the user's apply/reject decision.
    Proposed,
    /// The last request failed; the user can try again.
    Failed,
}

/// How a submitted request resolved — one row of the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStatus {
    /// Proposed and still awaiting a decision.
    Pending,
    /// The proposal was applied to the live patch.
    Applied,
    /// The proposal was rejected and discarded.
    Rejected,
    /// A newer request replaced this one before it was decided.
    Superseded,
    /// The request failed; carries a human-readable reason.
    Failed(String),
}

/// One entry in the panel's transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// The natural-language request the user submitted.
    pub request: String,
    /// The assistant's plain-language explanation (empty on failure).
    pub explanation: String,
    /// How the request resolved.
    pub status: TurnStatus,
}

/// The assistant chat panel's state: the live patch, the transcript, and any pending
/// proposal awaiting a decision.
pub struct EditSession {
    doc: ProjectDoc,
    transcript: Vec<Turn>,
    pending: Option<EditOutcome>,
}

impl EditSession {
    /// Starts a panel over the current live patch.
    #[must_use]
    pub fn new(doc: ProjectDoc) -> Self {
        Self {
            doc,
            transcript: Vec::new(),
            pending: None,
        }
    }

    /// The live patch (unchanged until a proposal is applied).
    #[must_use]
    pub fn doc(&self) -> &ProjectDoc {
        &self.doc
    }

    /// The conversation so far, oldest first.
    #[must_use]
    pub fn transcript(&self) -> &[Turn] {
        &self.transcript
    }

    /// The proposal awaiting a decision, if any — its `diff` is what the panel previews.
    #[must_use]
    pub fn pending(&self) -> Option<&EditOutcome> {
        self.pending.as_ref()
    }

    /// What the panel is waiting on.
    #[must_use]
    pub fn state(&self) -> EditState {
        if self.pending.is_some() {
            EditState::Proposed
        } else if matches!(self.transcript.last(), Some(t) if matches!(t.status, TurnStatus::Failed(_)))
        {
            EditState::Failed
        } else {
            EditState::Idle
        }
    }

    /// Submits `instruction` and proposes an edit against the **live** patch. A proposal is
    /// only previewed — the live patch is not touched until [`apply`](Self::apply). If a
    /// proposal was already pending, it is superseded (the user typed a new request instead
    /// of deciding on the old one).
    ///
    /// On success the panel enters [`EditState::Proposed`]; on failure it enters
    /// [`EditState::Failed`] and records the reason. Returns the same result `edit_patch`
    /// does so the caller can surface provider errors.
    ///
    /// # Errors
    /// [`GenerateError`] from the underlying [`edit_patch`] call.
    pub fn submit(
        &mut self,
        provider: &dyn AiProvider,
        registry: &Registry,
        config: &ProviderConfig,
        instruction: &str,
        max_attempts: u32,
    ) -> Result<(), GenerateError> {
        let result = edit_patch(
            provider,
            registry,
            config,
            &self.doc,
            instruction,
            max_attempts,
        );
        self.record(instruction, result)
    }

    /// Records a proposal that was computed **elsewhere** — e.g. by a background worker that
    /// ran [`edit_patch`] against a snapshot of [`doc`](Self::doc) — into the session's state
    /// machine, so a UI host can keep the blocking provider call off the render thread and
    /// hand the result back on the main thread without the session ever crossing threads.
    ///
    /// Behaves exactly like the tail of [`submit`](Self::submit): a new request supersedes
    /// any still-undecided proposal, then on `Ok` a pending turn is pushed and the outcome
    /// stored ([`EditState::Proposed`]); on `Err` a failed turn is recorded
    /// ([`EditState::Failed`]) and the live patch is left untouched. `result` should be the
    /// outcome of `edit_patch` run against this session's document.
    ///
    /// # Errors
    /// Forwards the `Err` passed in, after recording it on the transcript.
    pub fn record(
        &mut self,
        instruction: &str,
        result: Result<EditOutcome, GenerateError>,
    ) -> Result<(), GenerateError> {
        // A new request supersedes an undecided proposal.
        if self.pending.take().is_some() {
            if let Some(last) = self.transcript.last_mut() {
                if last.status == TurnStatus::Pending {
                    last.status = TurnStatus::Superseded;
                }
            }
        }

        match result {
            Ok(outcome) => {
                self.transcript.push(Turn {
                    request: instruction.to_owned(),
                    explanation: outcome.explanation.clone(),
                    status: TurnStatus::Pending,
                });
                self.pending = Some(outcome);
                Ok(())
            }
            Err(e) => {
                self.transcript.push(Turn {
                    request: instruction.to_owned(),
                    explanation: String::new(),
                    status: TurnStatus::Failed(e.to_string()),
                });
                Err(e)
            }
        }
    }

    /// Applies the pending proposal: it becomes the live patch. Returns the new live patch,
    /// or `None` if there was nothing to apply. The caller installs this as one undo step.
    pub fn apply(&mut self) -> Option<&ProjectDoc> {
        let outcome = self.pending.take()?;
        self.doc = outcome.doc;
        if let Some(last) = self.transcript.last_mut() {
            if last.status == TurnStatus::Pending {
                last.status = TurnStatus::Applied;
            }
        }
        Some(&self.doc)
    }

    /// Rejects the pending proposal: it is discarded and the live patch is unchanged.
    /// Returns `true` if a proposal was pending.
    pub fn reject(&mut self) -> bool {
        if self.pending.take().is_none() {
            return false;
        }
        if let Some(last) = self.transcript.last_mut() {
            if last.status == TurnStatus::Pending {
                last.status = TurnStatus::Rejected;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ScriptedProvider;
    use crate::EXAMPLE_PATCH_RON;

    fn reply_with(ron: &str) -> String {
        format!("Here is the edited patch.\n\n```ron\n{ron}\n```\n")
    }

    fn setup() -> (Registry, ProviderConfig, ProjectDoc) {
        let registry = Registry::with_builtins();
        // Start from an empty patch; the scripted reply proposes the example patch.
        let doc =
            seclab_project::from_ron("(version: 1, nodes: [], edges: [])").expect("empty doc");
        (registry, ProviderConfig::default(), doc)
    }

    #[test]
    fn a_fresh_session_is_idle_with_the_given_patch() {
        let (_, _, doc) = setup();
        let session = EditSession::new(doc.clone());
        assert_eq!(session.state(), EditState::Idle);
        assert_eq!(session.doc(), &doc);
        assert!(session.transcript().is_empty());
        assert!(session.pending().is_none());
    }

    #[test]
    fn a_submitted_edit_is_proposed_but_not_yet_applied() {
        let (registry, config, doc) = setup();
        let mut session = EditSession::new(doc.clone());
        let provider = ScriptedProvider::replies(&[&reply_with(EXAMPLE_PATCH_RON)]);

        session
            .submit(&provider, &registry, &config, "add a filter", 3)
            .expect("proposes an edit");

        assert_eq!(session.state(), EditState::Proposed);
        assert!(session.pending().is_some(), "a proposal is pending");
        // The live patch is untouched until the user applies.
        assert_eq!(
            session.doc(),
            &doc,
            "the canvas is not changed by a proposal"
        );
        assert_eq!(session.transcript().len(), 1);
        assert_eq!(session.transcript()[0].request, "add a filter");
        assert_eq!(session.transcript()[0].status, TurnStatus::Pending);
    }

    #[test]
    fn applying_swaps_in_the_proposed_patch() {
        let (registry, config, doc) = setup();
        let mut session = EditSession::new(doc.clone());
        let provider = ScriptedProvider::replies(&[&reply_with(EXAMPLE_PATCH_RON)]);
        session
            .submit(&provider, &registry, &config, "add a filter", 3)
            .unwrap();
        let proposed = session.pending().unwrap().doc.clone();

        let applied = session.apply().expect("there was a proposal").clone();
        assert_eq!(applied, proposed, "the live patch is now the proposal");
        assert_ne!(applied, doc, "and differs from the original");
        assert_eq!(session.state(), EditState::Idle);
        assert!(session.pending().is_none());
        assert_eq!(session.transcript()[0].status, TurnStatus::Applied);
    }

    #[test]
    fn rejecting_discards_the_proposal_and_keeps_the_patch() {
        let (registry, config, doc) = setup();
        let mut session = EditSession::new(doc.clone());
        let provider = ScriptedProvider::replies(&[&reply_with(EXAMPLE_PATCH_RON)]);
        session
            .submit(&provider, &registry, &config, "add a filter", 3)
            .unwrap();

        assert!(session.reject(), "there was a proposal to reject");
        assert_eq!(session.doc(), &doc, "the canvas is unchanged");
        assert_eq!(session.state(), EditState::Idle);
        assert!(session.pending().is_none());
        assert_eq!(session.transcript()[0].status, TurnStatus::Rejected);
    }

    #[test]
    fn apply_and_reject_are_no_ops_without_a_proposal() {
        let (_, _, doc) = setup();
        let mut session = EditSession::new(doc);
        assert!(session.apply().is_none());
        assert!(!session.reject());
    }

    #[test]
    fn a_failed_request_is_recorded_and_leaves_the_patch_alone() {
        let (registry, config, doc) = setup();
        let mut session = EditSession::new(doc.clone());
        // A reply with no RON block never converges within the cap → GenerateError.
        let provider =
            ScriptedProvider::replies(&["Sorry, I only have prose for you.", "still prose"]);

        let err = session.submit(&provider, &registry, &config, "do something", 2);
        assert!(err.is_err(), "a non-converging edit fails");
        assert_eq!(session.state(), EditState::Failed);
        assert_eq!(session.doc(), &doc, "the canvas is untouched on failure");
        assert!(matches!(
            session.transcript()[0].status,
            TurnStatus::Failed(_)
        ));
    }

    #[test]
    fn a_new_request_supersedes_an_undecided_proposal() {
        let (registry, config, doc) = setup();
        let mut session = EditSession::new(doc);
        let provider = ScriptedProvider::replies(&[
            &reply_with(EXAMPLE_PATCH_RON),
            &reply_with(EXAMPLE_PATCH_RON),
        ]);

        session
            .submit(&provider, &registry, &config, "first idea", 3)
            .unwrap();
        session
            .submit(&provider, &registry, &config, "second idea", 3)
            .unwrap();

        assert_eq!(session.transcript().len(), 2);
        assert_eq!(session.transcript()[0].status, TurnStatus::Superseded);
        assert_eq!(session.transcript()[1].status, TurnStatus::Pending);
        assert_eq!(session.state(), EditState::Proposed);
    }

    #[test]
    fn a_session_recovers_after_a_failure() {
        let (registry, config, doc) = setup();
        let mut session = EditSession::new(doc);
        let provider = ScriptedProvider::replies(&[
            "prose only",
            "prose again",
            &reply_with(EXAMPLE_PATCH_RON),
        ]);

        assert!(session
            .submit(&provider, &registry, &config, "bad", 2)
            .is_err());
        assert_eq!(session.state(), EditState::Failed);

        session
            .submit(&provider, &registry, &config, "good", 3)
            .expect("a later request can still succeed");
        assert_eq!(session.state(), EditState::Proposed);
        assert!(session.apply().is_some());
        assert_eq!(session.state(), EditState::Idle);
    }

    /// Computes an outcome the way a background worker would (via `edit_patch`), then hands it
    /// to `record` on a session that never saw the provider — the threaded-host path.
    fn worker_outcome(instruction: &str) -> (ProjectDoc, Result<EditOutcome, GenerateError>) {
        let (registry, config, doc) = setup();
        let provider = ScriptedProvider::replies(&[&reply_with(EXAMPLE_PATCH_RON)]);
        let outcome = edit_patch(&provider, &registry, &config, &doc, instruction, 3);
        (doc, outcome)
    }

    #[test]
    fn record_stores_an_outcome_computed_off_thread_as_a_proposal() {
        let (doc, outcome) = worker_outcome("build it");
        let proposed = outcome.as_ref().expect("valid outcome").doc.clone();
        let mut session = EditSession::new(doc.clone());

        session
            .record("build it", outcome)
            .expect("records the proposal");

        assert_eq!(session.state(), EditState::Proposed);
        assert_eq!(session.pending().unwrap().doc, proposed);
        assert_eq!(
            session.doc(),
            &doc,
            "recording a proposal does not touch the canvas"
        );
        assert_eq!(session.transcript().len(), 1);
        assert_eq!(session.transcript()[0].request, "build it");
        assert_eq!(session.transcript()[0].status, TurnStatus::Pending);
        // Applying it swaps the proposal in, same as a submit would.
        assert_eq!(session.apply().cloned(), Some(proposed));
    }

    #[test]
    fn record_of_an_error_marks_the_turn_failed_and_keeps_the_patch() {
        let (_, _, doc) = setup();
        let mut session = EditSession::new(doc.clone());

        let err = session.record(
            "do it",
            Err(GenerateError::NotConverged {
                issues: vec!["no code block".to_owned()],
                attempts: 2,
            }),
        );
        assert!(err.is_err());
        assert_eq!(session.state(), EditState::Failed);
        assert_eq!(session.doc(), &doc);
        assert!(matches!(
            session.transcript()[0].status,
            TurnStatus::Failed(_)
        ));
    }

    #[test]
    fn record_supersedes_an_undecided_proposal_like_submit() {
        let (doc, first) = worker_outcome("first");
        let mut session = EditSession::new(doc);
        session.record("first", first).unwrap();
        let (_, second) = worker_outcome("second");
        session.record("second", second).unwrap();

        assert_eq!(session.transcript().len(), 2);
        assert_eq!(session.transcript()[0].status, TurnStatus::Superseded);
        assert_eq!(session.transcript()[1].status, TurnStatus::Pending);
        assert_eq!(session.state(), EditState::Proposed);
    }
}
