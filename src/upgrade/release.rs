//! Talking to GitHub Releases without an HTTP client in the binary:
//! `comemory upgrade` shells out through [`crate::fetch`] (`curl`, falling
//! back to `wget`), the same tools the shell installer itself needs, so the
//! upgrade path adds no TLS stack to the crate. Two operations: resolve the
//! `latest` redirect to a tag, and download one asset to a file.

use std::path::Path;

use crate::config::env::env_parse;
use crate::fetch;
use crate::prelude::*;

/// Where releases live. `COMEMORY_RELEASES_URL` overrides it — a test hook
/// so the suite can point at a loopback fixture server, not a user knob.
pub const DEFAULT_RELEASES_URL: &str = "https://github.com/Falconiere/comemory/releases";

/// The release base URL, trailing slash trimmed.
pub fn releases_url() -> Result<String> {
    Ok(env_parse::<String>("COMEMORY_RELEASES_URL")?.map_or_else(
        || DEFAULT_RELEASES_URL.to_string(),
        |u| u.trim_end_matches('/').to_string(),
    ))
}

/// The tag `<base>/latest` redirects to (`v0.19.0`).
pub fn latest_tag(base: &str) -> Result<String> {
    let url = format!("{base}/latest");
    let landed = fetch::final_url(&url)?;
    let tag = landed
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let looks_like_tag = tag
        .strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    if looks_like_tag {
        Ok(tag.to_string())
    } else {
        Err(Error::Unavailable(format!(
            "could not resolve the latest release from {url} (landed on {landed})"
        )))
    }
}

/// Download `url` to `dest`, failing on any non-2xx status.
pub fn download(url: &str, dest: &Path) -> Result<()> {
    fetch::download(url, dest)
}
