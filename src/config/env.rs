//! `COMEMORY_*` env-var overrides for [`Config`].
//!
//! Split out of `file.rs` to keep each config file narrow: `file.rs` owns
//! the struct definitions, defaults, file overlay, and shared validation;
//! this module owns env parsing. Both entry points funnel through the same
//! `Config::validate` pass so file and env layers enforce identical
//! invariants.

use super::file::{AutoReindexMode, Config};
use crate::prelude::*;

/// Read an env var as text: `None` when unset or when the value is not valid
/// Unicode. A non-Unicode value is ignored exactly like an unset one — never an
/// error — so every `COMEMORY_*` reader below agrees on what an unreadable
/// value means, and the free-form sync overrides keep the behavior they had
/// before they moved onto the shared override table.
fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn env_nonblank(name: &str) -> Option<String> {
    env_text(name).filter(|value| !value.trim().is_empty())
}

/// Read an env var and parse it as `T`; `Ok(None)` when unset or not valid
/// Unicode (see [`env_text`]), `Err` naming the variable on parse failure. A
/// `String` target takes the value verbatim, which is how the free-form sync
/// timeouts are read. Only the parse happens here — range invariants live in
/// `Config::validate` so the file overlay enforces them identically. Also
/// reused by `eval::tune` for its `COMEMORY_TUNE_MIN_GOLDEN` test hook.
pub(crate) fn env_parse<T: std::str::FromStr>(name: &str) -> Result<Option<T>>
where
    T::Err: std::fmt::Display,
{
    let Some(v) = env_text(name) else {
        return Ok(None);
    };
    let parsed = v
        .parse::<T>()
        .map_err(|e| Error::Other(format!("invalid env var {name}={v}: {e}")))?;
    Ok(Some(parsed))
}

/// Read an env var as exactly `n` comma-separated numbers; `Ok(None)` when
/// unset, `Err` naming the variable on a shape or parse failure.
///
/// Only the shape is checked here; the range invariants (finite, ordering,
/// sign) live in `Config::validate` so the file overlay enforces them
/// identically. The shared core of [`env_pair`] and [`env_triple`].
fn env_numbers<T: std::str::FromStr>(name: &str, n: usize) -> Result<Option<Vec<T>>>
where
    T::Err: std::fmt::Display,
{
    let Some(v) = env_text(name) else {
        return Ok(None);
    };
    let parts: Vec<&str> = v.split(',').collect();
    if parts.len() != n {
        return Err(Error::Other(format!(
            "invalid env var {name}={v}: expected {n} comma-separated numbers"
        )));
    }
    let mut out = Vec::with_capacity(n);
    for (i, part) in parts.iter().enumerate() {
        out.push(part.trim().parse::<T>().map_err(|e| {
            Error::Other(format!("invalid env var {name}={v}: value {}: {e}", i + 1))
        })?);
    }
    Ok(Some(out))
}

/// Parse one of the boolean env vars, naming the variable when it is not one.
pub(super) fn parse_bool_env(name: &str, raw: &str) -> Result<bool> {
    match raw {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        other => Err(Error::Other(format!(
            "invalid env var {name}: '{other}' (expected true|1|yes|on or false|0|no|off)"
        ))),
    }
}

