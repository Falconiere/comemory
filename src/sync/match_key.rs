//! GitHub App org-repo allowlist match keys (`2026-09-02-memory-sync-design.md`).

use serde::{Deserialize, Serialize};

/// One allowlisted repository — `full_name` and `name` are lowercase.
///
/// Platform wire uses camelCase (`fullName`); on-disk cache uses snake_case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowlistRepo {
    /// Lowercase `owner/name` from the GitHub App installation.
    #[serde(alias = "fullName")]
    pub full_name: String,
    /// Lowercase repository basename (the segment after the final `/`).
    pub name: String,
}

/// Result of classifying a memory's `repo` label against the allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    /// Label matches an allowlisted repo (exact `full_name` or unique basename).
    Allowed,
    /// Empty or unbound label — v1 sync skips personal/unbound memories.
    SkippedPersonal,
    /// Non-empty label that does not match any allowlisted repo.
    SkippedNotInOrg,
    /// Basename matches two or more allowlisted repos — label must be fully qualified.
    SkippedAmbiguous,
}

/// Lowercase trimmed repo label (`CodaSignal/Foo` → `codasignal/foo`).
pub fn normalize_repo_label(label: &str) -> String {
    label.trim().to_lowercase()
}

/// Classify `label` against `allowlist`: exact `full_name`, unique basename,
/// or a skip reason for CLI `sync status`.
pub fn classify_repo(label: &str, allowlist: &[AllowlistRepo]) -> MatchOutcome {
    let normalized = normalize_repo_label(label);
    if normalized.is_empty() {
        return MatchOutcome::SkippedPersonal;
    }
    if normalized.contains('/') {
        let allowed = allowlist.iter().any(|repo| repo.full_name == normalized);
        return if allowed {
            MatchOutcome::Allowed
        } else {
            MatchOutcome::SkippedNotInOrg
        };
    }
    let mut hits = 0u32;
    for repo in allowlist {
        if repo.name == normalized {
            hits += 1;
            if hits > 1 {
                return MatchOutcome::SkippedAmbiguous;
            }
        }
    }
    match hits {
        1 => MatchOutcome::Allowed,
        0 => MatchOutcome::SkippedNotInOrg,
        _ => MatchOutcome::SkippedAmbiguous,
    }
}

/// Parse an `https` or `ssh` git remote into lowercase `owner/name`.
///
/// Handles `git@host:owner/repo.git` and `https://host/owner/repo.git`,
/// stripping a trailing `.git` suffix. Returns `None` when the URL shape
/// is not recognized.
pub fn normalize_git_remote(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = if let Some(rest) = trimmed.strip_prefix("git@") {
        rest.split_once(':').map(|(_, p)| p)?
    } else if let Ok(parsed) = url::Url::parse(trimmed) {
        let mut segments: Vec<&str> = parsed.path_segments()?.collect();
        if segments.first() == Some(&"") {
            segments.remove(0);
        }
        if segments.len() < 2 {
            return None;
        }
        let owner = segments[0];
        let repo = segments[1].trim_end_matches(".git");
        return Some(format!("{}/{}", owner.to_lowercase(), repo.to_lowercase()));
    } else {
        return None;
    };

    let mut parts = path.split('/');
    let owner = parts.next()?.trim_end_matches(".git");
    let repo = parts.next()?.trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{}/{}", owner.to_lowercase(), repo.to_lowercase()))
}

#[cfg(test)]
#[path = "tests/match_key.rs"]
mod tests;
