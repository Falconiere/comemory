//! The per-request repo scope (console-api spec §1 "Repo scope"): an
//! optional default for every read that accepts a `repo` filter, resolved
//! from the `X-Comemory-Repo` header first and the server's own `--repo`
//! default second. [`RepoScope`] is an axum extractor any handler can add;
//! its [`RepoScope::apply`] folds the resolved scope into a request's own
//! `repo` field **only when that field is absent** — an explicit query/body
//! `repo` always wins, so a client that never sends the header (on a server
//! started without `--repo`) sees no change at all.
//!
//! The org-scope header (`X-Comemory-Org`) is deliberately not modelled:
//! accounts and orgs are out of scope (spec Non-Goal 1).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use std::convert::Infallible;

use crate::serve::AppState;

/// The header name, lowercase as `http` stores it.
pub const REPO_HEADER: &str = "x-comemory-repo";

/// The resolved default repo scope — `None` when neither the header (absent,
/// empty, or not valid UTF-8: an unreadable scope is treated as no scope
/// rather than a rejection, since it is a filter, not a credential) nor the
/// server's `--repo` supplied one.
#[derive(Debug, Clone, Default)]
pub struct RepoScope(pub Option<String>);

impl RepoScope {
    /// Fill `repo` from the resolved scope when the request left it unset —
    /// and only then. Named for that guard rather than `apply`, because a
    /// call site reading `scope.fill_if_absent(&mut req.repo)` looks like it might
    /// overwrite an explicit `?repo=`, which is the one thing this must
    /// never do: the header and `--repo` are defaults, not overrides.
    pub fn fill_if_absent(&self, repo: &mut Option<String>) {
        if repo.is_none() {
            repo.clone_from(&self.0);
        }
    }
}

impl FromRequestParts<AppState> for RepoScope {
    type Rejection = Infallible;

    /// Header first, the server's `--repo` second. Nothing here awaits, so
    /// the future is built ready rather than through an `async fn`.
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = std::result::Result<Self, Self::Rejection>> + Send {
        let header = parts
            .headers
            .get(REPO_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        std::future::ready(Ok(Self(
            header.or_else(|| state.repo().map(str::to_string)),
        )))
    }
}

#[cfg(test)]
#[path = "tests/scope.rs"]
mod tests;
