//! Cloud platform client: API base URL and RFC 8628 device-code login.
//!
//! Mints the organization-scoped key via `POST /v1/device/mint-org-key` and
//! persists it as [`crate::sync::auth_file::AuthFile`] — one `auth.json`
//! schema shared with `sync`, which reads the same file. HTTP for device
//! auth shells out through [`crate::utilities::fetch`] (curl/wget). Sync and capture
//! use in-process reqwest with the `rustls` feature for https platforms.
//! CLI-only: there is no `/api/v1` route for `auth`.

/// Resolve the platform API base URL (`--api-url` / `COMEMORY_API` / default).
pub mod api_url;
/// RFC 8628 device code → token poll → mint-org-key.
pub mod device;

pub use api_url::{DEFAULT_API_URL, resolve as resolve_api_url};
pub use device::{LoginOutcome, StatusReport, login, org_status};

/// RFC 8628 `client_id` the platform accepts for the CLI device grant.
pub const CLIENT_ID: &str = "comemory-cli";
