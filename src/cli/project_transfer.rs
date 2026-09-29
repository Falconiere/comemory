//! `comemory project export|import` (#342) — the offline transfer core over
//! two data directories. This file owns the flags, the bundle file I/O and
//! the TTY rendering; the cores are `domains::projects::{export, import}`.
//! A skipped import is an answer, not a failure: it exits `0` and names both
//! digests.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;

use crate::cli::output::json;
use crate::domains::projects::authority::{self, Envelope};
use crate::domains::projects::bundle::{self, Bundle};
use crate::domains::projects::export;
use crate::domains::projects::import::{self, Outcome};
use crate::prelude::*;
use crate::utilities::context::Ctx;
use crate::utilities::private_file::write_atomically;

/// Args for `project export`.
#[derive(ClapArgs, Debug)]
pub struct ExportArgs {
    /// The project's UUID.
    pub id: String,
    /// Write the bundle to this file (owner-only, replaced atomically)
    /// instead of stdout.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

/// Args for `project import`.
#[derive(ClapArgs, Debug)]
pub struct ImportArgs {
    /// The bundle file `project export` wrote, or `-` for stdin.
    pub bundle: String,
    /// The other side, recorded on the transfer binding (default:
    /// `file:<absolute path>`, or `stdin`).
    #[arg(long)]
    pub remote: Option<String>,
}

/// `project export`: the bundle on stdout, or in `--output` with a summary.
pub fn export(
    ctx: &mut Ctx<'_>,
    envelope: &Envelope,
    a: ExportArgs,
    json_flag: bool,
) -> Result<()> {
    let bundle = authority::run(ctx, envelope, export::Request { id: a.id })?;
    let Some(path) = a.output else {
        return json::write(&bundle);
    };
    write_atomically(&path, &serde_json::to_vec(&bundle)?)?;
    let summary = serde_json::json!({
        "projectId": bundle.project_id,
        "digest": bundle.digest,
        "rows": row_count(&bundle),
        "output": path.display().to_string(),
    });
    if json_flag {
        return json::write(&summary);
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "exported {} ({} rows, digest {}) to {}",
        bundle.project_id,
        row_count(&bundle),
        short(&bundle.digest),
        path.display()
    )?;
    Ok(())
}

/// `project import`: parse the bundle, import it, and say what happened.
pub fn import(
    ctx: &mut Ctx<'_>,
    envelope: &Envelope,
    a: ImportArgs,
    json_flag: bool,
) -> Result<()> {
    let (bytes, source) = read_bundle(&a.bundle)?;
    let bundle = bundle::parse(&bytes)?;
    let remote = a.remote.unwrap_or(source);
    let resp = authority::run(
        ctx,
        envelope,
        import::Import {
            bundle,
            remote,
            remap: None,
        },
    )?;
    if json_flag {
        return json::write(&resp);
    }
    let (id, digest) = (&resp.project_id, short(&resp.digest));
    let local = resp.local_digest.as_deref().map(short).unwrap_or_default();
    let mut out = std::io::stdout().lock();
    match resp.outcome {
        Outcome::Imported => writeln!(out, "imported {id} ({} rows, digest {digest})", resp.rows),
        Outcome::Unchanged => {
            writeln!(
                out,
                "unchanged {id}: this copy is identical (digest {digest})"
            )
        }
        Outcome::Skipped => writeln!(
            out,
            "skipped {id}: this data directory holds a different copy \
             (local digest {local}, bundle digest {digest}); both are kept"
        ),
    }?;
    Ok(())
}

/// The bundle's bytes and its default remote label.
fn read_bundle(arg: &str) -> Result<(Vec<u8>, String)> {
    if arg == "-" {
        let mut bytes = Vec::new();
        std::io::stdin().lock().read_to_end(&mut bytes)?;
        return Ok((bytes, "stdin".to_string()));
    }
    let path = Path::new(arg);
    let bytes = std::fs::read(path)?;
    let absolute = std::path::absolute(path)?;
    Ok((bytes, format!("file:{}", absolute.display())))
}

/// Rows in every table of `bundle`.
fn row_count(bundle: &Bundle) -> usize {
    bundle.tables.iter().map(|t| t.rows.len()).sum()
}

/// The first twelve hex characters of a digest, for a person to compare.
fn short(digest: &str) -> String {
    digest.chars().take(12).collect()
}
