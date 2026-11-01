//! The bug-reporter orchestration: one small state-machine that ties sign-in, the token
//! store and issue creation together (000254), so the editor's "Report a problem / feature
//! request" button is a thin view over it — the same split the assistant uses
//! ([`EditSession`](crate::edit_session), [`DesignSession`](crate::design)).
//!
//! [`ReportSession`] owns the reporter's [`TokenStore`] and the public OAuth client id, and
//! drives the [`HttpTransport`] seam, so the whole flow — device-flow sign-in, persisting
//! the token, assembling + scrubbing the payload, and POSTing the issue — is exercised
//! headlessly against a mock with zero network. The UI calls the steps and renders the
//! current state; it never re-implements the policy.
//!
//! Two safety properties live here, not in the view:
//! 1. **Auth gate** — [`submit`](ReportSession::submit) refuses with
//!    [`AuthError::Unauthorized`] when no token is stored (no account → no report), and
//!    clears the token on a 401 so the UI re-prompts a sign-in.
//! 2. **Self-redaction** — the signed-in token is always added to the scrub list before the
//!    payload is built, so the reporter can never leak its own credential into an issue.

use crate::github::{
    create_issue, AuthError, CreatedIssue, DeviceCode, DeviceFlow, PollOutcome, TokenStore,
    REPORT_CLIENT_ID, REPORT_REPO_NAME, REPORT_REPO_OWNER,
};
use crate::report::{build_issue, Metadata, ReportDraft};

/// Orchestrates the in-app bug reporter over a [`TokenStore`].
///
/// Holds the reporter's public OAuth `client_id` and a mutable token store; each step takes
/// the [`HttpTransport`](crate::http::HttpTransport) so tests inject a mock and production
/// injects the real one. Construct with [`ReportSession::new`] (the baked-in
/// [`REPORT_CLIENT_ID`]) or [`ReportSession::with_client_id`].
pub struct ReportSession<'a> {
    client_id: String,
    store: &'a mut dyn TokenStore,
}

impl<'a> ReportSession<'a> {
    /// A session over `store` using the baked-in public [`REPORT_CLIENT_ID`].
    #[must_use]
    pub fn new(store: &'a mut dyn TokenStore) -> Self {
        Self {
            client_id: REPORT_CLIENT_ID.to_owned(),
            store,
        }
    }

