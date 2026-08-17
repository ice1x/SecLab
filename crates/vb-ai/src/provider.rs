//! The LLM provider abstraction: one trait every backend implements, plus the request /
//! response types and the model/provider configuration.
//!
//! This layer is deliberately transport-free — no HTTP, no async runtime, no keys. It
//! defines *what* a completion is so the assistant loop (epics 12) can be written and
//! tested against a mock, while concrete network adapters (Anthropic — 000113, an
//! OpenAI-compatible endpoint — 000114) implement [`AiProvider`] behind the same shape.
//!
//! **Keys never live here.** BYOK credentials are read from the OS keychain by the
//! concrete adapter at call time (000112); [`ProviderConfig`] carries only the choice of
//! provider and model and the sampling knobs.

use serde::{Deserialize, Serialize};

/// Default model — Anthropic's Claude, the reference provider. Overridable in config.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

/// Default response budget in tokens.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Which backend family a provider speaks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ProviderKind {
    /// Anthropic Claude (the reference implementation, 000113).
    #[default]
    Anthropic,
    /// Any OpenAI-compatible chat-completions endpoint (000114).
    OpenAiCompatible,
}

impl ProviderKind {
    /// Short human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAiCompatible => "openai-compatible",
        }
    }
}

/// Who authored a message in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// A turn from the human.
    User,
    /// A turn from the assistant.
    Assistant,
}

/// One conversation turn (the system prompt travels separately on [`ChatRequest`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who wrote it.
    pub role: Role,
    /// The text content.
    pub content: String,
}

impl Message {
    /// A user turn.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An assistant turn.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Provider selection and sampling parameters. **No API key** — see the module docs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Which backend family to call.
    pub provider: ProviderKind,
    /// The model identifier passed to that backend.
    pub model: String,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature (0 = deterministic).
    pub temperature: f32,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Anthropic,
            model: DEFAULT_MODEL.to_owned(),
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: 1.0,
        }
    }
}

/// A single completion request: the system prompt, the conversation so far, and the
/// model/sampling settings resolved from a [`ProviderConfig`].
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// The full system prompt (see [`crate::build_system_prompt`]).
    pub system: String,
    /// Conversation turns, oldest first.
    pub messages: Vec<Message>,
    /// Model identifier.
    pub model: String,
    /// Token budget for the reply.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
}

impl ChatRequest {
    /// Builds a request for `config` with the given system prompt and conversation.
    #[must_use]
    pub fn new(config: &ProviderConfig, system: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            system: system.into(),
            messages,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
        }
    }
}

/// A completion reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatResponse {
    /// The assistant's text.
    pub content: String,
    /// Why generation stopped, if the backend reports it (e.g. `"end_turn"`).
    pub stop_reason: Option<String>,
}

impl ChatResponse {
    /// A response carrying only text.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            stop_reason: None,
        }
    }
}

/// Why a completion failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// No credential was available for the provider (BYOK not configured).
    MissingKey,
    /// The transport failed (connection, timeout, TLS…).
    Network(String),
    /// The backend returned an error status/body.
    Api(String),
    /// The response could not be decoded into the expected shape.
    Decode(String),
    /// The backend returned no usable content.
    NoContent,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::MissingKey => write!(f, "no API key configured for this provider"),
            AiError::Network(e) => write!(f, "network error: {e}"),
            AiError::Api(e) => write!(f, "provider API error: {e}"),
            AiError::Decode(e) => write!(f, "could not decode the response: {e}"),
            AiError::NoContent => write!(f, "the provider returned no content"),
        }
    }
}

impl std::error::Error for AiError {}

/// A large-language-model backend that turns a [`ChatRequest`] into a [`ChatResponse`].
///
/// Implementations are transport-specific (an HTTP adapter, a local model, or a mock).
/// The call is synchronous: a networked adapter blocks or drives its own runtime
/// internally, keeping this trait — and the assistant loop built on it — free of an
/// async dependency.
pub trait AiProvider {
    /// Which backend family this is.
    fn kind(&self) -> ProviderKind;

