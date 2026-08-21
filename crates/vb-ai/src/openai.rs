//! The OpenAI-compatible provider adapter (backlog 000114).
//!
//! One adapter for the whole family of endpoints that speak the OpenAI Chat Completions
//! API (`POST /v1/chat/completions`) — OpenAI itself, and the many gateways and local
//! servers (Azure OpenAI, Together, Groq, OpenRouter, llama.cpp's server, LM Studio, …)
//! that copy its shape. That is why the [`ProviderKind`] is `OpenAiCompatible` and the
//! endpoint is configurable via [`with_url`](OpenAiProvider::with_url).
//!
//! Same structure as the Anthropic adapter ([`crate::anthropic`]): the wire codec
//! ([`build_request_body`], [`parse_response`]) is pure, always compiled, and unit-tested
//! with no network; [`OpenAiProvider`] wires it to a [`HttpTransport`] and reads the BYOK
//! key from a [`KeyStore`] at call time. The two schemas differ in two places, handled
//! here: the system prompt is the **first message** with role `system` (Anthropic carries
//! it in a top-level field), and the reply text is `choices[0].message.content` (Anthropic
//! returns an array of content blocks).

use crate::http::{Header, HttpTransport};
use crate::keystore::KeyStore;
use crate::provider::{AiError, AiProvider, ChatRequest, ChatResponse, ProviderKind, Role};

/// The default Chat Completions endpoint (OpenAI's own). Override for any other
/// OpenAI-compatible host with [`OpenAiProvider::with_url`].
pub const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Serialises a [`ChatRequest`] into the Chat Completions request body.
///
/// The system prompt, when non-empty, is prepended as the first message with role
/// `system`; conversation turns follow with roles `user` / `assistant`.
#[must_use]
pub fn build_request_body(req: &ChatRequest) -> serde_json::Value {
    let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if !req.system.is_empty() {
        messages.push(serde_json::json!({ "role": "system", "content": req.system }));
    }
    for m in &req.messages {
        messages.push(serde_json::json!({
            "role": role_wire(m.role),
            "content": m.content,
        }));
    }
    serde_json::json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "temperature": req.temperature,
        "messages": messages,
    })
}

/// The wire spelling of a conversation role.
fn role_wire(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Interprets a raw Chat Completions reply.
///
/// A non-2xx status becomes [`AiError::Api`] carrying `error.message` when present (else
/// the raw body). On success the assistant text is `choices[0].message.content`; a missing
/// or empty content is [`AiError::NoContent`], and a body that will not parse is
/// [`AiError::Decode`].
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

    let first = value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|c| c.first());

    let text = first
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if text.is_empty() {
        return Err(AiError::NoContent);
    }

    let stop_reason = first
        .and_then(|c| c.get("finish_reason"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(ChatResponse {
        content: text.to_owned(),
        stop_reason,
    })
}

/// The OpenAI-compatible adapter: a [`HttpTransport`] plus a [`KeyStore`] for the BYOK key.
pub struct OpenAiProvider<T, K> {
    transport: T,
    keys: K,
    url: String,
}

impl<T, K> OpenAiProvider<T, K>
where
    T: HttpTransport,
    K: KeyStore,
{
    /// A provider posting to OpenAI's public Chat Completions endpoint.
    pub fn new(transport: T, keys: K) -> Self {
        Self {
            transport,
            keys,
            url: OPENAI_URL.to_owned(),
        }
    }

    /// Points the adapter at another OpenAI-compatible endpoint (Azure, a gateway, a local
    /// server). Give the full URL including the path.
    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }
}

