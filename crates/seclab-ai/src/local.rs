//! Offline / local model support (backlog 000111).
//!
//! "Local models via the same abstraction" needs no new protocol: the popular local LLM
//! servers — [Ollama](https://ollama.com), [LM Studio](https://lmstudio.ai), and
//! llama.cpp's bundled server — all expose an **OpenAI-compatible** Chat Completions
//! endpoint on `localhost`. So a local model is reached through the exact wire codec the
//! cloud adapter uses ([`crate::openai`]); the only differences are that the request never
//! leaves the machine and that a local server is keyless by default.
//!
//! [`LocalProvider`] captures that: it implements [`AiProvider`] by reusing
//! [`openai::build_request_body`](crate::openai::build_request_body) /
//! [`openai::parse_response`](crate::openai::parse_response), points at a
//! [`LocalBackend`]'s `localhost` URL (or a custom one), and sends **no** `Authorization`
//! header unless a token is explicitly set with [`with_token`](LocalProvider::with_token)
//! (for a secured local gateway). Because everything stays on the machine, this is the
//! privacy-preserving path — no BYOK key, no cloud round-trip.

use crate::http::{Header, HttpTransport};
use crate::openai::{build_request_body, models_url, parse_models_list, parse_response};
use crate::provider::{AiError, AiProvider, ChatRequest, ChatResponse, ProviderKind};

/// A known local LLM server exposing an OpenAI-compatible Chat Completions endpoint.
///
/// The ports are each project's documented default; override the full URL with
/// [`LocalProvider::at`] for a non-standard port or host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalBackend {
    /// Ollama's OpenAI-compatible endpoint (default port 11434).
    Ollama,
    /// LM Studio's local server (default port 1234).
    LmStudio,
    /// llama.cpp's bundled `server` (default port 8080).
    LlamaCpp,
}

impl LocalBackend {
    /// Every known local backend, in display order — for a UI that offers them as presets.
    pub const ALL: [LocalBackend; 3] = [
        LocalBackend::Ollama,
        LocalBackend::LmStudio,
        LocalBackend::LlamaCpp,
    ];

    /// The default Chat Completions URL this backend listens on.
    #[must_use]
    pub fn url(self) -> &'static str {
        match self {
            LocalBackend::Ollama => "http://localhost:11434/v1/chat/completions",
            LocalBackend::LmStudio => "http://localhost:1234/v1/chat/completions",
            LocalBackend::LlamaCpp => "http://localhost:8080/v1/chat/completions",
        }
    }

    /// A short human-readable label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            LocalBackend::Ollama => "ollama",
            LocalBackend::LmStudio => "lm-studio",
            LocalBackend::LlamaCpp => "llama.cpp",
        }
    }
}

/// An [`AiProvider`] talking to a local, OpenAI-compatible server — offline, keyless by
/// default. Speaks the same wire protocol as [`OpenAiProvider`](crate::openai::OpenAiProvider)
/// but requires no [`KeyStore`](crate::keystore::KeyStore), since a local server needs no
/// cloud credential.
pub struct LocalProvider<T> {
    transport: T,
    url: String,
    token: Option<String>,
}

