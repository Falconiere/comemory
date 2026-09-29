//! `projects.slug`, derived from the charter's `name` — ported exactly from
//! the platform's `project-slug.ts`. The plain slug is tried first; the
//! create core retries with [`disambiguate`] only after a real unique-index
//! collision, never a pre-check.

/// How long a base slug may be.
const BASE_MAX: usize = 60;

/// Lowercase, hyphenated, ASCII-only: every run of anything but `[a-z0-9]`
/// collapses to one `-`, leading and trailing `-` are dropped, an empty
/// result falls back to `project`, and the rest is cut at 60 characters.
#[must_use]
pub fn base_slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "project".to_string()
    } else {
        trimmed.chars().take(BASE_MAX).collect()
    }
}

/// The slug to try on zero-based `attempt`: `base`, then `base-2`, `base-3`, …
#[must_use]
pub fn disambiguate(base: &str, attempt: usize) -> String {
    if attempt == 0 {
        base.to_string()
    } else {
        format!("{base}-{}", attempt + 1)
    }
}

#[cfg(test)]
#[path = "tests/slug.rs"]
mod tests;
