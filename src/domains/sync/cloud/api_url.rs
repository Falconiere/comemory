//! Platform API base URL resolution for cloud auth.
//!
//! Precedence (highest wins): explicit CLI override → `COMEMORY_API` env →
//! [`DEFAULT_API_URL`]. Trailing slashes are stripped so path joins stay
//! stable.

use crate::config::env::env_parse;
use crate::prelude::*;

/// Production platform API origin.
pub const DEFAULT_API_URL: &str = "https://api.comemory.io";

/// Resolve the API base URL. `cli_override` is the `--api-url` flag when set.
pub fn resolve(cli_override: Option<&str>) -> Result<String> {
    if let Some(raw) = cli_override.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(trim_slash(raw));
    }
    if let Some(raw) = env_parse::<String>("COMEMORY_API")? {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(trim_slash(trimmed));
        }
    }
    Ok(DEFAULT_API_URL.to_string())
}

fn trim_slash(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

#[cfg(test)]
#[path = "tests/api_url.rs"]
mod tests;
