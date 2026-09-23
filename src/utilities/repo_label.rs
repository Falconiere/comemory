//! What an operator types for a repository, reduced to one comparable form.
//!
//! A label is user input: `CodaSignal/Foo`, `codasignal/foo` and
//! ` CodaSignal/Foo ` name the same repository. Every table keyed by a label
//! and every policy lookup has to agree on that, so the reduction lives in one
//! place rather than once per caller. `domains::sync::skip_repos` re-exports
//! it under its historical name; the shared layer owns it because
//! `store::repository_approval` keys rows by it and the documents capability
//! looks rows up by it, and neither may reach into a domain.

/// Lowercase trimmed repo label (`CodaSignal/Foo` -> `codasignal/foo`).
#[must_use]
pub fn normalize(label: &str) -> String {
    label.trim().to_lowercase()
}

#[cfg(test)]
#[path = "tests/repo_label.rs"]
mod tests;
