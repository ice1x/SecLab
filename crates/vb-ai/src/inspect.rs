//! "What will be sent?" inspector: a structured, reviewable breakdown of the exact
//! payload a [`ChatRequest`] carries, plus a per-session gate that forces that review
//! before the **first** request leaves the machine.
//!
//! [`crate::preview_request`] already renders a request as one text blob. This module is
//! the richer, UI-facing view: the payload is split into labelled [`Segment`]s — the
//! system prompt and every conversation turn — each carrying its own byte size and token
//! estimate, so a user can see *what* is being sent and *how much* of it (a large patch,
//! a long history) before consenting. Sizes reuse the [`crate::budget`] estimator so the
//! numbers here match the budget guard shown alongside.
//!
//! [`SessionInspector`] models the "before the first request of a session" requirement:
//! it demands a review until the user acknowledges the payload once, after which repeat
//! sends in the same session no longer force the modal (the payload stays inspectable on
//! demand).

use crate::budget::{estimate_tokens, estimate_usage, Usage};
use crate::provider::{ChatRequest, ProviderKind, Role};

/// One inspectable part of the outgoing payload: the system prompt or a single turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Human label, e.g. `"system prompt"` or `"message 2 (assistant)"`.
    pub label: String,
    /// Who authored it; `None` for the system prompt (which travels separately).
    pub role: Option<Role>,
    /// Wire size of this segment's content in bytes (UTF-8).
    pub bytes: usize,
    /// Estimated tokens for this segment (see [`crate::estimate_tokens`]).
    pub est_tokens: usize,
    /// The exact content that would be sent.
    pub content: String,
}

/// A structured breakdown of exactly what a [`ChatRequest`] would transmit.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestInspection {
    /// The backend the payload is bound for.
    pub provider: ProviderKind,
    /// Model identifier that will be requested.
    pub model: String,
    /// Reply token budget.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// The payload split into inspectable parts, system prompt first.
    pub segments: Vec<Segment>,
    /// Token usage estimate (prompt + reply budget), matching the budget guard.
    pub usage: Usage,
}

impl RequestInspection {
    /// Total wire size of the whole prompt payload in bytes (system + all turns).
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.segments.iter().map(|s| s.bytes).sum()
    }

    /// Number of conversation turns (excludes the system prompt).
    #[must_use]
    pub fn turn_count(&self) -> usize {
        self.segments.iter().filter(|s| s.role.is_some()).count()
    }

    /// Renders the inspection as human-readable text: a header with the destination and
    /// size estimate, then each segment with its own size, so a plain terminal or a
    /// tooltip can show the full "what will be sent" view without a custom widget.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "what will be sent to {} ({})\n\
             sampling: max_tokens={}, temperature={}\n\
             estimate: {} prompt tokens, up to {} total, {} bytes\n",
            self.provider.label(),
            self.model,
            self.max_tokens,
            self.temperature,
            self.usage.prompt_tokens,
            self.usage.total(),
            self.total_bytes(),
        );
        for segment in &self.segments {
            out.push_str(&format!(
                "\n── {} ── ({} bytes, ~{} tok)\n{}\n",
                segment.label, segment.bytes, segment.est_tokens, segment.content
            ));
        }
        out
    }
}

/// Builds a [`RequestInspection`] for `request` bound for `provider`, splitting the
/// payload into the system prompt and one segment per conversation turn.
#[must_use]
pub fn inspect_request(request: &ChatRequest, provider: ProviderKind) -> RequestInspection {
    let mut segments = Vec::with_capacity(request.messages.len() + 1);
    segments.push(Segment {
        label: "system prompt".to_owned(),
        role: None,
        bytes: request.system.len(),
        est_tokens: estimate_tokens(&request.system),
        content: request.system.clone(),
    });
    for (i, message) in request.messages.iter().enumerate() {
        let who = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        segments.push(Segment {
            label: format!("message {} ({who})", i + 1),
            role: Some(message.role),
            bytes: message.content.len(),
            est_tokens: estimate_tokens(&message.content),
            content: message.content.clone(),
        });
    }
    RequestInspection {
        provider,
        model: request.model.clone(),
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        segments,
        usage: estimate_usage(request),
    }
}

/// A per-session review gate: the "what will be sent?" inspector must be shown and
/// acknowledged before the first request of a session goes out.
///
/// The default is **review required**. A UI checks [`review_required`], renders
/// [`inspect`] while it holds, and calls [`mark_reviewed`] once the user has seen the
/// payload. Thereafter repeat sends in the same session no longer force the modal — the
/// payload is still available via [`inspect`] on demand.
///
/// [`review_required`]: SessionInspector::review_required
/// [`inspect`]: SessionInspector::inspect
/// [`mark_reviewed`]: SessionInspector::mark_reviewed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionInspector {
    reviewed: bool,
}

impl SessionInspector {
    /// A fresh session: the first payload must be reviewed before sending.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the payload must be shown for review before the next send.
    #[must_use]
    pub fn review_required(&self) -> bool {
        !self.reviewed
    }