    /// A session with an explicit `client_id` (for tests or an alternate OAuth app).
    #[must_use]
    pub fn with_client_id(store: &'a mut dyn TokenStore, client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            store,
        }
    }

    /// Whether a token is stored (the user is signed in).
    ///
    /// # Errors
    /// [`AuthError::Transport`] if the token store cannot be read.
    pub fn is_signed_in(&self) -> Result<bool, AuthError> {
        self.store.is_signed_in().map_err(store_err)
    }

    /// Step 1 of sign-in: request a device + user code to show the user.
    ///
    /// # Errors
    /// As [`DeviceFlow::start`].
    pub fn begin_sign_in(
        &self,
        transport: &dyn crate::http::HttpTransport,
    ) -> Result<DeviceCode, AuthError> {
        DeviceFlow::new(transport, self.client_id.clone()).start()
    }

    /// Step 2 of sign-in: poll once for the token. On [`PollOutcome::Authorized`] the token
    /// is persisted to the store before the outcome is returned, so the caller only has to
    /// stop polling — the session is then signed in.
    ///
    /// # Errors
    /// As [`DeviceFlow::poll_once`], plus [`AuthError::Transport`] if storing the token fails.
    pub fn poll_sign_in(
        &mut self,
        transport: &dyn crate::http::HttpTransport,
        device_code: &str,
    ) -> Result<PollOutcome, AuthError> {
        let outcome = DeviceFlow::new(transport, self.client_id.clone()).poll_once(device_code)?;
        if let PollOutcome::Authorized(token) = &outcome {
            self.store.store_access(token).map_err(store_err)?;
        }
        Ok(outcome)
    }

    /// Signs out: clears the stored token. Idempotent.
    ///
    /// # Errors
    /// [`AuthError::Transport`] if the store cannot be written.
    pub fn sign_out(&mut self) -> Result<(), AuthError> {
        self.store.clear().map_err(store_err)
    }

    /// Submits a report as a GitHub issue, as the signed-in user.
    ///
    /// Requires a stored token (else [`AuthError::Unauthorized`]). The stored token is added
    /// to the scrub list automatically — so the reporter can never leak its own credential —
    /// alongside any `extra_secrets` the caller passes (e.g. BYOK API keys). On a 401 the
    /// token is cleared so the UI prompts a fresh sign-in.
    ///
    /// # Errors
    /// [`AuthError::Unauthorized`] when not signed in or the token was rejected, otherwise as
    /// [`create_issue`].
    pub fn submit(
        &mut self,
        transport: &dyn crate::http::HttpTransport,
        draft: &ReportDraft,
        meta: &Metadata,
        log_excerpt: &str,
        extra_secrets: &[String],
    ) -> Result<CreatedIssue, AuthError> {
        let token = self
            .store
            .load()
            .map_err(store_err)?
            .ok_or(AuthError::Unauthorized)?;

        // Never leak our own token into the issue: scrub it along with any caller secrets.
        let mut secrets: Vec<String> = extra_secrets.to_vec();
        secrets.push(token.clone());
        let payload = build_issue(draft, meta, log_excerpt, &secrets);

        match create_issue(
            transport,
            &token,
            REPORT_REPO_OWNER,
            REPORT_REPO_NAME,
            &payload,
        ) {
            Ok(issue) => Ok(issue),
            Err(AuthError::Unauthorized) => {
                // The token is stale/revoked — drop it so the UI re-prompts sign-in.
                let _ = self.store.clear();
                Err(AuthError::Unauthorized)
            }
            Err(e) => Err(e),
        }
    }
}

