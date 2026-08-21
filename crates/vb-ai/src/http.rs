//! The HTTP transport seam shared by the network provider adapters (000113, 000114).
//!
//! [`AiProvider`](crate::provider::AiProvider) is transport-free by design (see
//! [`crate::provider`]); the concrete network adapters split cleanly into a pure request
//! codec (always compiled and unit-tested — no network) and this thin IO seam. A
//! [`HttpTransport`] turns a prepared JSON POST into a raw [`HttpReply`]; the adapter then
//! parses that reply. In tests a mock transport returns canned bytes, so the full
//! `complete` path — key lookup, body assembly, header set, response decoding — is
//! exercised with zero requests leaving the machine.
//!
//! The only piece that actually opens a socket is [`ReqwestTransport`], gated behind the
//! optional `net` feature. The default build (and CI) never compiles it, so nothing here
//! can send a real request unless a caller explicitly opts in with `--features net`.

use crate::provider::AiError;

/// A raw HTTP reply: the status code and the body decoded as UTF-8 text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    /// The HTTP status code (e.g. `200`, `401`).
    pub status: u16,
    /// The response body as text (JSON, for the adapters here).
    pub body: String,
}

/// One request header as a `(name, value)` pair.
pub type Header<'a> = (&'a str, &'a str);

/// Posts a JSON body to a URL and returns the raw reply.
///
/// The seam that keeps the provider adapters testable: production uses
/// [`ReqwestTransport`] (behind `net`); tests inject a mock that captures the request and
/// replays a canned response, so no request ever leaves the machine in CI.
pub trait HttpTransport {
    /// POSTs `body` to `url` with `headers` set, returning the status and body.
    ///
    /// Implementations translate transport failures (connect, timeout, TLS) into
    /// [`AiError::Network`]; a non-2xx status is **not** an error here — it is returned as
    /// an [`HttpReply`] for the adapter's parser to interpret.
    ///
    /// # Errors
    /// [`AiError::Network`] if the request cannot be completed at all.
    fn post_json(
        &self,
        url: &str,
        headers: &[Header<'_>],
        body: &str,
    ) -> Result<HttpReply, AiError>;
}

/// A real blocking HTTP transport over `reqwest` (rustls). Behind the `net` feature so the
/// default build carries no HTTP stack and cannot open a socket.
///
/// The provider trait is synchronous, so this uses `reqwest`'s blocking client, which
/// drives its own internal runtime — the assistant loop stays free of an async dependency.
#[cfg(feature = "net")]
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

#[cfg(feature = "net")]
impl ReqwestTransport {
    /// A transport with a default client and the given per-request timeout.
    ///
    /// # Errors
    /// [`AiError::Network`] if the underlying client cannot be constructed.
    pub fn new(timeout: std::time::Duration) -> Result<Self, AiError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| AiError::Network(e.to_string()))?;
        Ok(Self { client })
    }
}

#[cfg(feature = "net")]
impl HttpTransport for ReqwestTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[Header<'_>],
        body: &str,
    ) -> Result<HttpReply, AiError> {
        let mut req = self.client.post(url).body(body.to_owned());
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let resp = req.send().map_err(|e| AiError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp.text().map_err(|e| AiError::Network(e.to_string()))?;
        Ok(HttpReply { status, body })
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! A recording mock transport for the adapter tests.

    use std::cell::RefCell;

    use super::{Header, HttpReply, HttpTransport};
    use crate::provider::AiError;

    /// One captured request.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Captured {
        pub url: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    /// A transport that records every request and replays a fixed reply (or a network
    /// error). Never touches the network.
    pub struct MockTransport {
        reply: Result<HttpReply, AiError>,
        pub seen: RefCell<Vec<Captured>>,
    }

    impl MockTransport {
        /// Replays `status`/`body` for every request.
        pub fn replying(status: u16, body: impl Into<String>) -> Self {
            Self {
                reply: Ok(HttpReply {
                    status,
                    body: body.into(),
                }),
                seen: RefCell::new(Vec::new()),
            }
        }

        /// Fails every request with a network error (transport-down simulation).
        pub fn failing(msg: impl Into<String>) -> Self {
            Self {
                reply: Err(AiError::Network(msg.into())),
                seen: RefCell::new(Vec::new()),
            }
        }

        /// The single request captured so far (panics if not exactly one).
        pub fn only_request(&self) -> Captured {
            let seen = self.seen.borrow();
            assert_eq!(seen.len(), 1, "expected exactly one request");
            seen[0].clone()
        }

        /// How many requests reached the transport.
        pub fn call_count(&self) -> usize {
            self.seen.borrow().len()
        }
    }

    impl HttpTransport for MockTransport {
        fn post_json(
            &self,
            url: &str,
            headers: &[Header<'_>],
            body: &str,
        ) -> Result<HttpReply, AiError> {
            self.seen.borrow_mut().push(Captured {
                url: url.to_owned(),
                headers: headers
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                body: body.to_owned(),
            });
            self.reply.clone()
        }
    }
}
