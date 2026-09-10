//! Cloud platform client: API base URL and RFC 8628 device-code login.
//!
//! Mints the unbound device key via `POST /v1/device/mint-device-key` and
//! persists it as [`crate::sync::auth_file::AuthFile`] — one `auth.json`
//! schema shared with `sync`, which reads the same file. HTTP shells out
//! through [`crate::fetch`] (curl/wget); no TLS stack in the crate.
//! CLI-only: there is no `/api/v1` route for `auth`.

/// Resolve the platform API base URL (`--api-url` / `COMEMORY_API` / default).
pub mod api_url;
/// RFC 8628 device code → token poll → mint-device-key.
pub mod device;

pub use api_url::{DEFAULT_API_URL, resolve as resolve_api_url};
pub use device::{
    LoginOutcome, StatusReport, WorkspaceInfo, list_workspaces, login, workspace_status,
};

/// RFC 8628 `client_id` the platform accepts for the CLI device grant.
pub const CLIENT_ID: &str = "comemory-cli";
