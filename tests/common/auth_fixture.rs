#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    // Shared across several test binaries; each uses a different subset.
    dead_code
)]
//! One builder for the org-scoped `auth.json` every sync test needs.
//!
//! Written once here rather than as a struct literal in each suite: `AuthFile`
//! is the schema this change reshapes, and a literal per test file is a
//! per-file edit every time a field moves.

use comemory::config::paths::Paths;
use comemory::sync::auth_file::{AUTH_SCHEMA_VERSION, AuthFile};

/// Default org workspace id the loopback sync fixture serves.
pub const FIXTURE_WORKSPACE: &str = "ws-org";

/// An org-scoped credential pointing at `api_url`, reaching `workspace_id`.
pub fn org_auth(api_url: &str, secret: &str, workspace_id: &str) -> AuthFile {
    AuthFile {
        version: AUTH_SCHEMA_VERSION,
        secret: secret.into(),
        key_prefix: "cmk_bbbb".into(),
        api_url: api_url.into(),
        organization_id: "99999999-8888-7777-6666-555555555555".into(),
        organization_slug: "acme".into(),
        organization_name: "Acme, Inc.".into(),
        workspace_id: workspace_id.into(),
        email: None,
    }
}

/// Write an org-scoped credential into `paths`.
pub fn seed_org_auth(paths: &Paths, api_url: &str, secret: &str, workspace_id: &str) -> AuthFile {
    let auth = org_auth(api_url, secret, workspace_id);
    auth.save(paths).expect("write auth.json");
    auth
}

/// Raw JSON of a credential written before organization scoping: no `version`,
/// with the two fields v2 dropped. Used to prove the version gate rejects it.
pub fn legacy_v1_json(api_url: &str) -> String {
    serde_json::json!({
        "secret": "cmk_legacy",
        "key_prefix": "cmk_lega",
        "personal_workspace_id": "ws-personal",
        "api_url": api_url,
        "device_name": "laptop",
    })
    .to_string()
}