/// Maps a token-store failure onto the reporter's error type.
fn store_err(e: crate::keystore::KeyStoreError) -> AuthError {
    AuthError::Transport(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{AccessToken, MemoryTokenStore};
    use crate::http::mock::MockTransport;
    use crate::report::ReportKind;

    fn draft() -> ReportDraft {
        ReportDraft::new(ReportKind::Bug, "no sound", "the VCA stays shut")
    }

    #[test]
    fn submit_without_sign_in_is_unauthorized_and_opens_no_request() {
        let mut store = MemoryTokenStore::new();
        let mut session = ReportSession::new(&mut store);
        assert!(!session.is_signed_in().unwrap());

        let transport = MockTransport::replying(201, "{}");
        let err = session
            .submit(&transport, &draft(), &Metadata::current(None), "", &[])
            .unwrap_err();
        assert_eq!(err, AuthError::Unauthorized);
        assert_eq!(transport.call_count(), 0, "nothing was sent");
    }

    #[test]
    fn begin_sign_in_requests_a_device_code() {
        let mut store = MemoryTokenStore::new();
        let session = ReportSession::with_client_id(&mut store, "Iv1.test");
        let transport = MockTransport::replying(
            200,
            r#"{"device_code":"dc","user_code":"UC-1","verification_uri":"https://github.com/login/device","expires_in":900,"interval":5}"#,
        );
        let code = session.begin_sign_in(&transport).unwrap();
        assert_eq!(code.user_code, "UC-1");
        assert!(transport.only_request().body.contains("Iv1.test"));
    }

    #[test]
    fn poll_authorized_persists_the_token() {
        let mut store = MemoryTokenStore::new();
        let mut session = ReportSession::new(&mut store);
        let transport = MockTransport::replying(
            200,
            r#"{"access_token":"gho_fresh","token_type":"bearer","scope":"public_repo"}"#,
        );
        let outcome = session.poll_sign_in(&transport, "dc").unwrap();
        assert!(matches!(outcome, PollOutcome::Authorized(_)));
        assert!(session.is_signed_in().unwrap(), "token stored on authorize");
        assert_eq!(store.load().unwrap().as_deref(), Some("gho_fresh"));
    }

    #[test]
    fn poll_pending_stores_nothing() {
        let mut store = MemoryTokenStore::new();
        let mut session = ReportSession::new(&mut store);
        let transport = MockTransport::replying(200, r#"{"error":"authorization_pending"}"#);
        assert_eq!(
            session.poll_sign_in(&transport, "dc").unwrap(),
            PollOutcome::Pending
        );
        assert!(!session.is_signed_in().unwrap());
    }

    #[test]
    fn submit_signed_in_creates_the_issue() {
        let mut store = MemoryTokenStore::new();
        store.store("gho_tok").unwrap();
        let mut session = ReportSession::new(&mut store);

        let transport = MockTransport::replying(
            201,
            r#"{"number":7,"html_url":"https://github.com/ice1x/SecLab/issues/7","title":"x"}"#,
        );
        let issue = session
            .submit(&transport, &draft(), &Metadata::current(None), "", &[])
            .unwrap();
        assert_eq!(issue.number, 7);

        let req = transport.only_request();
        assert_eq!(req.url, "https://api.github.com/repos/ice1x/SecLab/issues");
        assert!(req
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer gho_tok"));
    }

    #[test]
    fn submit_scrubs_the_sessions_own_token_out_of_the_logs() {
        let mut store = MemoryTokenStore::new();
        store.store("gho_SELFTOKEN").unwrap();
        let mut session = ReportSession::new(&mut store);

        // The token leaked into the logs; submit must redact it even though the caller did
        // not list it as a secret.
        let logs = "request used gho_SELFTOKEN and failed";
        let transport = MockTransport::replying(
            201,
            r#"{"number":1,"html_url":"https://github.com/ice1x/SecLab/issues/1"}"#,
        );
        session
            .submit(&transport, &draft(), &Metadata::current(None), logs, &[])
            .unwrap();
        let body = transport.only_request().body;
        assert!(!body.contains("gho_SELFTOKEN"), "own token never leaves");
    }

    #[test]
    fn submit_on_401_signs_out_so_the_ui_reprompts() {
        let mut store = MemoryTokenStore::new();
        store.store("gho_stale").unwrap();
        let mut session = ReportSession::new(&mut store);

        let transport = MockTransport::replying(401, r#"{"message":"Bad credentials"}"#);
        let err = session
            .submit(&transport, &draft(), &Metadata::current(None), "", &[])
            .unwrap_err();
        assert_eq!(err, AuthError::Unauthorized);
        assert!(
            !session.is_signed_in().unwrap(),
            "stale token cleared on 401"
        );
    }

    #[test]
    fn sign_out_clears_the_token() {
        let mut store = MemoryTokenStore::new();
        store.store("gho_tok").unwrap();
        let mut session = ReportSession::new(&mut store);
        assert!(session.is_signed_in().unwrap());
        session.sign_out().unwrap();
        assert!(!session.is_signed_in().unwrap());
    }

    #[test]
    fn extra_secrets_are_scrubbed_too() {
        let mut store = MemoryTokenStore::new();
        store.store("gho_tok").unwrap();
        let mut session = ReportSession::new(&mut store);

        let logs = "the key sk-ant-BYOKKEY was used";
        let transport = MockTransport::replying(201, r#"{"number":2,"html_url":"https://x/2"}"#);
        session
            .submit(
                &transport,
                &draft(),
                &Metadata::current(None),
                logs,
                &["sk-ant-BYOKKEY".to_owned()],
            )
            .unwrap();
        assert!(!transport.only_request().body.contains("sk-ant-BYOKKEY"));
    }

    // Keep AccessToken in scope used (the authorize path builds one internally); this guards
    // that the re-export path compiles for the UI layer too.
    #[test]
    fn access_token_type_is_reachable() {
        let tok = AccessToken {
            token: "t".into(),
            scope: "public_repo".into(),
            token_type: "bearer".into(),
        };
        assert_eq!(tok.scope, "public_repo");
    }
}
