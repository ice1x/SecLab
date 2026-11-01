//! GitHub OAuth **device flow** — the sign-in the in-app bug reporter gates on (000250).
//!
//! The "Report a problem / feature request" button may only submit for an authenticated
//! user (no account → no report). Rather than build an account system, SecLab signs the
//! user in with their existing GitHub account via the [OAuth device flow] — the same flow
//! `gh auth login` uses: the app shows a short code, the user enters it at
//! <https://github.com/login/device> in a browser, and the app then polls for the user's
//! own access token. The flow needs only a **public** `client_id` (no client secret), so
//! nothing secret ships in the binary; the resulting token belongs to the user and
//! authorizes creating the report issue as themselves.
//!
//! This module is the transport-agnostic core: [`DeviceFlow::start`] requests a device
//! code and [`DeviceFlow::poll_once`] performs one poll of the token endpoint, classifying
//! GitHub's reply into a [`PollOutcome`]. It drives the shared [`HttpTransport`] seam
//! (see [`crate::http`]), so every path is unit-tested against a mock that replays canned
//! JSON with zero requests leaving the machine; only `--features net` links a real socket.
//! Token storage (keychain) and issue creation build on top of this in follow-up tasks.
//!
//! [OAuth device flow]: https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow

use serde::Deserialize;

use crate::http::{Header, HttpTransport};
use crate::keystore::KeyStoreError;

/// GitHub's device-code request endpoint (step 1 of the device flow).
pub const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
/// GitHub's access-token polling endpoint (step 3 of the device flow).
pub const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
/// The `grant_type` the token poll must carry for the device flow.
pub const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The OAuth scope the bug reporter requests: create issues on the public SecLab repo.
///
/// `public_repo` is the least privilege that still lets the user's token open an issue on a
/// public repository; it grants no access to private repos.
pub const REPORT_SCOPE: &str = "public_repo";

/// Why a device-flow step could not be completed.
///
/// A *fatal* OAuth error (`access_denied`, `expired_token`) is **not** an `AuthError` — it
/// is a normal [`PollOutcome`] the caller acts on. `AuthError` is reserved for a request
/// that could not be understood at all: a dead transport, an unexpected HTTP status, an
/// unparseable body, or an unrecognized OAuth error code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The HTTP transport failed (connection, timeout, TLS).
    Transport(String),
    /// GitHub returned an unexpected HTTP status.
    Status(u16),
    /// The response body could not be decoded into the expected shape.
    Decode(String),
    /// GitHub returned an OAuth error this flow does not model.
    Oauth(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Transport(e) => write!(f, "network error talking to GitHub: {e}"),
            AuthError::Status(code) => write!(f, "GitHub returned HTTP {code}"),
            AuthError::Decode(e) => write!(f, "could not read GitHub's response: {e}"),
            AuthError::Oauth(e) => write!(f, "GitHub rejected the request: {e}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// The device code the user must enter in their browser, plus the polling parameters.
///
/// Returned by [`DeviceFlow::start`]. The app shows `user_code` and sends the user to
/// `verification_uri`; it then polls with `device_code` no more often than every
/// `interval` seconds, giving up after `expires_in` seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    /// The long opaque code the app polls with (not shown to the user).
    pub device_code: String,
    /// The short human code the user types at `verification_uri`.
    pub user_code: String,
    /// Where the user enters `user_code` (e.g. `https://github.com/login/device`).
    pub verification_uri: String,
    /// Seconds until this code expires.
    pub expires_in: u64,
    /// Minimum seconds the app must wait between polls.
    pub interval: u64,
}

/// A user access token obtained from the device flow.
///
/// Debug/Display never print the token so it cannot leak into a log; see
/// [`AccessToken::redacted`] for a safe rendering.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken {
    /// The bearer token (`gho_…`). Treated as a secret: never logged, stored in the OS
    /// keychain by a later task.
    pub token: String,
    /// The granted scope string (e.g. `public_repo`).
    pub scope: String,
    /// The token type GitHub returned (`bearer`).
    pub token_type: String,
}

impl AccessToken {
    /// A log-safe one-liner that states the scope/type but never the token itself.
    #[must_use]
    pub fn redacted(&self) -> String {
        format!(
            "AccessToken {{ token: <redacted>, scope: {:?}, token_type: {:?} }}",
            self.scope, self.token_type
        )
    }
}

impl std::fmt::Debug for AccessToken {
    /// Redacts the token so a `{:?}` in a log or panic can never expose it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.redacted())
    }
}

