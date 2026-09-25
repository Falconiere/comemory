//! HTTP to an upstream for the exchange client: one request builder, one
//! answer classification.
//!
//! Every call carries the credential, a bounded timeout and — against a
//! managed origin — the negotiated protocol and policy revision, which the
//! answer must echo. An answer is either decoded data or a [`Failure`] the
//! session turns into a network state; nothing here decides what to do next.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{AUTHORIZATION, HeaderValue, RETRY_AFTER};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domains::sync::client_protocol::{PROTOCOL_HEADER, REVISION_HEADER};
use crate::domains::sync::drain::retry_after;
use crate::prelude::*;

/// Why a request produced no usable answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// `401`/`403`: the credential no longer authenticates.
    Auth(u16),
    /// `429`, with the delay `Retry-After` asked for.
    RateLimited(Option<Duration>),
    /// A timeout, a refused connection or a `5xx`.
    Unavailable(String),
    /// `404`: the upstream does not serve the route.
    NotFound,
    /// `409` with its code (`epoch_mismatch`, `cursor_ahead`,
    /// `sync_policy_changed`, `sync_upgrade_required`, `staging_incomplete`).
    Conflict(String),
    /// `413`, or a `400` refusing the request body.
    Refused(String),
    /// A `2xx` whose body is not the answer this protocol promised, or a
    /// managed answer that does not echo the negotiated protocol and revision.
    Protocol(String),
}

impl Failure {
    /// Whether the upstream answered that its stream is not the cursor's
    /// (`epoch_mismatch`, `cursor_ahead`) — a rebootstrap, not a failure.
    #[must_use]
    pub fn replaced_stream(&self) -> bool {
        matches!(self, Self::Conflict(code) if code == "epoch_mismatch" || code == "cursor_ahead")
    }
}

/// A decoded answer or why there is none.
pub type Answer<T> = std::result::Result<T, Failure>;

/// One upstream origin and credential.
#[derive(Debug, Clone)]
pub struct Transport {
    base: String,
    secret: String,
    client: Client,
    /// `(protocol, revision)` a managed origin must echo; `None` for an
    /// unmanaged engine, which has no policy revision to echo.
    managed: Option<(&'static str, i64)>,
    /// In-pass retries of an unavailable upstream.
    retries: u8,
}

/// The `{ok, data, error}` envelope every sync route answers with.
#[derive(serde::Deserialize)]
struct Envelope<T> {
    ok: bool,
    data: Option<T>,
    #[serde(default)]
    error: Option<EnvelopeError>,
}

#[derive(serde::Deserialize)]
struct EnvelopeError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

impl Transport {
    /// A transport for `api_url` under `timeout`.
    ///
    /// # Errors
    /// [`Error::Other`] when the HTTP client cannot be built.
    pub fn new(api_url: &str, secret: &str, timeout: Duration) -> Result<Self> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Other(format!("http client: {e}")))?;
        Ok(Self {
            base: api_url.trim_end_matches('/').to_string(),
            secret: secret.to_string(),
            client,
            managed: None,
            retries: 2,
        })
    }

    /// The same transport, speaking `protocol` at `revision` to a managed
    /// origin that must echo both.
    #[must_use]
    pub fn managed(mut self, protocol: &'static str, revision: i64) -> Self {
        self.managed = Some((protocol, revision));
        self
    }

    /// Whether this origin is managed (has a policy authority in front).
    #[must_use]
    pub const fn is_managed(&self) -> bool {
        self.managed.is_some()
    }

    /// `GET {api_url}{path}?{query}`.
    pub fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Answer<T> {
        self.send(self.client.get(format!("{}{path}", self.base)).query(query))
    }

    /// `POST {api_url}{path}` with a JSON body.
    pub fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Answer<T> {
        self.send(self.client.post(format!("{}{path}", self.base)).json(body))
    }

    /// Send, classify, check the managed echo, decode the envelope.
    fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Answer<T> {
        let mut request = request.header(AUTHORIZATION, format!("Bearer {}", self.secret));
        if let Some((protocol, revision)) = self.managed {
            request = request
                .header(PROTOCOL_HEADER, protocol)
                .header(REVISION_HEADER, revision.to_string());
        }
        let response = request
            .send()
            .map_err(|e| Failure::Unavailable(format!("{e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(classify(status, response));
        }
        self.check_echo(&response)?;
        let text = response
            .text()
            .map_err(|e| Failure::Unavailable(format!("reading the answer: {e}")))?;
        let envelope: Envelope<T> = serde_json::from_str(&text)
            .map_err(|e| Failure::Protocol(format!("undecodable answer: {e}")))?;
        match (envelope.ok, envelope.data) {
            (true, Some(data)) => Ok(data),
            _ => Err(Failure::Protocol(envelope.error.map_or_else(
                || "answer carries no data".to_string(),
                |e| format!("{}: {}", e.code, e.message),
            ))),
        }
    }

    /// A managed answer must echo the protocol and revision it was asked under.
    fn check_echo(&self, response: &Response) -> Answer<()> {
        let Some((protocol, revision)) = self.managed else {
            return Ok(());
        };
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v: &HeaderValue| v.to_str().ok())
                .map(str::to_string)
        };
        if header(PROTOCOL_HEADER).as_deref() != Some(protocol) {
            return Err(Failure::Protocol(format!(
                "the origin does not speak {protocol}"
            )));
        }
        if header(REVISION_HEADER).and_then(|v| v.parse::<i64>().ok()) != Some(revision) {
            return Err(Failure::Conflict("sync_policy_changed".to_string()));
        }
        Ok(())
    }
}

