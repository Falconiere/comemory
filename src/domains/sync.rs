//! `domains::sync` — everything about keeping this machine and the
//! organization's platform holding the same memories and code index.
//!
//! [`auth_file`] owns the one org-scoped credential; [`client`] speaks the
//! wire; [`push`], [`pull`], [`code`] and [`verify`] are the directions of
//! travel, run once together by [`initial`]. [`redact`] and [`skip_repos`]
//! decide what may leave this machine.
//!
//! Every SQL string stays in the central `store`. Clap flags, prompts, process
//! launch and every `writeln!` stay in `cli`; HTTP policy stays in `serve`.

/// Load/save the org-scoped `auth.json` credential (v2; a v1 file is refused).
pub mod auth_file;
/// Enveloped platform HTTP over reqwest + rustls, plus the workspace
/// channel's ticket and URL.
pub mod client;
/// The two code-index calls layered on [`client`]'s base URL and credential.
pub mod client_code;
/// `comemory sync`'s code-index push: diff by blob OID, send only what differs.
pub mod code;
/// The pure code-push diff and its batching — no store, no network.
pub mod code_plan;
/// Platform API base URL and the RFC 8628 device login that mints the key.
pub mod cloud;
/// The user-level auto-sync daemon (launchd / systemd --user), opt-in.
pub mod daemon;
/// Rendered launchd plist and systemd unit bodies.
pub mod daemon_templates;
/// Daemon unit lifecycle: write, load, start, stop, status.
pub mod daemon_unit;
/// The server side of the protocol: wire models plus the changes, manifest,
/// import and code-import cores `serve` routes call.
pub mod exchange;
/// The exhaustive pull-then-push-then-code run `auth login` performs.
pub mod initial;
/// Git synchronization of the data directory itself — commit, pull, push —
/// which is the memory store's own `store-sync` job, not platform push/pull.
pub mod memory_store;
/// The cursored pull half of `comemory sync`.
pub mod pull;
/// The push half of `comemory sync`, filtered by `skip_repos` alone.
pub mod push;
/// The inline outbox drain a local save or delete triggers, time-bounded and
/// never fatal to the write.
pub mod push_on_save;
/// Curated secret scan (`rules.toml`) before a memory is enqueued for push.
pub mod redact;
/// The one client-side filter left: the `[sync] skip_repos` glob matcher.
pub mod skip_repos;
/// Manifest compare and bucket repair.
pub mod verify;

pub use auth_file::AuthFile;
pub use initial::{InitialSyncStats, run_initial_sync};
pub use redact::{Finding, RULE_SET_VERSION, findings, redact, scan, scan_with_override};
pub use skip_repos::{SkipMatcher, normalize_repo_label};
