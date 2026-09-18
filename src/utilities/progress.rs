//! The progress / cancellation contract a long-running job reports through.
//!
//! Transport-neutral on purpose (#166): the walk that emits the events lives
//! in a domain (`index-code`, `reembed`), the sink that consumes them lives in
//! delivery (`serve::jobs::worker`, which streams them as SSE), and the CLI
//! passes `None` for the no-op. Neither side may own the trait.

/// Progress-reporting sink for a long walk: [`ProgressSink::on_progress`]
/// after every candidate file (indexed or skipped alike) with the running
/// `(done, total)` file counts, [`ProgressSink::on_log`] for one
/// human-readable line per file actually (re)indexed. `serve::jobs::worker`
/// implements this over the job registry; the CLI never constructs one —
/// `crate::domains::code::index_code::run` passes `None`, which is the "no-op" the plan calls
/// for. Implementations must be best-effort: neither method
/// returns a `Result`, so a reporting failure can only be handled (e.g.
/// `tracing::warn!`) inside the implementation itself, never by failing the
/// walk it instruments.
pub trait ProgressSink: Send + Sync {
    /// Report progress after processing one candidate file.
    fn on_progress(&self, done: u64, total: u64);
    /// Append one line to the job's log tail.
    fn on_log(&self, line: &str);
    /// Whether the caller asked this run to stop (`POST /jobs/{id}/cancel`).
    /// Polled at every file boundary; `true` makes the walk return
    /// [`Error::Cancelled`] and roll its transaction back. Defaults to
    /// `false` — a sink that cannot be cancelled never is.
    fn is_cancelled(&self) -> bool {
        false
    }
}