impl Transport {
    /// Run `call`, retrying an unavailable upstream at most
    /// [`Self::retries`] times with a jittered 1–8 s wait between attempts.
    /// Every other answer — a success, a rate limit (whose `Retry-After` the
    /// pass honors), a refusal — returns at once.
    pub fn retrying<T>(&self, mut call: impl FnMut(&Self) -> Answer<T>) -> Answer<T> {
        let mut attempt = 0;
        loop {
            match call(self) {
                Err(Failure::Unavailable(why)) if attempt < self.retries => {
                    attempt += 1;
                    tracing::debug!(attempt, %why, "upstream unavailable; retrying in this pass");
                    std::thread::sleep(crate::domains::sync::drain::backoff::in_pass(
                        crate::domains::sync::drain::backoff::fraction(),
                    ));
                }
                answer => return answer,
            }
        }
    }

    /// The same transport, making a single attempt per request — the inline
    /// push after a save must not sleep between retries.
    #[must_use]
    pub const fn without_retries(mut self) -> Self {
        self.retries = 0;
        self
    }
}

/// Turn a non-success status into a [`Failure`].
fn classify(status: StatusCode, response: Response) -> Failure {
    let retry = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(retry_after::parse);
    let code = response
        .text()
        .ok()
        .and_then(|text| serde_json::from_str::<Envelope<serde_json::Value>>(&text).ok())
        .and_then(|e| e.error)
        .map(|e| conflict_code(&e.code, &e.message))
        .unwrap_or_default();
    match status.as_u16() {
        401 | 403 => Failure::Auth(status.as_u16()),
        429 => Failure::RateLimited(retry),
        404 => Failure::NotFound,
        409 => Failure::Conflict(code),
        400 | 413 => Failure::Refused(format!("HTTP {status} {code}")),
        _ if status.is_server_error() => Failure::Unavailable(format!("HTTP {status}")),
        _ => Failure::Protocol(format!("HTTP {status} {code}")),
    }
}

/// The machine code of a refusal: the envelope's own code, or — for the
/// engine's generic `conflict` — the first `snake_case` word its message
/// carries before a colon (`… cursor_ahead: cursor is at …`).
fn conflict_code(code: &str, message: &str) -> String {
    if code != "conflict" {
        return code.to_string();
    }
    message
        .split(':')
        .map(str::trim)
        .find(|part| {
            *part != "conflict"
                && !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        })
        .unwrap_or(code)
        .to_string()
}

#[cfg(test)]
#[path = "tests/transport.rs"]
mod tests;
