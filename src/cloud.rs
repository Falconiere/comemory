//! Cloud platform client: API base URL, device-code login, and local
//! workspace-key credentials (`auth.json`).
//!
//! Slice 1 only — mints a workspace-bound `cmk_` via RFC 8628 device auth
//! + `POST /v1/device/mint-workspace-key`. No sync / device-key path here.
//! HTTP shells out through [`crate::fetch`] (curl/wget); no TLS stack in
//! the crate. CLI-only: there is no `/api/v1` route for `auth`.

/// Resolve the platform API base URL (`--api-url` / `COMEMORY_API` / default).
pub mod api_url;
/// `auth.json` credentials load / save / clear (mode `0600`, atomic).
pub mod credentials;
/// RFC 8628 device code → token poll → mint-workspace-key.
pub mod device;

pub use api_url::{DEFAULT_API_URL, resolve as resolve_api_url};
pub use credentials::{Credentials, clear, effective_secret, load, save, secret_override};
pub use device::{LoginOutcome, StatusReport, login, workspace_status};

/// RFC 8628 `client_id` the platform accepts for the CLI device grant.
pub const CLIENT_ID: &str = "comemory-cli";
