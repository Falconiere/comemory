//! Whether a drain must stop at its next boundary: the logout barrier stands
//! ([`crate::domains::sync::auth_barrier`]), or the resident coordinator in
//! this process is shutting down.
//!
//! Checked when a run starts and after every batch, never in the middle of
//! one: an operation already sent is answered and recorded, so stopping loses
//! nothing and doubles nothing. A stopped pass ends [`End::Cancelled`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::config::Paths;
use crate::domains::sync::auth_barrier;
use crate::domains::sync::drain::report::End;

/// Set once by the coordinator's shutdown; one coordinator per process.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Ask every drain in this process to stop at its next boundary.
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

/// Whether a drain over `paths` must stop now.
#[must_use]
pub fn requested(paths: &Paths) -> bool {
    SHUTDOWN.load(Ordering::SeqCst) || auth_barrier::active(paths)
}

/// How a pass ends at a batch boundary, if it does: [`End::Cancelled`] when
/// a stop is [`requested`], [`End::Budget`] once `budget` has elapsed since
/// `started`.
#[must_use]
pub fn boundary(paths: &Paths, started: Instant, budget: Option<Duration>) -> Option<End> {
    if requested(paths) {
        return Some(End::Cancelled);
    }
    budget
        .is_some_and(|b| started.elapsed() >= b)
        .then_some(End::Budget)
}

#[cfg(test)]
#[path = "tests/stop.rs"]
mod tests;