impl<T> LocalProvider<T>
where
    T: HttpTransport,
{
    /// A provider pointed at a known [`LocalBackend`]'s default endpoint.
    pub fn new(transport: T, backend: LocalBackend) -> Self {
        Self {
            transport,
            url: backend.url().to_owned(),
            token: None,
        }
    }

    /// A provider pointed at a custom OpenAI-compatible URL (non-standard port or host).
    pub fn at(transport: T, url: impl Into<String>) -> Self {
        Self {
            transport,
            url: url.into(),
            token: None,
        }
    }

    /// Sets a bearer token for a local gateway that requires one. Omit for the usual
    /// keyless local server.
    #[must_use]
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Lists the models the local server currently has installed.
    ///
    /// Queries the server's `GET /v1/models` endpoint — derived from the configured chat
    /// URL via [`models_url`] — and returns each model identifier in server order. This
    /// lets the UI offer the installed models (e.g. `qwen2.5-coder:14b`) as a picklist
    /// instead of making the user type the name. Keyless by default, like
    /// [`complete`](LocalProvider::complete); a token set with
    /// [`with_token`](LocalProvider::with_token) is sent as a bearer for a secured gateway.
    ///
    /// # Errors
    /// [`AiError::Network`] if the server is unreachable, [`AiError::Api`] for a non-2xx
    /// reply, or [`AiError::Decode`] for a body that will not parse.
    pub fn list_models(&self) -> Result<Vec<String>, AiError> {
        let url = models_url(&self.url);
        let bearer = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut headers: Vec<Header<'_>> = Vec::with_capacity(1);
        if let Some(b) = &bearer {
            headers.push(("authorization", b.as_str()));
        }
        let reply = self.transport.get(&url, &headers)?;
        parse_models_list(reply.status, &reply.body)
    }
}