/// One `fn name<T>(name) -> Result<Option<tuple>>` per row, each a fixed-width
/// view over [`env_numbers`]; only the width and the tuple built from the
/// parsed values vary.
macro_rules! env_tuples {
    ($(
        $(#[$meta:meta])*
        $fn_name:ident => $n:literal, $tuple:ty, |$v:ident| $build:expr
    ),+ $(,)?) => {
        $(
            $(#[$meta])*
            fn $fn_name<T: std::str::FromStr + Copy>(name: &str) -> Result<Option<$tuple>>
            where
                T::Err: std::fmt::Display,
            {
                Ok(env_numbers::<T>(name, $n)?.map(|$v| $build))
            }
        )+
    };
}

env_tuples! {
    /// Read an env var as an `"a,b"` pair of numbers; `Ok(None)` when unset.
    /// Shared by `COMEMORY_RANK_PRIOR_CLAMP` (`f64`) and
    /// `COMEMORY_RETRIEVAL_BM25_WEIGHTS` (`f32`).
    env_pair => 2, (T, T), |v| (v[0], v[1]),
    /// Read an env var as an `"a,b,c"` triple of numbers; `Ok(None)` when
    /// unset. Used by `COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS` (`f32`, column
    /// order symbol,snippet,path_tokens).
    env_triple => 3, (T, T, T), |v| (v[0], v[1], v[2]),
}

/// Read a boolean env var (`true|1|yes|on` / `false|0|no|off`); `Ok(None)`
/// when unset.
fn env_bool(name: &str) -> Result<Option<bool>> {
    let Some(raw) = env_text(name) else {
        return Ok(None);
    };
    Ok(Some(parse_bool_env(name, &raw)?))
}

/// Read `COMEMORY_INDEXING_AUTO_REINDEX` (`lazy|hook|off`); `Ok(None)` when
/// unset, `Err` naming the variable on any other value.
fn env_auto_reindex(name: &str) -> Result<Option<AutoReindexMode>> {
    let Some(v) = env_text(name) else {
        return Ok(None);
    };
    match v.as_str() {
        "lazy" => Ok(Some(AutoReindexMode::Lazy)),
        "hook" => Ok(Some(AutoReindexMode::Hook)),
        "off" => Ok(Some(AutoReindexMode::Off)),
        other => Err(Error::Other(format!(
            "invalid env var {name}: '{other}' (expected lazy|hook|off)"
        ))),
    }
}

/// Apply a parsed override when present; an absent variable preserves the current layer.
fn apply_override<T>(
    target: &mut T,
    name: &str,
    read: impl FnOnce(&str) -> Result<Option<T>>,
) -> Result<()> {
    if let Some(value) = read(name)? {
        *target = value;
    }
    Ok(())
}

/// One `fn name(&mut self) -> Result<()>` per row, applying a table of
/// `COMEMORY_*` numeric overrides to the fields of one or more `Config`
/// sections, in order. Each entry names the field, its variable, and
/// optionally the reader (`env_parse` unless `via` says otherwise). Only the
/// table varies, so it replaces near-identical method bodies.
macro_rules! env_override_fns {
    (@one $target:expr, $var:literal) => {
        apply_override(&mut $target, $var, env_parse)?
    };
    (@one $target:expr, $var:literal, $read:path) => {
        apply_override(&mut $target, $var, $read)?
    };
    ($(
        $(#[$meta:meta])*
        $fn_name:ident($($section:ident { $($field:ident => $var:literal $(via $read:path)?),+ $(,)? })+)
    ),+ $(,)?) => {
        impl Config {
            $(
                $(#[$meta])*
                fn $fn_name(&mut self) -> Result<()> {
                    $($(
                        env_override_fns!(@one self.$section.$field, $var $(, $read)?);
                    )+)+
                    Ok(())
                }
            )+
        }
    };
}

impl Config {
    /// Apply `COMEMORY_*` env-var overrides on top of `self`.
    ///
    /// Unlike the previous infallible variant, parse failures (non-numeric
    /// `top_k` / thresholds, unknown `auto_reindex` mode, unknown boolean for
    /// `auto_sync`) are now surfaced as `Error::Other` rather than silently
    /// dropped. This catches typos at startup instead of letting them mask as
    /// "defaults applied". Rank/prune range invariants are enforced by the
    /// shared `Config::validate` pass at the end.
    pub fn with_env(mut self) -> Result<Self> {
        self.apply_indexing_env()?;
        self.apply_retrieval_env()?;
        self.apply_sync_env()?;
        self.apply_embed_hint_env();
        self.embed_cmd = env_nonblank("COMEMORY_EMBED_CMD");
        self.document_embed_cmd = env_nonblank("COMEMORY_DOCUMENT_EMBED_CMD");
        self.document_embed_hint = env_nonblank("COMEMORY_DOCUMENT_EMBED_HINT");
        self.code_embed_cmd = env_nonblank("COMEMORY_CODE_EMBED_CMD");
        self.code_embed_hint = env_nonblank("COMEMORY_CODE_EMBED_HINT");
        self.apply_tuning_env()?;
        // The `[observations]` reads live beside their own section, in
        // `config::observations`, rather than as another near-identical
        // `apply_*_env` method here.
        self.observations.apply_env()?;
        self.activity.apply_env()?;
        // The `[tune]` grid lists deliberately have NO env equivalents: a
        // four-list env value ("20,60,100" × 4 vars, or worse, one var with
        // semicolons) is unreadable and easy to misquote. Set them in
        // config.toml's `[tune]` section instead; validation still runs.
        self.validate()
    }

    /// `COMEMORY_EMBED_HINT` → [`Config::embed_hint`]. Infallible: the hint
    /// is free-form text, echoed back verbatim by `comemory doctor`.
    fn apply_embed_hint_env(&mut self) {
        // COMEMORY_VECTOR_DIM and COMEMORY_CODE_VECTOR_DIM are intentionally
        // not honoured here. The authoritative dim lives in the `memory_vec`
        // / `code_vec` vec0 DDL (`migrations/0002_v2_tables.sql`) and is
        // baked in at migration time; an env override would silently disagree
        // with the vtab and surface as `VecDimMismatch` at first insert.
        if let Some(v) = env_text("COMEMORY_EMBED_HINT") {
            self.embed_hint = Some(v);
        }
    }
}

env_override_fns! {
    /// `COMEMORY_INDEXING_*` → [`Config::indexing`].
    apply_indexing_env(indexing {
        auto_reindex => "COMEMORY_INDEXING_AUTO_REINDEX" via env_auto_reindex,
        max_file_bytes => "COMEMORY_INDEXING_MAX_FILE_BYTES",
    }),
    /// `COMEMORY_RETRIEVAL_*` → [`Config::retrieval`]. Only the parse happens
    /// here; the finite/positive invariants live in `Config::validate` so the
    /// file overlay is checked identically.
    apply_retrieval_env(retrieval {
        top_k => "COMEMORY_RETRIEVAL_TOP_K",
        max_page_window => "COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW",
        memory_threshold => "COMEMORY_RETRIEVAL_MEMORY_THRESHOLD",
        rrf_k => "COMEMORY_RETRIEVAL_RRF_K",
        graph_hops => "COMEMORY_RETRIEVAL_GRAPH_HOPS",
        graph_seeds => "COMEMORY_RETRIEVAL_GRAPH_SEEDS",
        bm25_weights => "COMEMORY_RETRIEVAL_BM25_WEIGHTS" via env_pair::<f32>,
        code_threshold => "COMEMORY_RETRIEVAL_CODE_THRESHOLD",
        code_bm25_weights => "COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS" via env_triple::<f32>,
        document_leg_weight => "COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT",
    }),
    /// `COMEMORY_SYNC_PUSH_ON_SAVE` / `COMEMORY_SYNC_CODE_INDEX` /
    /// `COMEMORY_SYNC_PUSH_ON_SAVE_TIMEOUT` / `COMEMORY_SYNC_PAUSE_WAIT` →
    /// [`Config::sync`], then `COMEMORY_GIT_AUTO_SYNC` → [`Config::git`].
    apply_sync_env(
        sync {
            push_on_save => "COMEMORY_SYNC_PUSH_ON_SAVE" via env_bool,
            code_index => "COMEMORY_SYNC_CODE_INDEX" via env_bool,
            push_on_save_timeout => "COMEMORY_SYNC_PUSH_ON_SAVE_TIMEOUT",
            pause_wait => "COMEMORY_SYNC_PAUSE_WAIT",
        }
        git { auto_sync => "COMEMORY_GIT_AUTO_SYNC" via env_bool }
    ),
    /// `COMEMORY_RANK_*` → [`Config::rank`], `COMEMORY_PRUNE_*` and
    /// `COMEMORY_LEARNING_RETENTION_DAYS` → [`Config::prune`], and
    /// `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` → [`Config::reinforce`].
    ///
    /// Parsing happens here; range invariants are enforced once for both
    /// env and file overlays by `Config::validate`.
    apply_tuning_env(
        rank {
            decay => "COMEMORY_RANK_DECAY",
            prior_clamp => "COMEMORY_RANK_PRIOR_CLAMP" via env_pair,
            mmr_lambda => "COMEMORY_RANK_MMR_LAMBDA",
            near_dup_hamming => "COMEMORY_RANK_NEAR_DUP_HAMMING",
        }
        prune {
            min_activation => "COMEMORY_PRUNE_MIN_ACTIVATION",
            min_feedback => "COMEMORY_PRUNE_MIN_FEEDBACK",
            low_value_default_below_quality => "COMEMORY_PRUNE_BELOW_QUALITY",
            learning_retention_days => "COMEMORY_LEARNING_RETENTION_DAYS",
            superseded_grace_days => "COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS",
        }
        reinforce { search_edit_days => "COMEMORY_REINFORCE_SEARCH_EDIT_DAYS" }
    ),
}

/// `COMEMORY_API_KEY` override for the device secret stored in `auth.json`.
/// Unset → callers use the on-disk secret from [`crate::domains::sync::AuthFile`].
pub fn api_key_override() -> Option<String> {
    std::env::var("COMEMORY_API_KEY")
        .ok()
        .filter(|v| !v.is_empty())
}

/// `COMEMORY_DAEMON_SUPERVISOR` override: which OS supervisor the sync
/// daemon reports and, for `ensure`, is asked to use (`launchd`, `systemd`,
/// `process`, `external`). Unset → auto-detect.
pub fn daemon_supervisor_override() -> Option<String> {
    std::env::var("COMEMORY_DAEMON_SUPERVISOR")
        .ok()
        .filter(|v| !v.is_empty())
}

/// Whether a user-facing lookup (`search` / `context`) should record access
/// tracking + the `retrieval_log` row this run — the `SearchOptions::track`
/// gate. `true` (the default) for every real invocation; lowered to `false`
/// only when `COMEMORY_DISABLE_ACCESS_TRACKING` is set truthy.
///
/// This is a test hook, not a user knob (mirrors `eval::tune`'s
/// `COMEMORY_TUNE_MIN_GOLDEN`): it lets a stability harness drive the binary
/// repeatedly without each query mutating `access_count` / `last_accessed`,
/// which feeds ACT-R activation and would reorder ranking between calls. An
/// unparsable value errors, naming the variable, like every other env hook.
/// Shared by `cli`, `serve`, and `mcp` so the three cannot drift.
pub fn access_tracking_enabled() -> Result<bool> {
    let disabled = env_parse::<bool>("COMEMORY_DISABLE_ACCESS_TRACKING")?.unwrap_or(false);
    Ok(!disabled)
}

/// True when `COMEMORY_SKIP_MIGRATION_BACKUP` is truthy (`"1"` or
/// `"true"`). Unset, empty, or any other value is `false` — a test/escape
/// hatch hook (mirrors `COMEMORY_DISABLE_ACCESS_TRACKING`'s
/// [`access_tracking_enabled`]), not a validated user knob, so an
/// unrecognized value degrades to the safer default (take the backup)
/// rather than erroring.
///
/// Lives here, not in `store::migrate`, because the `no-direct-env-var`
/// guardrail forbids reading env anywhere outside `config/`/`tests/`, and
/// `connection::open` (the migration preflight's call site) has no
/// `Config` to thread the value through.
pub fn skip_migration_backup() -> bool {
    matches!(
        std::env::var("COMEMORY_SKIP_MIGRATION_BACKUP").as_deref(),
        Ok("1" | "true")
    )
}

#[cfg(test)]
#[path = "tests/env.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/env_2.rs"]
mod tests_2;
