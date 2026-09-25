//! The network state a key carries between passes: what the last failure
//! means for the next request, and the clean state a success restores.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::domains::sync::drain::backoff;
use crate::domains::sync::drain::transport::Failure;
use crate::prelude::*;
use crate::store::sync_exchange::ExchangeRow;

/// `last_error` prefix of a backoff the upstream asked for with `Retry-After`,
/// which even a manual run honors.
pub const RATE_LIMITED: &str = "rate_limited";

/// The conflict a managed origin answers when the policy revision a request
/// carried is no longer current.
pub const POLICY_CHANGED: &str = "sync_policy_changed";

/// Record `failure` on `row`. `fingerprint` names the credential a `401`/`403`
/// suspends.
///
/// # Errors
/// [`Error::Other`] when the clock cannot be formatted.
pub fn fail(row: &mut ExchangeRow, failure: &Failure, fingerprint: &str) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    match failure {
        Failure::Auth(status) => {
            row.network_state = "auth_suspended".to_string();
            row.suspended_fingerprint = Some(fingerprint.to_string());
            row.last_error = Some(format!("HTTP {status}: the credential was refused"));
        }
        Failure::RateLimited(delay) => {
            row.consecutive_failures += 1;
            let delay = delay.unwrap_or_else(|| backoff::window(failures(row)));
            row.network_state = "backoff".to_string();
            row.retry_at = Some(stamp(now + delay)?);
            row.last_error = Some(format!("{RATE_LIMITED}: retry after {}s", delay.as_secs()));
        }
        Failure::Unavailable(why) => {
            row.consecutive_failures += 1;
            let delay = backoff::after_failures(failures(row), backoff::fraction());
            row.network_state = "backoff".to_string();
            row.retry_at = Some(stamp(now + delay)?);
            row.last_error = Some(why.clone());
        }
        Failure::NotFound | Failure::Conflict(_) | Failure::Refused(_) | Failure::Protocol(_) => {
            row.network_state = "protocol_error".to_string();
            row.last_error = Some(format!("{failure:?}"));
        }
    }
    Ok(())
}

/// Restore the clean state after a pass reached the upstream.
///
/// # Errors
/// [`Error::Other`] when the clock cannot be formatted.
pub fn succeed(row: &mut ExchangeRow) -> Result<()> {
    row.network_state = "ok".to_string();
    row.consecutive_failures = 0;
    row.retry_at = None;
    row.last_error = None;
    row.suspended_fingerprint = None;
    row.last_ok_at = Some(now()?);
    Ok(())
}

/// Whether `row` forbids a request now. A manual run skips only a backoff the
/// upstream asked for; an unattended one skips every backoff.
///
/// # Errors
/// [`Error::Other`] for a stored `retry_at` that does not parse.
pub fn gated(row: &ExchangeRow, fingerprint: &str, manual: bool) -> Result<bool> {
    if row.network_state == "auth_suspended" {
        return Ok(row.suspended_fingerprint.as_deref() == Some(fingerprint));
    }
    if row.network_state != "backoff" {
        return Ok(false);
    }
    let Some(retry_at) = row.retry_at.as_deref() else {
        return Ok(false);
    };
    let retry_at = OffsetDateTime::parse(retry_at, &Rfc3339)
        .map_err(|e| Error::Other(format!("stored retry_at {retry_at}: {e}")))?;
    if retry_at <= OffsetDateTime::now_utc() {
        return Ok(false);
    }
    let asked = row
        .last_error
        .as_deref()
        .is_some_and(|e| e.starts_with(RATE_LIMITED));
    Ok(!manual || asked)
}

/// The current time as RFC 3339.
///
/// # Errors
/// [`Error::Other`] when the clock cannot be formatted.
pub fn now() -> Result<String> {
    stamp(OffsetDateTime::now_utc())
}

fn stamp(at: OffsetDateTime) -> Result<String> {
    at.format(&Rfc3339)
        .map_err(|e| Error::Other(format!("timestamp: {e}")))
}

fn failures(row: &ExchangeRow) -> u32 {
    u32::try_from(row.consecutive_failures).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "tests/network.rs"]
mod tests;
