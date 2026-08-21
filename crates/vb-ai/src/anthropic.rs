//! The Anthropic (Claude) provider adapter — the reference [`AiProvider`] (backlog 000113).
//!
//! Speaks the Anthropic Messages API (`POST /v1/messages`). The interesting, fragile parts
//! — turning a [`ChatRequest`] into the wire JSON and a raw reply back into a
//! [`ChatResponse`] — are pure functions ([`build_request_body`], [`parse_response`]),
//! always compiled and exhaustively unit-tested with no network. [`AnthropicProvider`]
//! wires them to a [`HttpTransport`]: it reads the BYOK key from a [`KeyStore`] at call
//! time (never stored here), sets the auth headers, posts, and parses.
//!
//! The key is looked up per call and the real socket lives in
//! [`ReqwestTransport`](crate::http::ReqwestTransport) behind the `net` feature, so nothing
//! leaves the machine unless a caller supplies both a key and a networked transport. Wrap
//! this in [`GuardedProvider`](crate::privacy::GuardedProvider) to enforce the offline /
//! consent policy structurally.

use crate::http::{Header, HttpTransport};
use crate::keystore::KeyStore;
use crate::provider::{AiError, AiProvider, ChatRequest, ChatResponse, ProviderKind, Role};

/// The Messages API endpoint.
pub const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
/// The API version header value the Messages API expects.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Serialises a [`ChatRequest`] into the Anthropic Messages API request body.
///
/// The system prompt travels in the top-level `system` field (not as a message), and roles
/// are lower-cased to the wire vocabulary (`user` / `assistant`).
#[must_use]
pub fn build_request_body(req: &ChatRequest) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": role_wire(m.role),
                "content": m.content,
            })
        })
        .collect();
    serde_json::json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "temperature": req.temperature,
        "system": req.system,
        "messages": messages,
    })
}

/// The wire spelling of a role.
fn role_wire(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Interprets a raw Messages API reply.
///
/// A non-2xx status becomes [`AiError::Api`] carrying the `error.message` from the body
/// when present (else the raw body). On success the text of every `text` content block is
/// concatenated; an empty result is [`AiError::NoContent`], and a body that will not parse
/// is [`AiError::Decode`].
///
/// # Errors
/// [`AiError::Api`], [`AiError::Decode`], or [`AiError::NoContent`] per the rules above.
pub fn parse_response(status: u16, body: &str) -> Result<ChatResponse, AiError> {
    if !(200..300).contains(&status) {
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("HTTP {status}: {body}"));
        return Err(AiError::Api(message));
    }

    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| AiError::Decode(e.to_string()))?;

    let text: String = value
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(serde_json::Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default();

    if text.is_empty() {
        return Err(AiError::NoContent);
    }

    let stop_reason = value
        .get("stop_reason")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(ChatResponse {
        content: text,
        stop_reason,
    })
}

/// The Anthropic adapter: a [`HttpTransport`] plus a [`KeyStore`] to read the BYOK key from.
pub struct AnthropicProvider<T, K> {
    transport: T,
    keys: K,
    url: String,
}

impl<T, K> AnthropicProvider<T, K>
where
    T: HttpTransport,
    K: KeyStore,
{
    /// A provider posting to the public Anthropic endpoint.
    pub fn new(transport: T, keys: K) -> Self {
        Self {
            transport,
            keys,
            url: ANTHROPIC_URL.to_owned(),
        }
    }

    /// Overrides the endpoint (a corporate proxy, a gateway, or a test server).
    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }
}