/// The result of one poll of the token endpoint ([`DeviceFlow::poll_once`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// The user has not finished authorizing yet; poll again after `interval`.
    Pending,
    /// GitHub asked the app to slow down; use `interval` as the new minimum (it is the
    /// previous interval increased per GitHub's guidance).
    SlowDown { interval: u64 },
    /// The user authorized the app; the token is ready to use and store.
    Authorized(AccessToken),
    /// The user explicitly denied the request. Terminal — do not keep polling.
    Denied,
    /// The device code expired before the user authorized. Terminal — restart the flow.
    Expired,
}

/// Raw device-code JSON as GitHub returns it (when asked with `Accept: application/json`).
#[derive(Deserialize)]
struct DeviceCodeBody {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// Raw token-poll JSON: either a success (`access_token`) or an `error` code, both 200.
#[derive(Deserialize)]
struct TokenBody {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    interval: Option<u64>,
}

/// JSON headers for the device-flow requests: ask for and send JSON.
fn json_headers() -> [Header<'static>; 2] {
    [
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
    ]
}

/// Drives the GitHub device flow over a [`HttpTransport`].
///
/// Transport-agnostic by construction: tests inject a mock transport, production injects
/// [`ReqwestTransport`](crate::http::ReqwestTransport). Holds only the **public**
/// `client_id` and the requested `scope` — no secret.
pub struct DeviceFlow<'a> {
    transport: &'a dyn HttpTransport,
    client_id: String,
    scope: String,
}

impl<'a> DeviceFlow<'a> {
    /// A flow for `client_id` requesting [`REPORT_SCOPE`] over `transport`.
    #[must_use]
    pub fn new(transport: &'a dyn HttpTransport, client_id: impl Into<String>) -> Self {
        Self {
            transport,
            client_id: client_id.into(),
            scope: REPORT_SCOPE.to_owned(),
        }
    }

    /// Overrides the requested OAuth scope (default [`REPORT_SCOPE`]).
    #[must_use]
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Step 1: request a device + user code.
    ///
    /// # Errors
    /// [`AuthError::Transport`] if the request cannot be made, [`AuthError::Status`] for a
    /// non-200 reply, or [`AuthError::Decode`] if the body is not the expected JSON.
    pub fn start(&self) -> Result<DeviceCode, AuthError> {
        let body = format!(
            r#"{{"client_id":{},"scope":{}}}"#,
            json_str(&self.client_id),
            json_str(&self.scope)
        );
        let reply = self
            .transport
            .post_json(DEVICE_CODE_URL, &json_headers(), &body)
            .map_err(|e| AuthError::Transport(e.to_string()))?;
        if reply.status != 200 {
            return Err(AuthError::Status(reply.status));
        }
        let parsed: DeviceCodeBody =
            serde_json::from_str(&reply.body).map_err(|e| AuthError::Decode(e.to_string()))?;
        Ok(DeviceCode {
            device_code: parsed.device_code,
            user_code: parsed.user_code,
            verification_uri: parsed.verification_uri,
            expires_in: parsed.expires_in,
            interval: parsed.interval,
        })
    }

    /// Step 3: poll the token endpoint once, classifying the reply.
    ///
    /// Call this no more often than the device code's `interval`, bumping the wait on a
    /// [`PollOutcome::SlowDown`], and stop on [`PollOutcome::Authorized`],
    /// [`PollOutcome::Denied`] or [`PollOutcome::Expired`].
    ///
    /// # Errors
    /// [`AuthError::Transport`] / [`AuthError::Status`] / [`AuthError::Decode`] as in
    /// [`start`](DeviceFlow::start), or [`AuthError::Oauth`] for an unrecognized error code.
    pub fn poll_once(&self, device_code: &str) -> Result<PollOutcome, AuthError> {
        let body = format!(
            r#"{{"client_id":{},"device_code":{},"grant_type":{}}}"#,
            json_str(&self.client_id),
            json_str(device_code),
            json_str(DEVICE_GRANT_TYPE)
        );
        let reply = self
            .transport
            .post_json(ACCESS_TOKEN_URL, &json_headers(), &body)
            .map_err(|e| AuthError::Transport(e.to_string()))?;
        if reply.status != 200 {
            return Err(AuthError::Status(reply.status));
        }
        let parsed: TokenBody =
            serde_json::from_str(&reply.body).map_err(|e| AuthError::Decode(e.to_string()))?;

        if let Some(token) = parsed.access_token {
            return Ok(PollOutcome::Authorized(AccessToken {
                token,
                scope: parsed.scope.unwrap_or_default(),
                token_type: parsed.token_type.unwrap_or_default(),
            }));
        }
        match parsed.error.as_deref() {
            Some("authorization_pending") => Ok(PollOutcome::Pending),
            Some("slow_down") => Ok(PollOutcome::SlowDown {
                // GitHub returns a new, larger interval with slow_down; fall back to +5s.
                interval: parsed.interval.unwrap_or(0).max(DEFAULT_SLOW_DOWN_SECS),
            }),
            Some("access_denied") => Ok(PollOutcome::Denied),
            Some("expired_token") => Ok(PollOutcome::Expired),
            Some(other) => Err(AuthError::Oauth(other.to_owned())),
            None => Err(AuthError::Decode(
                "token reply had neither access_token nor error".to_owned(),
            )),
        }
    }
}