    /// Runs one completion.
    ///
    /// # Errors
    /// Any [`AiError`] the backend surfaces (missing key, network, API, decode…).
    fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError>;
}

/// A test/offline provider that replays a fixed script of replies in order, recording
/// the requests it was given. Lets the assistant loop be exercised without a network.
#[derive(Default)]
pub struct ScriptedProvider {
    replies: std::cell::RefCell<std::collections::VecDeque<Result<ChatResponse, AiError>>>,
    seen: std::cell::RefCell<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    /// Creates a provider that will return `replies` in order, one per [`complete`] call.
    ///
    /// [`complete`]: AiProvider::complete
    #[must_use]
    pub fn new(replies: Vec<Result<ChatResponse, AiError>>) -> Self {
        Self {
            replies: std::cell::RefCell::new(replies.into()),
            seen: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Convenience: a provider that returns each string as a plain text response.
    #[must_use]
    pub fn replies(texts: &[&str]) -> Self {
        Self::new(texts.iter().map(|t| Ok(ChatResponse::text(*t))).collect())
    }

    /// The requests seen so far, oldest first.
    #[must_use]
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.seen.borrow().clone()
    }
}

impl AiProvider for ScriptedProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError> {
        self.seen.borrow_mut().push(request.clone());
        self.replies
            .borrow_mut()
            .pop_front()
            .unwrap_or(Err(AiError::NoContent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_claude() {
        let c = ProviderConfig::default();
        assert_eq!(c.provider, ProviderKind::Anthropic);
        assert_eq!(c.model, DEFAULT_MODEL);
        assert_eq!(c.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(ProviderKind::default(), ProviderKind::Anthropic);
        assert_eq!(ProviderKind::Anthropic.label(), "anthropic");
    }

    #[test]
    fn a_request_carries_the_configs_model_and_sampling() {
        let config = ProviderConfig {
            model: "claude-opus-5".to_owned(),
            max_tokens: 1000,
            temperature: 0.2,
            ..Default::default()
        };
        let req = ChatRequest::new(&config, "sys", vec![Message::user("hi")]);
        assert_eq!(req.model, "claude-opus-5");
        assert_eq!(req.max_tokens, 1000);
        assert!((req.temperature - 0.2).abs() < f32::EPSILON);
        assert_eq!(req.system, "sys");
        assert_eq!(req.messages, vec![Message::user("hi")]);
    }

    #[test]
    fn scripted_provider_replays_in_order_and_records_requests() {
        let provider = ScriptedProvider::replies(&["first", "second"]);
        let config = ProviderConfig::default();

        let r1 = provider
            .complete(&ChatRequest::new(&config, "sys", vec![Message::user("a")]))
            .unwrap();
        assert_eq!(r1.content, "first");
        let r2 = provider
            .complete(&ChatRequest::new(&config, "sys", vec![Message::user("b")]))
            .unwrap();
        assert_eq!(r2.content, "second");

        // Exhausted script → NoContent.
        assert_eq!(
            provider.complete(&ChatRequest::new(&config, "sys", vec![])),
            Err(AiError::NoContent)
        );

        // It recorded every request it was handed.
        let seen = provider.requests();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0].messages[0].content, "a");
        assert_eq!(seen[1].messages[0].content, "b");
    }

    #[test]
    fn scripted_provider_surfaces_scripted_errors() {
        let provider = ScriptedProvider::new(vec![Err(AiError::MissingKey)]);
        let config = ProviderConfig::default();
        assert_eq!(
            provider.complete(&ChatRequest::new(&config, "s", vec![])),
            Err(AiError::MissingKey)
        );
    }

    #[test]
    fn config_round_trips_as_json_free_ron() {
        // Config persists to settings; make sure it serializes/parses.
        let config = ProviderConfig::default();
        let text = ron::to_string(&config).expect("serialize");
        let back: ProviderConfig = ron::from_str(&text).expect("parse");
        assert_eq!(back, config);
    }

    #[test]
    fn ai_error_messages_are_distinct() {
        assert_ne!(
            AiError::MissingKey.to_string(),
            AiError::NoContent.to_string()
        );
        assert!(AiError::Network("x".into()).to_string().contains('x'));
    }
}