impl<T> AiProvider for LocalProvider<T>
where
    T: HttpTransport,
{
    fn kind(&self) -> ProviderKind {
        // It speaks the OpenAI-compatible protocol.
        ProviderKind::OpenAiCompatible
    }

    fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError> {
        let body = build_request_body(request).to_string();
        // A local server is keyless by default; only attach auth when a token was set.
        let bearer = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut headers: Vec<Header<'_>> = Vec::with_capacity(2);
        if let Some(b) = &bearer {
            headers.push(("authorization", b.as_str()));
        }
        headers.push(("content-type", "application/json"));
        let reply = self.transport.post_json(&self.url, &headers, &body)?;
        parse_response(reply.status, &reply.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::mock::MockTransport;
    use crate::provider::{Message, ProviderConfig};

    fn request() -> ChatRequest {
        let config = ProviderConfig {
            provider: ProviderKind::OpenAiCompatible,
            model: "llama3.1".to_owned(),
            max_tokens: 128,
            temperature: 0.0,
        };
        ChatRequest::new(&config, "be helpful", vec![Message::user("ping")])
    }

    fn ok_reply() -> MockTransport {
        MockTransport::replying(
            200,
            r#"{"choices":[{"message":{"content":"pong"},"finish_reason":"stop"}]}"#,
        )
    }

    #[test]
    fn backends_expose_their_default_localhost_endpoints() {
        assert_eq!(
            LocalBackend::Ollama.url(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            LocalBackend::LmStudio.url(),
            "http://localhost:1234/v1/chat/completions"
        );
        assert_eq!(
            LocalBackend::LlamaCpp.url(),
            "http://localhost:8080/v1/chat/completions"
        );
        assert_eq!(LocalBackend::Ollama.label(), "ollama");
    }

    #[test]
    fn all_lists_every_backend_with_distinct_urls_and_labels() {
        let all = LocalBackend::ALL;
        assert_eq!(all.len(), 3);
        assert!(all.contains(&LocalBackend::Ollama));
        assert!(all.contains(&LocalBackend::LmStudio));
        assert!(all.contains(&LocalBackend::LlamaCpp));
        // Each preset points somewhere distinct and has its own label.
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.url(), b.url(), "{a:?} and {b:?} share a URL");
                assert_ne!(a.label(), b.label(), "{a:?} and {b:?} share a label");
            }
        }
    }

    #[test]
    fn kind_is_openai_compatible() {
        let provider = LocalProvider::new(ok_reply(), LocalBackend::Ollama);
        assert_eq!(provider.kind(), ProviderKind::OpenAiCompatible);
    }

    #[test]
    fn keyless_by_default_sends_no_authorization_header() {
        let provider = LocalProvider::new(ok_reply(), LocalBackend::Ollama);
        let resp = provider.complete(&request()).unwrap();
        assert_eq!(resp.content, "pong");

        let LocalProvider { transport, .. } = provider;
        let sent = transport.only_request();
        assert_eq!(sent.url, "http://localhost:11434/v1/chat/completions");
        assert!(
            !sent.headers.iter().any(|(k, _)| k == "authorization"),
            "a keyless local call must carry no Authorization header"
        );
        // Reuses the OpenAI codec: the system prompt is the leading `system` message.
        let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "be helpful");
    }

    #[test]
    fn with_token_attaches_a_bearer_for_a_secured_gateway() {
        let provider =
            LocalProvider::new(ok_reply(), LocalBackend::LmStudio).with_token("local-secret");
        provider.complete(&request()).unwrap();
        let LocalProvider { transport, .. } = provider;
        let sent = transport.only_request();
        assert!(sent
            .headers
            .contains(&("authorization".to_owned(), "Bearer local-secret".to_owned())));
    }

    #[test]
    fn at_targets_a_custom_url() {
        let provider = LocalProvider::at(ok_reply(), "http://192.168.1.5:9000/v1/chat/completions");
        provider.complete(&request()).unwrap();
        let LocalProvider { transport, .. } = provider;
        assert_eq!(
            transport.only_request().url,
            "http://192.168.1.5:9000/v1/chat/completions"
        );
    }

    #[test]
    fn a_dead_local_server_surfaces_as_network() {
        // Nothing listening: the transport fails to connect. Offline and honest.
        let provider = LocalProvider::new(
            MockTransport::failing("connection refused"),
            LocalBackend::LlamaCpp,
        );
        match provider.complete(&request()) {
            Err(AiError::Network(msg)) => assert!(msg.contains("connection refused")),
            other => panic!("expected Network, got {other:?}"),
        }
    }

    #[test]
    fn list_models_gets_v1_models_and_returns_the_ids() {
        let provider = LocalProvider::new(
            MockTransport::replying(
                200,
                r#"{"object":"list","data":[{"id":"qwen2.5-coder:14b"},{"id":"llama3.1:8b"}]}"#,
            ),
            LocalBackend::Ollama,
        );
        let models = provider.list_models().unwrap();
        assert_eq!(models, vec!["qwen2.5-coder:14b", "llama3.1:8b"]);

        let LocalProvider { transport, .. } = provider;
        let sent = transport.only_request();
        // The listing endpoint is the chat URL's sibling, and keyless by default.
        assert_eq!(sent.url, "http://localhost:11434/v1/models");
        assert!(
            !sent.headers.iter().any(|(k, _)| k == "authorization"),
            "a keyless local listing must carry no Authorization header"
        );
    }

    #[test]
    fn list_models_attaches_a_bearer_for_a_secured_gateway() {
        let provider = LocalProvider::new(ok_reply(), LocalBackend::LmStudio).with_token("tok");
        // ok_reply() returns a chat-completion body, not a models list, so the parse
        // yields an empty catalog — the point here is the request headers/url.
        let _ = provider.list_models();
        let LocalProvider { transport, .. } = provider;
        let sent = transport.only_request();
        assert_eq!(sent.url, "http://localhost:1234/v1/models");
        assert!(sent
            .headers
            .contains(&("authorization".to_owned(), "Bearer tok".to_owned())));
    }

    #[test]
    fn list_models_on_a_dead_server_surfaces_as_network() {
        let provider = LocalProvider::new(
            MockTransport::failing("connection refused"),
            LocalBackend::LlamaCpp,
        );
        match provider.list_models() {
            Err(AiError::Network(msg)) => assert!(msg.contains("connection refused")),
            other => panic!("expected Network, got {other:?}"),
        }
    }

    #[test]
    fn a_backend_error_status_is_surfaced() {
        let provider = LocalProvider::new(
            MockTransport::replying(404, r#"{"error":{"message":"model not found"}}"#),
            LocalBackend::Ollama,
        );
        assert_eq!(
            provider.complete(&request()),
            Err(AiError::Api("model not found".to_owned()))
        );
    }
}