/// GitHub's documented minimum slow-down bump when it omits a new interval.
const DEFAULT_SLOW_DOWN_SECS: u64 = 5;

/// Persistent storage for the signed-in user's GitHub token (000251).
///
/// The bug reporter is auth-gated: a stored token means "signed in". Like the BYOK API
/// keys (see [`crate::keystore`]), the token is a **secret** — it must never land in a
/// project file, RON, or telemetry — so it lives only in a `TokenStore`. Two backends
/// implement the trait: [`MemoryTokenStore`], the ephemeral in-session reference (and the
/// dev/offline default), and — behind the `keychain` feature — [`KeychainTokenStore`],
/// which persists to the OS keychain. Neither backend is serializable, so the token cannot
/// reach disk by accident; only the token string is stored (the scope/type from
/// [`AccessToken`] are informational and not persisted).
///
/// Signing out is [`TokenStore::clear`] — idempotent, so clearing when already signed out
/// is not an error.
pub trait TokenStore {
    /// Stores `token`, replacing any existing one. The token is trimmed; an empty or
    /// whitespace-only token is rejected.
    ///
    /// # Errors
    /// [`KeyStoreError::EmptyKey`] for a blank token, or [`KeyStoreError::Backend`].
    fn store(&mut self, token: &str) -> Result<(), KeyStoreError>;

    /// Reads the stored token, or `None` when signed out.
    ///
    /// # Errors
    /// [`KeyStoreError::Backend`] if the store cannot be read.
    fn load(&self) -> Result<Option<String>, KeyStoreError>;

    /// Removes any stored token (sign-out). Idempotent: clearing when signed out succeeds.
    ///
    /// # Errors
    /// [`KeyStoreError::Backend`] if the store cannot be written.
    fn clear(&mut self) -> Result<(), KeyStoreError>;

    /// Convenience: persist the token from a freshly obtained [`AccessToken`], storing only
    /// the secret string (scope/type are not persisted).
    ///
    /// # Errors
    /// As [`store`](TokenStore::store).
    fn store_access(&mut self, token: &AccessToken) -> Result<(), KeyStoreError> {
        self.store(&token.token)
    }

    /// Whether a token is currently stored (i.e. the user is signed in).
    ///
    /// # Errors
    /// [`KeyStoreError::Backend`] if the store cannot be read.
    fn is_signed_in(&self) -> Result<bool, KeyStoreError> {
        Ok(self.load()?.is_some())
    }
}

/// Validates and normalizes a token before storage: trims surrounding whitespace and
/// rejects an empty result.
fn normalize_token(token: &str) -> Result<String, KeyStoreError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(KeyStoreError::EmptyKey);
    }
    Ok(trimmed.to_owned())
}

/// An in-session, in-memory token store — the offline/dev default and the tested reference.
///
/// Deliberately not `Serialize`/`Deserialize`: the token lives only in memory for the
/// session, exactly like [`MemoryKeyStore`](crate::keystore::MemoryKeyStore).
#[derive(Default)]
pub struct MemoryTokenStore {
    token: Option<String>,
}

impl MemoryTokenStore {
    /// An empty (signed-out) store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    fn store(&mut self, token: &str) -> Result<(), KeyStoreError> {
        self.token = Some(normalize_token(token)?);
        Ok(())
    }

    fn load(&self) -> Result<Option<String>, KeyStoreError> {
        Ok(self.token.clone())
    }

    fn clear(&mut self) -> Result<(), KeyStoreError> {
        self.token = None;
        Ok(())
    }
}

/// The OS-keychain-backed token store (macOS Security.framework via the `keyring` crate).
///
/// Enabled by the `keychain` feature. The token persists across sessions in the system
/// secure store under the app's service name and a fixed account label; the app holds only
/// those names, never the token material.
#[cfg(feature = "keychain")]
pub struct KeychainTokenStore {
    service: String,
    account: String,
}

#[cfg(feature = "keychain")]
impl KeychainTokenStore {
    /// The keychain account label the GitHub report token is filed under.
    pub const ACCOUNT: &'static str = "github-report-token";

