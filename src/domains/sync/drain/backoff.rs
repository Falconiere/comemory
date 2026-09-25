//! How long to wait before the next attempt: full jitter inside a window that
//! doubles per consecutive failure.
//!
//! Randomly jittered, per client, so a restarted platform does not bring every
//! client back on the same instant; bounded, so a long outage never parks a
//! client for more than five minutes between tries.

use std::time::Duration;

/// Ceiling of the across-pass window.
pub const MAX: Duration = Duration::from_mins(5);

/// The across-pass delay after `failures` consecutive failed passes (1 for
/// the first): uniform in `[0, min(2^failures s, MAX)]`, drawn from `fraction`.
#[must_use]
pub fn after_failures(failures: u32, fraction: f64) -> Duration {
    window(failures).mul_f64(fraction.clamp(0.0, 1.0))
}

/// The upper bound [`after_failures`] draws under.
#[must_use]
pub fn window(failures: u32) -> Duration {
    Duration::from_secs(1_u64 << failures.min(9)).min(MAX)
}

/// The delay before in-pass retry `attempt` (1 for the first retry): uniform
/// in `[1 s, 8 s]` — a pass retries a failed request at most twice.
#[must_use]
pub fn in_pass(fraction: f64) -> Duration {
    Duration::from_secs(1) + Duration::from_secs(7).mul_f64(fraction.clamp(0.0, 1.0))
}

/// A fraction in `[0, 1]` for the jitter, read from `/dev/urandom`, with the
/// clock's sub-second remainder standing in where it cannot be read.
#[must_use]
pub fn fraction() -> f64 {
    let mut bytes = [0_u8; 2];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_ok()
    {
        return f64::from(u16::from_le_bytes(bytes)) / f64::from(u16::MAX);
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    f64::from(nanos % 1_000) / 1_000.0
}

#[cfg(test)]
#[path = "tests/backoff.rs"]
mod tests;
