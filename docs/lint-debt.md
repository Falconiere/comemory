# Lint debt baseline (`clippy::pedantic` allow-list, `Cargo.toml [lints.clippy]`)

Status: documented baseline, burned down line by line · Owner: whoever fixes
a lint next

DEVIATION D7 (`docs/toolu/specs/folder-structure-migration.md` section 1.2):
adopting `clippy::pedantic` wholesale at the toolu-conventions migration
would have made `-D warnings` a red build (630 warnings measured on
`cargo clippy --lib` before this table existed). Instead, `Cargo.toml`
declares eight pedantic lints at `allow`, each with the count measured at
migration time, so the crate can adopt `pedantic` today without hiding the
gap. Burning a lint down means: fix every site, delete its line from
`Cargo.toml [lints.clippy]`, and the lint goes live under `-D warnings`
immediately. Do not add a lint to this list without the same treatment
(measure on `cargo clippy --lib`, document the count and the reason, commit
both together).

## The eight allowed lints

| Lint | Count | Notes |
| --- | --- | --- |
| `missing_errors_doc` | 202 | One `# Errors` section per fallible public fn. Bulk, but mechanical. |
| `doc_markdown` | 145 | Un-backticked identifiers in doc comments. Largely machine-applicable via `cargo clippy --fix`. |
| `cast_possible_wrap` | 37 | Mostly `usize`/`u64` → `i64` at SQLite boundaries; several become real `TryFrom`. |
| `unused_async` | 25 | Async CLI handlers kept uniform for the dispatcher — may be a legitimate permanent allow; decide deliberately rather than by default. |
| `cast_precision_loss` | 23 | Deliberate in ranking math (`u64`/`usize` → `f64`); likely a permanent, *documented* crate-level allow. |
| `cast_sign_loss` | 21 | SQLite returns `i64` for non-negative counts — `u64::try_from(...)` at the boundary. |
| `cast_possible_truncation` | 18 | Bounded by construction at each site; audit each one. |
| `needless_pass_by_value` | 16 | Signature changes; cheap fix but touches call sites. |

Total: **487**. Counts were re-confirmed against `Cargo.toml`'s current
`[lints.clippy]` D7 block at the time this doc was written and match exactly.
Everything else in the pedantic set is already clean or was fixed during the
migration's lint-table phase. Notably **zero** `too_many_lines`,
`unwrap_used`, `expect_used`, `print_stdout`, `print_stderr`, `todo`,
`unimplemented`, or `dbg_macro` in `src/` — those lints land at their final
(non-allow) severity immediately and carry no line here.

## Also allowed, out of D7 scope

`Cargo.toml` carries one further `clippy::pedantic` allow that is **not**
part of the D7 debt above and should not be merged into it:

- `struct_field_names = "allow"` — 1 hit, `eval::bandit::Arm::arm_id`. The
  field name is a serialized `comemory bandit --json` output key read
  directly by roughly ten assertions across `tests/eval__bandit.rs` and
  `tests/eval__bandit_rng.rs`. Renaming it to drop the struct-name prefix is
  a CLI-output/test-file change, not a lint fix, so it is tracked separately
  and is not counted in the "Total: 487" above.

## Suggested burn-down order

Cheapest and most mechanical first, so the count drops fast and the
remaining lines get harder (and more deliberate) as the list shortens:

1. **`doc_markdown` (145)** — run `cargo clippy --fix` for the mechanical
   majority (un-backticked identifiers), then hand-fix the residue clippy's
   `--fix` mode declines to touch (ambiguous prose, doc examples).
2. **`missing_errors_doc` (202)** — add a `# Errors` section to every fallible
   `pub fn`. Bulk work, but each site is a one-paragraph, low-risk addition;
   good for a dedicated PR that touches nothing else.
3. **`needless_pass_by_value` (16)** — smallest count with a real code
   change (borrow instead of move). Touches call sites, so land it before
   the cast lints below in case any signature it changes also has a cast at
   the call site.
4. **`cast_possible_truncation` (18)** — audit each site; most are already
   bounded by construction and become a `debug_assert!` plus a comment, or a
   `TryFrom` at the one or two sites that are not provably bounded.
5. **`cast_possible_wrap` (37)** — the SQLite `usize`/`u64` → `i64` boundary
   sites. Several are mechanical `i64::try_from(...).map_err(...)?`
   conversions; a handful are `as i64` casts that are provably safe and get a
   short comment plus a narrower per-expression `#[allow]`-free fix (bound
   the value first, then cast).
6. **`cast_sign_loss` (21)** — the mirror image of the previous line: SQLite
   returns `i64` for counts that are always non-negative. Convert at the
   boundary with `u64::try_from(...)?` rather than `as u64`.
7. **`cast_precision_loss` (23)** — ranking-math `u64`/`usize` → `f64`
   conversions. Likely ends as a **permanent, documented** per-expression
   allow rather than a fix, since floating-point ranking scores do not need
   exact integer precision above 2^53 elements — decide deliberately when
   this line is reached rather than reflexively fixing every site.
8. **`unused_async` (25)** — async CLI dispatch handlers kept `async fn` for
   signature uniformity even where a given handler does no `.await`. Likely
   ends as a **permanent, documented** allow for the same reason as
   `cast_precision_loss` — decide deliberately, don't strip `async` from a
   handler just to silence the lint if it breaks dispatcher uniformity.

Re-measure with `cargo clippy --lib` before starting any line above — these
counts drift as the crate grows, and a stale count is worse than none.
