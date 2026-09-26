//! `domains::sync` — everything about keeping this machine and the
//! organization's platform holding the same memories and code index.
//!
//! [`auth_file`][a] owns the one org-scoped credential; [`client`][c] speaks
//! the wire; `push`, `pull`, `code` and `verify` are the directions of travel,
//! run once together by `initial` and on every hook-fired pass by `auto`; repository policy, `redact`, and
//! `skip_repos` decide what may leave this machine.
//!
//! [a]: crate::domains::sync::auth_file
//! [c]: crate::domains::sync::client
//!
//! Every SQL string stays in the central `store`. Clap flags, prompts, process
//! launch and every `writeln!` stay in `cli`; HTTP policy stays in `serve`.

/// The durable logout barrier that hides every credential until a login.
pub mod auth_barrier;
/// Load/save the org-scoped `auth.json` credential (v2; a v1 file is refused).
pub mod auth_file;
/// `comemory sync --action auto`: the coalesced, cwd-free pass hooks fire.
pub mod auto;
/// Enveloped platform HTTP over reqwest + rustls, plus the workspace
/// channel's ticket and URL.
pub mod client;
/// The two code-index calls layered on [`client`]'s base URL and credential.
pub mod client_code;
/// Status fetch and wire model for repository policy negotiation.
pub mod client_policy;
/// Managed data-request protocol headers and response validation.
pub mod client_protocol;
/// Platform API base URL and the RFC 8628 device login that mints the key.
pub mod cloud;
/// `comemory sync`'s code-index push: diff by blob OID, send only what differs.
pub mod code;
/// The pure code-push diff and its batching — no store, no network.
pub mod code_plan;
/// One canonical repository's code-manifest diff and upload.
pub mod code_repo_push;
/// The user-level auto-sync daemon (launchd / systemd --user), opt-in.
pub mod daemon;
/// Rendered launchd plist and systemd unit bodies.
pub mod daemon_templates;
/// Daemon unit lifecycle: write, load, start, stop, status.
pub mod daemon_unit;
/// The `replica-v1` exchange client: negotiation, the push/pull drain, holds,
/// backoff, replay, verification and workspace keying (#255).
pub mod drain;
/// The server side of the protocol: wire models plus the changes, manifest,
/// import and code-import cores `serve` routes call.
pub mod exchange;
/// The exhaustive pull-then-push-then-code run `auth login` performs.
pub mod initial;
/// The `comemory auth` sequences: device login, status probe, logout.
pub mod login;
/// What one `comemory sync` run does: the session it opens and the three
/// composite action sequences.
pub mod manual;
/// Git synchronization of the data directory itself — commit, pull, push —
/// which is the memory store's own `store-sync` job, not platform push/pull.
pub mod memory_store;
/// The cursored pull half of `comemory sync`.
pub mod pull;
/// The push half of `comemory sync`, filtered by repository policy and local exclusions.
pub mod push;
/// The inline outbox drain a local save or delete triggers, time-bounded and
/// never fatal to the write.
pub mod push_on_save;
/// Curated secret scan (`rules.toml`) before a memory is enqueued for push.
pub mod redact;
/// The `replica-v1` journal protocol: contract, acceptance, reads.
pub mod replica;
/// Strict `github.com` remote parsing into lowercase `owner/name` identities.
pub mod repository_identity;
/// Server policy resolved against local checkout remotes and legacy mappings.
pub mod repository_policy;
/// The local `[sync] skip_repos` glob matcher, applied after repository policy.
pub mod skip_repos;
/// Manifest compare and bucket repair.
pub mod vector_rule;
pub mod verify;
/// `comemory watch`: hold the workspace channel and pull on every nudge.
pub mod watch;

pub use auth_file::AuthFile;
pub use initial::{InitialSyncStats, run_initial_sync};
pub use redact::{Finding, RULE_SET_VERSION, findings, redact, scan, scan_with_override};
pub use skip_repos::{SkipMatcher, normalize_repo_label};