impl<T, K> AiProvider for AnthropicProvider<T, K>
where
    T: HttpTransport,
    K: KeyStore,
{
    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError> {
        // Read the key per call; a key-store backend failure is surfaced, a missing key is
        // MissingKey — and in both cases nothing is sent.
        let key = self
            .keys
            .get_key(ProviderKind::Anthropic)
            .map_err(|e| AiError::Api(e.to_string()))?
            .ok_or(AiError::MissingKey)?;

        let body = build_request_body(request).to_string();
        let headers: [Header<'_>; 3] = [
            ("x-api-key", key.as_str()),
            ("anthropic-version", ANTHROPIC_VERSION),
            ("content-type", "application/json"),
        ];
        let reply = self.transport.post_json(&self.url, &headers, &body)?;
        parse_response(reply.status, &reply.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::mock::MockTransport;
    use crate::keystore::MemoryKeyStore;
    use crate::provider::{Message, ProviderConfig};

    fn keyed() -> MemoryKeyStore {
        let mut store = MemoryKeyStore::new();
        store
            .set_key(ProviderKind::Anthropic, "sk-test-123")
            .unwrap();
        store
    }

    fn request() -> ChatRequest {
        let config = ProviderConfig {
            model: "claude-sonnet-5".to_owned(),
            max_tokens: 512,
            temperature: 0.0,
            ..Default::default()
        };
        ChatRequest::new(
            &config,
            "you are helpful",
            vec![
                Message::user("hi"),
                Message::assistant("hello"),
                Message::user("more"),
            ],
        )
    }

    #[test]
    fn request_body_matches_the_messages_api_shape() {
        let body = build_request_body(&request());
        assert_eq!(body["model"], "claude-sonnet-5");
        assert_eq!(body["max_tokens"], 512);
        assert_eq!(body["temperature"], 0.0);
        // System prompt is a top-level field, not a message.
        assert_eq!(body["system"], "you are helpful");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hi");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");
    }

    #[test]
    fn parse_extracts_and_concatenates_text_blocks() {
        let body = r#"{
            "id": "msg_1",
            "content": [
                {"type": "text", "text": "Hello, "},
                {"type": "text", "text": "world"}
            ],
            "stop_reason": "end_turn"
        }"#;
        let resp = parse_response(200, body).unwrap();
        assert_eq!(resp.content, "Hello, world");
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn parse_ignores_non_text_blocks() {
        let body = r#"{"content":[
            {"type":"thinking","text":"scratch"},
            {"type":"text","text":"answer"}
        ]}"#;
        let resp = parse_response(200, body).unwrap();
        assert_eq!(resp.content, "answer");
        assert_eq!(resp.stop_reason, None);
    }

    #[test]
    fn parse_maps_error_status_to_api_error_with_the_providers_message() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert_eq!(
            parse_response(401, body),
            Err(AiError::Api("invalid x-api-key".to_owned()))
        );
    }

    #[test]
    fn parse_falls_back_to_raw_body_on_unstructured_error() {
        let err = parse_response(500, "upstream boom").unwrap_err();
        match err {
            AiError::Api(msg) => {
                assert!(msg.contains("500"));
                assert!(msg.contains("upstream boom"));
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn parse_reports_empty_content_as_no_content() {
        assert_eq!(
            parse_response(200, r#"{"content":[]}"#),
            Err(AiError::NoContent)
        );
    }

    #[test]
    fn parse_reports_undecodable_success_body_as_decode() {
        match parse_response(200, "not json at all") {
            Err(AiError::Decode(_)) => {}
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn complete_posts_to_the_endpoint_with_auth_headers_and_body() {
        let transport = MockTransport::replying(
            200,
            r#"{"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#,
        );
        let provider = AnthropicProvider::new(transport, keyed());

        let resp = provider.complete(&request()).unwrap();
        assert_eq!(resp.content, "ok");

        // Inspect what the transport was handed.
        let AnthropicProvider { transport, .. } = provider;
        let sent = transport.only_request();
        assert_eq!(sent.url, ANTHROPIC_URL);
        assert!(sent
            .headers
            .contains(&("x-api-key".to_owned(), "sk-test-123".to_owned())));
        assert!(sent
            .headers
            .contains(&("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned())));
        // The body is the serialised request.
        let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
        assert_eq!(body["model"], "claude-sonnet-5");
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn complete_without_a_key_is_missing_key_and_sends_nothing() {
        let transport = MockTransport::replying(200, "{}");
        let provider = AnthropicProvider::new(transport, MemoryKeyStore::new());

        assert_eq!(provider.complete(&request()), Err(AiError::MissingKey));

        let AnthropicProvider { transport, .. } = provider;
        assert_eq!(
            transport.call_count(),
            0,
            "no request may leave without a key"
        );
    }

    #[test]
    fn complete_surfaces_a_transport_failure_as_network() {
        let transport = MockTransport::failing("connection refused");
        let provider = AnthropicProvider::new(transport, keyed());
        match provider.complete(&request()) {
            Err(AiError::Network(msg)) => assert!(msg.contains("connection refused")),
            other => panic!("expected Network, got {other:?}"),
        }
    }

    #[test]
    fn complete_reports_its_provider_kind() {
        let provider = AnthropicProvider::new(MockTransport::replying(200, "{}"), keyed());
        assert_eq!(provider.kind(), ProviderKind::Anthropic);
    }

    #[test]
    fn with_url_overrides_the_endpoint() {
        let transport =
            MockTransport::replying(200, r#"{"content":[{"type":"text","text":"ok"}]}"#);
        let provider =
            AnthropicProvider::new(transport, keyed()).with_url("http://localhost:9/v1/messages");
        provider.complete(&request()).unwrap();
        let AnthropicProvider { transport, .. } = provider;
        assert_eq!(
            transport.only_request().url,
            "http://localhost:9/v1/messages"
        );
    }
}
