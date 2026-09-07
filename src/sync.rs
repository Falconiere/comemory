//! Cloud-sync client helpers: match keys, redaction, auth file, allowlist cache.

pub mod allowlist_cache;
pub mod auth_file;
pub mod auto;
pub mod client;
pub mod match_key;
pub mod pull;
pub mod push;
pub mod redact;
pub mod verify;

pub use allowlist_cache::{AllowlistCache, classify_with_cache};
pub use auth_file::AuthFile;
pub use match_key::{
    AllowlistRepo, MatchOutcome, classify_repo, normalize_git_remote, normalize_repo_label,
};
pub use redact::{scan, scan_with_override};