    /// Builds the inspection to show the user (available whether or not a review is
    /// currently forced, so the payload can always be re-examined).
    #[must_use]
    pub fn inspect(&self, request: &ChatRequest, provider: ProviderKind) -> RequestInspection {
        inspect_request(request, provider)
    }

    /// Records that the user has seen and acknowledged the payload for this session,
    /// so later sends no longer force the review.
    pub fn mark_reviewed(&mut self) {
        self.reviewed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatRequest, Message, ProviderConfig};

    fn request() -> ChatRequest {
        let config = ProviderConfig {
            model: "claude-sonnet-5".to_owned(),
            max_tokens: 512,
            temperature: 0.2,
            ..Default::default()
        };
        ChatRequest::new(
            &config,
            "you are a synth assistant",
            vec![
                Message::user("make a bass"),
                Message::assistant("```ron\n(nodes: [])\n```"),
                Message::user("add a filter"),
            ],
        )
    }

    #[test]
    fn it_splits_the_payload_into_system_plus_one_segment_per_turn() {
        let inspection = inspect_request(&request(), ProviderKind::Anthropic);
        // 1 system + 3 messages.
        assert_eq!(inspection.segments.len(), 4);
        assert_eq!(inspection.segments[0].label, "system prompt");
        assert_eq!(inspection.segments[0].role, None);
        assert_eq!(inspection.segments[1].label, "message 1 (user)");
        assert_eq!(inspection.segments[1].role, Some(Role::User));
        assert_eq!(inspection.segments[2].label, "message 2 (assistant)");
        assert_eq!(inspection.segments[2].role, Some(Role::Assistant));
        assert_eq!(inspection.turn_count(), 3);
    }

    #[test]
    fn segment_sizes_are_the_content_bytes_and_token_estimate() {
        let inspection = inspect_request(&request(), ProviderKind::Anthropic);
        let system = &inspection.segments[0];
        assert_eq!(system.bytes, "you are a synth assistant".len());
        assert_eq!(
            system.est_tokens,
            estimate_tokens("you are a synth assistant")
        );
        assert_eq!(system.content, "you are a synth assistant");
    }

    #[test]
    fn totals_match_the_budget_estimator() {
        let req = request();
        let inspection = inspect_request(&req, ProviderKind::Anthropic);
        // Usage is the same estimate the budget guard shows.
        assert_eq!(inspection.usage, estimate_usage(&req));
        // total_bytes sums every segment.
        let expected: usize = std::iter::once(req.system.len())
            .chain(req.messages.iter().map(|m| m.content.len()))
            .sum();
        assert_eq!(inspection.total_bytes(), expected);
    }

    #[test]
    fn it_carries_the_destination_and_sampling_settings() {
        let inspection = inspect_request(&request(), ProviderKind::OpenAiCompatible);
        assert_eq!(inspection.provider, ProviderKind::OpenAiCompatible);
        assert_eq!(inspection.model, "claude-sonnet-5");
        assert_eq!(inspection.max_tokens, 512);
        assert!((inspection.temperature - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn render_shows_destination_estimate_and_every_segment() {
        let text = inspect_request(&request(), ProviderKind::Anthropic).render();
        assert!(text.contains("what will be sent to anthropic"));
        assert!(text.contains("claude-sonnet-5"));
        assert!(text.contains("prompt tokens"));
        assert!(text.contains("bytes"));
        // Every segment's content is present.
        assert!(text.contains("you are a synth assistant"));
        assert!(text.contains("make a bass"));
        assert!(text.contains("add a filter"));
        assert!(text.contains("message 3 (user)"));
    }

    #[test]
    fn a_request_with_no_turns_still_inspects_the_system_prompt() {
        let req = ChatRequest::new(&ProviderConfig::default(), "just a system prompt", vec![]);
        let inspection = inspect_request(&req, ProviderKind::Anthropic);
        assert_eq!(inspection.segments.len(), 1);
        assert_eq!(inspection.turn_count(), 0);
        assert_eq!(inspection.total_bytes(), "just a system prompt".len());
    }

    #[test]
    fn a_new_session_requires_review_before_the_first_send() {
        let inspector = SessionInspector::new();
        assert!(
            inspector.review_required(),
            "first request must be reviewed"
        );
    }

    #[test]
    fn acknowledging_clears_the_review_for_later_sends() {
        let mut inspector = SessionInspector::new();
        inspector.mark_reviewed();
        assert!(
            !inspector.review_required(),
            "once reviewed, repeat sends no longer force the modal"
        );
    }

    #[test]
    fn the_payload_stays_inspectable_after_acknowledgement() {
        let mut inspector = SessionInspector::new();
        inspector.mark_reviewed();
        // inspect() works regardless of whether a review is currently forced.
        let inspection = inspector.inspect(&request(), ProviderKind::Anthropic);
        assert_eq!(inspection.segments.len(), 4);
    }
}