    /// A store filing the token under `service` (e.g. `"SecLab"`) and [`Self::ACCOUNT`].
    #[must_use]
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: Self::ACCOUNT.to_owned(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, KeyStoreError> {
        keyring::Entry::new(&self.service, &self.account)
            .map_err(|e| KeyStoreError::Backend(e.to_string()))
    }
}

#[cfg(feature = "keychain")]
impl TokenStore for KeychainTokenStore {
    fn store(&mut self, token: &str) -> Result<(), KeyStoreError> {
        let token = normalize_token(token)?;
        self.entry()?
            .set_password(&token)
            .map_err(|e| KeyStoreError::Backend(e.to_string()))
    }

    fn load(&self) -> Result<Option<String>, KeyStoreError> {
        match self.entry()?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyStoreError::Backend(e.to_string())),
        }
    }

    fn clear(&mut self) -> Result<(), KeyStoreError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeyStoreError::Backend(e.to_string())),
        }
    }
}

/// Encodes `s` as a JSON string literal (quotes + escapes), so request bodies are valid
/// JSON even when a value contains `"` or `\`.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::mock::MockTransport;

    #[test]
    fn start_posts_client_id_and_scope_and_parses_the_code() {
        let transport = MockTransport::replying(
            200,
            r#"{"device_code":"dc-123","user_code":"WXYZ-1234",
                "verification_uri":"https://github.com/login/device",
                "expires_in":900,"interval":5}"#,
        );
        let flow = DeviceFlow::new(&transport, "Iv1.public_id");

        let code = flow.start().expect("start should succeed");
        assert_eq!(code.device_code, "dc-123");
        assert_eq!(code.user_code, "WXYZ-1234");
        assert_eq!(code.verification_uri, "https://github.com/login/device");
        assert_eq!(code.expires_in, 900);
        assert_eq!(code.interval, 5);

        // The request went to the device-code endpoint carrying the public client_id + scope.
        let req = transport.only_request();
        assert_eq!(req.url, DEVICE_CODE_URL);
        assert!(req.body.contains("Iv1.public_id"), "client_id in body");
        assert!(req.body.contains("public_repo"), "default scope in body");
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "Accept" && v == "application/json"),
            "asks for JSON"
        );
    }

    #[test]
    fn start_maps_a_dead_transport_to_transport_error() {
        let transport = MockTransport::failing("connection refused");
        let flow = DeviceFlow::new(&transport, "id");
        match flow.start() {
            Err(AuthError::Transport(msg)) => assert!(msg.contains("connection refused")),
            other => panic!("expected Transport error, got {other:?}"),
        }
    }

    #[test]
    fn start_rejects_a_non_200_status() {
        let transport = MockTransport::replying(404, "nope");
        let flow = DeviceFlow::new(&transport, "id");
        assert_eq!(flow.start(), Err(AuthError::Status(404)));
    }

    #[test]
    fn start_rejects_an_unparseable_body() {
        let transport = MockTransport::replying(200, "not json");
        let flow = DeviceFlow::new(&transport, "id");
        assert!(matches!(flow.start(), Err(AuthError::Decode(_))));
    }

    #[test]
    fn with_scope_overrides_the_default() {
        let transport = MockTransport::replying(
            200,
            r#"{"device_code":"d","user_code":"u",
                "verification_uri":"v","expires_in":1,"interval":1}"#,
        );
        let flow = DeviceFlow::new(&transport, "id").with_scope("repo");
        flow.start().unwrap();
        let req = transport.only_request();
        assert!(req.body.contains("\"repo\""), "custom scope sent");
        assert!(!req.body.contains("public_repo"), "default not sent");
    }

    #[test]
    fn poll_pending_is_classified() {
        let transport = MockTransport::replying(200, r#"{"error":"authorization_pending"}"#);
        let flow = DeviceFlow::new(&transport, "id");
        assert_eq!(flow.poll_once("dc").unwrap(), PollOutcome::Pending);

        // The poll hit the token endpoint with the device code + grant type.
        let req = transport.only_request();
        assert_eq!(req.url, ACCESS_TOKEN_URL);
        assert!(req.body.contains("\"dc\""), "device_code sent");
        assert!(req.body.contains(DEVICE_GRANT_TYPE), "grant_type sent");
    }

    #[test]
    fn poll_slow_down_carries_the_new_interval() {
        let transport = MockTransport::replying(200, r#"{"error":"slow_down","interval":10}"#);
        let flow = DeviceFlow::new(&transport, "id");
        assert_eq!(
            flow.poll_once("dc").unwrap(),
            PollOutcome::SlowDown { interval: 10 }
        );
    }

    #[test]
    fn poll_slow_down_without_interval_falls_back() {
        let transport = MockTransport::replying(200, r#"{"error":"slow_down"}"#);
        let flow = DeviceFlow::new(&transport, "id");
        assert_eq!(
            flow.poll_once("dc").unwrap(),
            PollOutcome::SlowDown { interval: 5 }
        );
    }

    #[test]
    fn poll_authorized_returns_the_token() {
        let transport = MockTransport::replying(
            200,
            r#"{"access_token":"gho_secret","token_type":"bearer","scope":"public_repo"}"#,
        );
        let flow = DeviceFlow::new(&transport, "id");
        match flow.poll_once("dc").unwrap() {
            PollOutcome::Authorized(tok) => {
                assert_eq!(tok.token, "gho_secret");
                assert_eq!(tok.scope, "public_repo");
                assert_eq!(tok.token_type, "bearer");
            }
            other => panic!("expected Authorized, got {other:?}"),
        }
    }

    #[test]
    fn poll_denied_and_expired_are_terminal_outcomes() {
        let denied = MockTransport::replying(200, r#"{"error":"access_denied"}"#);
        assert_eq!(
            DeviceFlow::new(&denied, "id").poll_once("dc").unwrap(),
            PollOutcome::Denied
        );
        let expired = MockTransport::replying(200, r#"{"error":"expired_token"}"#);
        assert_eq!(
            DeviceFlow::new(&expired, "id").poll_once("dc").unwrap(),
            PollOutcome::Expired
        );
    }

    #[test]
    fn poll_unknown_error_is_an_oauth_error() {
        let transport = MockTransport::replying(200, r#"{"error":"incorrect_client_credentials"}"#);
        let flow = DeviceFlow::new(&transport, "id");
        assert_eq!(
            flow.poll_once("dc"),
            Err(AuthError::Oauth("incorrect_client_credentials".to_owned()))
        );
    }

    #[test]
    fn poll_empty_reply_is_a_decode_error() {
        let transport = MockTransport::replying(200, "{}");
        let flow = DeviceFlow::new(&transport, "id");
        assert!(matches!(flow.poll_once("dc"), Err(AuthError::Decode(_))));
    }

    #[test]
    fn access_token_never_prints_the_secret() {
        let tok = AccessToken {
            token: "gho_topsecret".to_owned(),
            scope: "public_repo".to_owned(),
            token_type: "bearer".to_owned(),
        };
        let shown = format!("{tok:?}");
        assert!(!shown.contains("gho_topsecret"), "token must be redacted");
        assert!(shown.contains("public_repo"), "scope is safe to show");
        assert!(tok.redacted().contains("<redacted>"));
    }

    #[test]
    fn json_str_escapes_quotes_and_backslashes() {
        assert_eq!(json_str(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(json_str("line\n"), r#""line\n""#);
    }

    #[test]
    fn token_store_round_trips_and_sign_out_clears() {
        let mut store = MemoryTokenStore::new();
        assert_eq!(store.load().unwrap(), None);
        assert!(!store.is_signed_in().unwrap(), "starts signed out");

        store.store("gho_abc").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("gho_abc"));
        assert!(store.is_signed_in().unwrap(), "signed in after store");

        // Sign out.
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
        assert!(!store.is_signed_in().unwrap());
        // Clearing again (already signed out) is a no-op, not an error.
        store.clear().unwrap();
    }

    #[test]
    fn token_store_replaces_and_trims_and_rejects_blank() {
        let mut store = MemoryTokenStore::new();
        store.store("first").unwrap();
        store.store("  second  ").unwrap();
        assert_eq!(
            store.load().unwrap().as_deref(),
            Some("second"),
            "newest token wins and is trimmed"
        );

        assert_eq!(store.store("   "), Err(KeyStoreError::EmptyKey));
        assert_eq!(store.store(""), Err(KeyStoreError::EmptyKey));
        // A rejected store leaves the previous token intact.
        assert_eq!(store.load().unwrap().as_deref(), Some("second"));
    }

    #[test]
    fn store_access_persists_only_the_secret_from_an_access_token() {
        let tok = AccessToken {
            token: "gho_from_flow".to_owned(),
            scope: "public_repo".to_owned(),
            token_type: "bearer".to_owned(),
        };
        let mut store = MemoryTokenStore::new();
        store.store_access(&tok).unwrap();
        // Only the token string is stored; scope/type are not persisted.
        assert_eq!(store.load().unwrap().as_deref(), Some("gho_from_flow"));
    }
}
