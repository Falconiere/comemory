//! Canonical GitHub repository identities derived from checkout remotes.

use url::Url;

/// Parse a supported `github.com` checkout remote into lowercase `owner/name`.
///
/// Accepted forms are HTTPS, `ssh://`, and Git's SCP-style SSH spelling.
/// Credentials on HTTPS, non-`git` SSH users, ports, queries, fragments, and
/// paths other than exactly `owner/name` are refused.
#[must_use]
pub fn canonical_github_repository(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if let Some(path) = remote.strip_prefix("git@github.com:") {
        return canonical_github_name(path);
    }

    let parsed = Url::parse(remote).ok()?;
    let authority = remote.split_once("://")?.1.split('/').next()?;
    if parsed.host_str()? != "github.com" || parsed.query().is_some() || parsed.fragment().is_some()
    {
        return None;
    }
    match parsed.scheme() {
        "https" if authority == "github.com" => {}
        "ssh" if authority == "git@github.com" => {}
        _ => return None,
    }
    canonical_github_name(parsed.path().trim_start_matches('/'))
}

/// Normalize an exact GitHub `owner/name` policy identity.
#[must_use]
pub fn canonical_github_name(path: &str) -> Option<String> {
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() || !valid_component(owner) || !valid_component(name) {
        return None;
    }
    Some(format!("{owner}/{name}").to_ascii_lowercase())
}

fn valid_component(component: &str) -> bool {
    !component.is_empty()
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
#[path = "tests/repository_identity.rs"]
mod tests;
