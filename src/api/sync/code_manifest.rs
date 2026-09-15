//! `GET /sync/code/manifest?repo=` — the per-file digest list the pushing
//! side diffs against, plus the repo's head and co-change cursor.

use crate::api::Ctx;
use crate::api::sync::code_import_rules::repo_label_error;
use crate::api::sync::code_types::{CodeFileRef, CodeManifestResponse};
use crate::prelude::*;
use crate::store::{indexed_files, repo_marker};

/// List every `indexed_files` row for `repo` with the marker's cursors. An
/// unknown label is not an error: the answer is an empty list, which is
/// exactly what makes a first push send everything. A label the import
/// would refuse (untrimmed, or carrying `:`) is refused here too, so a
/// client cannot be told "empty" about a repo it could never push.
///
/// # Errors
/// [`Error::BadRequest`] for an invalid label; store failures otherwise.
pub fn run(ctx: &mut Ctx<'_>, repo: &str) -> Result<CodeManifestResponse> {
    if let Some(reason) = repo_label_error(repo) {
        return Err(Error::BadRequest(reason.into()));
    }
    let conn = ctx.conn()?;
    let files = indexed_files::list_for_repo(conn, repo)?
        .into_iter()
        .map(|(path, blob_oid)| CodeFileRef { path, blob_oid })
        .collect();
    Ok(CodeManifestResponse {
        repo: repo.to_owned(),
        head: repo_marker::last_head(conn, repo)?,
        mined_commit: repo_marker::last_mined_commit(conn, repo)?,
        files,
    })
}
