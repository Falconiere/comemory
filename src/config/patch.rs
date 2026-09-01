//! The one read-patch-write primitive over `config.toml`, shared by every
//! writer that rewrites part of the file in place: `comemory tune`/`bandit
//! --apply` (`eval::tune::apply_to_config_file`), the `[reinforce]` toggle
//! behind `comemory hooks` (`api::hooks`), and the console-api routes
//! (`PUT /config/retrieval`, `PUT /gc/policy`, `PATCH /memory-stores/{id}`).
//! One home so the atomic tmp+rename, the missing-file bootstrap, and the
//! "key exists but is not a table" refusal cannot drift between callers
//! (Binding Rule 1).
//!
//! The file round-trips through `toml::Value`, so comments in an existing
//! file are lost — documented in `tune --apply`'s CLI help. Validation is
//! the caller's job: build the would-be `Config` in memory and run
//! `Config::validate` BEFORE calling [`patch_config_file`], so an invalid
//! knob never reaches disk.

use std::path::Path;

use toml::Value;
use toml::map::Map;

use crate::prelude::*;

/// A TOML table — the root document or one `[section]`.
pub type Table = Map<String, Value>;

/// Read `path` (an absent file is an empty document), hand its root table
/// to `edit`, and write the result back atomically (tmp + rename in the
/// same directory). A parse failure of the existing file is an
/// `Error::Config` naming the file — never silently overwritten.
pub fn patch_config_file(path: &Path, edit: impl FnOnce(&mut Table) -> Result<()>) -> Result<()> {
    let mut root: Value = if path.exists() {
        let raw = std::fs::read_to_string(path).map_err(Error::Io)?;
        toml::from_str(&raw).map_err(|e| Error::Config(format!("config.toml: {e}")))?
    } else {
        Value::Table(Map::new())
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| Error::Config("config.toml: root is not a table".into()))?;
    edit(table)?;
    let rendered = toml::to_string_pretty(&root)
        .map_err(|e| Error::Config(format!("config.toml render: {e}")))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, rendered).map_err(Error::Io)?;
    std::fs::rename(&tmp, path).map_err(Error::Io)?;
    Ok(())
}

/// Fetch-or-create the named sub-table of `table`. Errors when the key
/// exists but is not a table (a malformed config must not be silently
/// overwritten).
pub fn section<'t>(table: &'t mut Table, name: &str) -> Result<&'t mut Table> {
    table
        .entry(name)
        .or_insert_with(|| Value::Table(Map::new()))
        .as_table_mut()
        .ok_or_else(|| Error::Config(format!("config.toml: [{name}] is not a table")))
}

#[cfg(test)]
#[path = "tests/patch.rs"]
mod tests;