impl<T, K> AiProvider for OpenAiProvider<T, K>
where
    T: HttpTransport,
    K: KeyStore,
{
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AiError> {
        // Read the key per call; missing → MissingKey, and nothing is sent in that case.
        let key = self
            .keys
            .get_key(ProviderKind::OpenAiCompatible)
            .map_err(|e| AiError::Api(e.to_string()))?
            .ok_or(AiError::MissingKey)?;

        let body = build_request_body(request).to_string();
        let bearer = format!("Bearer {key}");
        let headers: [Header<'_>; 2] = [
            ("authorization", bearer.as_str()),
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
            .set_key(ProviderKind::OpenAiCompatible, "sk-oai-9")
            .unwrap();
        store
    }

    fn request_with(system: &str) -> ChatRequest {
        let config = ProviderConfig {
            provider: ProviderKind::OpenAiCompatible,
            model: "gpt-4o-mini".to_owned(),
            max_tokens: 256,
            temperature: 0.5,
        };
        ChatRequest::new(
            &config,
            system,
            vec![
                Message::user("hi"),
                Message::assistant("hey"),
                Message::user("go"),
            ],
        )
    }

    #[test]
    fn request_body_prepends_the_system_message() {
        let body = build_request_body(&request_with("be terse"));
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["max_tokens"], 256);
        assert_eq!(body["temperature"], 0.5);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4, "system + 3 turns");
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "be terse");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[3]["role"], "user");
    }

    #[test]
    fn an_empty_system_prompt_is_omitted() {
        let body = build_request_body(&request_with(""));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3, "no system message when the prompt is empty");
        assert_eq!(msgs[0]["role"], "user");
    }

    #[test]
    fn parse_reads_the_first_choice_message() {
        let body = r#"{
            "choices": [
                {"index": 0, "message": {"role": "assistant", "content": "the answer"},
                 "finish_reason": "stop"}
            ]
        }"#;
        let resp = parse_response(200, body).unwrap();
        assert_eq!(resp.content, "the answer");
        assert_eq!(resp.stop_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parse_maps_error_status_to_api_error_with_the_providers_message() {
        let body =
            r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
        assert_eq!(
            parse_response(401, body),
            Err(AiError::Api("Incorrect API key provided".to_owned()))
        );
    }

    #[test]
    fn parse_falls_back_to_raw_body_on_unstructured_error() {
        match parse_response(503, "service unavailable") {
            Err(AiError::Api(msg)) => {
                assert!(msg.contains("503"));
                assert!(msg.contains("service unavailable"));
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn parse_reports_missing_content_as_no_content() {
        assert_eq!(
            parse_response(200, r#"{"choices":[{"message":{"role":"assistant"}}]}"#),
            Err(AiError::NoContent)
        );
        assert_eq!(
            parse_response(200, r#"{"choices":[]}"#),
            Err(AiError::NoContent)
        );
    }

    #[test]
    fn parse_reports_undecodable_success_body_as_decode() {
        match parse_response(200, "<html>nope</html>") {
            Err(AiError::Decode(_)) => {}
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn complete_posts_with_a_bearer_token_and_body() {
        let transport = MockTransport::replying(
            200,
            r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
        );
        let provider = OpenAiProvider::new(transport, keyed());

        let resp = provider.complete(&request_with("sys")).unwrap();
        assert_eq!(resp.content, "ok");

        let OpenAiProvider { transport, .. } = provider;
        let sent = transport.only_request();
        assert_eq!(sent.url, OPENAI_URL);
        assert!(sent
            .headers
            .contains(&("authorization".to_owned(), "Bearer sk-oai-9".to_owned())));
        let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
        assert_eq!(body["messages"][0]["role"], "system");
    }

    #[test]
    fn complete_without_a_key_is_missing_key_and_sends_nothing() {
        let transport = MockTransport::replying(200, "{}");
        let provider = OpenAiProvider::new(transport, MemoryKeyStore::new());
        assert_eq!(
            provider.complete(&request_with("s")),
            Err(AiError::MissingKey)
        );
        let OpenAiProvider { transport, .. } = provider;
        assert_eq!(transport.call_count(), 0);
    }

    #[test]
    fn complete_surfaces_a_transport_failure_as_network() {
        let transport = MockTransport::failing("dns failure");
        let provider = OpenAiProvider::new(transport, keyed());
        match provider.complete(&request_with("s")) {
            Err(AiError::Network(msg)) => assert!(msg.contains("dns failure")),
            other => panic!("expected Network, got {other:?}"),
        }
    }

    #[test]
    fn kind_is_openai_compatible_and_url_is_overridable() {
        let transport =
            MockTransport::replying(200, r#"{"choices":[{"message":{"content":"x"}}]}"#);
        let provider = OpenAiProvider::new(transport, keyed())
            .with_url("http://localhost:1234/v1/chat/completions");
        assert_eq!(provider.kind(), ProviderKind::OpenAiCompatible);
        provider.complete(&request_with("s")).unwrap();
        let OpenAiProvider { transport, .. } = provider;
        assert_eq!(
            transport.only_request().url,
            "http://localhost:1234/v1/chat/completions"
        );
    }
}
