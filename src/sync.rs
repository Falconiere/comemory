//! Cloud-sync client helpers: credentials, redaction, the local skip filter,
//! and the two push paths — memories (`push`) and the code index (`code`).

pub mod auth_file;
pub mod client;
pub mod client_code;
pub mod code;
pub mod code_plan;
pub mod daemon;
pub mod daemon_templates;
pub mod daemon_unit;
pub mod initial;
pub mod pull;
pub mod push;
pub mod push_on_save;
pub mod redact;
pub mod skip_repos;
pub mod verify;

pub use auth_file::AuthFile;
pub use initial::{InitialSyncStats, run_initial_sync};
pub use redact::{Finding, RULE_SET_VERSION, findings, redact, scan, scan_with_override};
pub use skip_repos::{SkipMatcher, normalize_repo_label};
