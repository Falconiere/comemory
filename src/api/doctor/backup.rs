//! Check 4 of `comemory doctor`: the newest `comemory.db.pre-v{N}.bak`
//! migration snapshot beside the live database. Split out of `checks.rs`
//! for the size ceiling; it reuses that module's `newest_matching` scan
//! (which `api::doctor::system` shares) and its `ok` builder.

use crate::config::Paths;
use crate::prelude::*;

use super::checks::{Check, newest_matching, ok};

/// Check 4: the newest `comemory.db.pre-v{N}.bak` snapshot beside the live
/// db, if any. Absent is `"ok"` with a detail saying so, not a failure —
/// see the module doc.
pub(crate) fn migration_backup(paths: &Paths) -> Result<(Check, Option<String>, Option<u64>)> {
    let db_path = paths.db_path();
    let Some(db_name) = db_path.file_name().and_then(|n| n.to_str()) else {
        return Ok((
            ok("migration backup", "db path has no file name"),
            None,
            None,
        ));
    };
    let prefix = format!("{db_name}.pre-v");
    let newest = newest_matching(paths.data_dir(), &prefix)?;
    Ok(match newest {
        Some((path, size)) => (
            ok("migration backup", format!("found {}", path.display())),
            Some(path.to_string_lossy().into_owned()),
            Some(size),
        ),
        None => (
            ok("migration backup", "no pre-migration backup present"),
            None,
            None,
        ),
    })
}
