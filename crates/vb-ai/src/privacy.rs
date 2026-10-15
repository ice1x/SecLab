//! Privacy controls for outgoing assistant requests: an offline switch, an explicit
//! consent gate, and a "what will be sent?" preview.
//!
//! Nothing about a patch leaves the machine implicitly. [`GuardedProvider`] wraps any
//! [`AiProvider`] and refuses to call it unless the [`PrivacySettings`] allow — so the
//! whole assistant stack (generate, edit, explain) inherits the policy structurally,
//! not by remembering to check. The default settings **deny**: consent is required
//! before the first send. [`preview_request`] renders the exact bytes a request would
//! carry, so the user can inspect a payload before granting consent.

use serde::{Deserialize, Serialize};

use crate::provider::{AiError, AiProvider, ChatRequest, ChatResponse, ProviderKind, Role};

/// User privacy settings governing whether a request may be sent.
///
/// Persisted as part of the assistant settings (see [`crate::persist`]); `#[serde(default)]`
/// so an older or partial file still loads to the deny-by-default policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacySettings {
    /// When on, no request may leave the machine at all.
    pub offline: bool,
    /// The user has explicitly consented to sending the project to the provider.
    pub consent_to_send: bool,
}

impl Default for PrivacySettings {
    /// Deny by default: online but without consent, so nothing sends until the user
    /// opts in.
    fn default() -> Self {
        Self {
            offline: false,
            consent_to_send: false,
        }
    }
}

impl PrivacySettings {
    /// The policy reason a send is blocked, or `None` when sending is allowed.
    /// Offline mode is checked first — it overrides consent.
    #[must_use]
    pub fn blocked_reason(&self) -> Option<AiError> {
        if self.offline {
            Some(AiError::OfflineMode)
        } else if !self.consent_to_send {
            Some(AiError::ConsentRequired)
        } else {
            None
        }
    }

    /// Whether a request may currently be sent.
    #[must_use]
    pub fn may_send(&self) -> bool {
        self.blocked_reason().is_none()
    }
}

/// Wraps a provider and enforces [`PrivacySettings`] before any request is forwarded.
pub struct GuardedProvider<'a> {
    inner: &'a dyn AiProvider,
    settings: PrivacySettings,
}

impl<'a> GuardedProvider<'a> {
    /// Guards `inner` with `settings`.
    #[must_use]
    pub fn new(inner: &'a dyn AiProvider, settings: PrivacySettings) -> Self {
        Self { inner, settings }
    }
}

impl AiProvider for GuardedProvider<'_> {
    fn kind(&self) -> ProviderKind {
        self.inner.kind()
    }

    fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError> {
        // Refuse *before* touching the inner provider, so a blocked request never leaves.
        if let Some(reason) = self.settings.blocked_reason() {
            return Err(reason);
        }
        self.inner.complete(request)
    }
}

/// Renders the exact payload a request would send — the system prompt and every
/// conversation turn — so the user can review it before consenting.
#[must_use]
pub fn preview_request(request: &ChatRequest) -> String {
    let mut out = format!(
        "model: {}\nmax_tokens: {}\ntemperature: {}\n\n=== SYSTEM ===\n{}\n",
        request.model, request.max_tokens, request.temperature, request.system
    );
    for message in &request.messages {
        let role = match message.role {
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
        };
        out.push_str(&format!("\n=== {role} ===\n{}\n", message.content));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ChatRequest, Message, ProviderConfig, ScriptedProvider};

    fn request() -> ChatRequest {
        ChatRequest::new(
            &ProviderConfig::default(),
            "you are a synth assistant",
            vec![Message::user("make a bass")],
        )
    }

    #[test]
    fn the_default_policy_requires_consent() {
        let settings = PrivacySettings::default();
        assert!(!settings.may_send());
        assert_eq!(settings.blocked_reason(), Some(AiError::ConsentRequired));
    }

    #[test]
    fn offline_mode_blocks_even_with_consent() {
        let settings = PrivacySettings {
            offline: true,
            consent_to_send: true,
        };
        assert_eq!(settings.blocked_reason(), Some(AiError::OfflineMode));
    }

    #[test]
    fn consent_online_allows_sending() {
        let settings = PrivacySettings {
            offline: false,
            consent_to_send: true,
        };
        assert!(settings.may_send());
        assert_eq!(settings.blocked_reason(), None);
    }

    #[test]
    fn a_blocked_guard_never_calls_the_inner_provider() {
        let inner = ScriptedProvider::replies(&["secret reply"]);
        let guard = GuardedProvider::new(&inner, PrivacySettings::default());

        // Consent not given → refused, and the inner provider is untouched.
        assert_eq!(guard.complete(&request()), Err(AiError::ConsentRequired));
        assert!(inner.requests().is_empty(), "nothing was sent");

        // Offline overrides consent.
        let offline = GuardedProvider::new(
            &inner,
            PrivacySettings {
                offline: true,
                consent_to_send: true,
            },
        );
        assert_eq!(offline.complete(&request()), Err(AiError::OfflineMode));
        assert!(inner.requests().is_empty());
    }

    #[test]
    fn a_permitted_guard_forwards_to_the_inner_provider() {
        let inner = ScriptedProvider::replies(&["hello"]);
        let guard = GuardedProvider::new(
            &inner,
            PrivacySettings {
                offline: false,
                consent_to_send: true,
            },
        );
        assert_eq!(guard.complete(&request()).unwrap().content, "hello");
        assert_eq!(inner.requests().len(), 1, "the request went through");
    }

    #[test]
    fn preview_shows_the_full_payload() {
        let text = preview_request(&request());
        assert!(
            text.contains("you are a synth assistant"),
            "system prompt shown"
        );
        assert!(text.contains("make a bass"), "user turn shown");
        assert!(text.contains("model:"), "model/settings shown");
    }

    #[test]
    fn the_whole_assistant_stack_inherits_the_gate() {
        use crate::generate::{generate_patch, GenerateError};
        use vb_nodes::Registry;

        // A provider that WOULD answer, but the guard blocks it → generate_patch fails
        // with the policy error and the inner provider is never called.
        let inner = ScriptedProvider::replies(&["```ron\n...\n```"]);
        let guard = GuardedProvider::new(&inner, PrivacySettings::default());
        let err = generate_patch(
            &guard,
            &Registry::with_builtins(),
            &ProviderConfig::default(),
            "a pad",
            None,
            3,
        )
        .unwrap_err();
        assert_eq!(err, GenerateError::Provider(AiError::ConsentRequired));
        assert!(inner.requests().is_empty());
    }
}
