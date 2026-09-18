# `domains/capture/`

**What belongs here:** client-side session capture and distillation against the
comemory.io platform (Slices 3–4). Transcripts never leave the machine; this
capability reads them, redacts free text with a versioned attestation, and posts
receipts / candidate batches over the org-scoped credential. It also owns the
Claude Code `SessionEnd` contract on both ends — the command written into the
tool's settings file, and the payload that command hands back on stdin.

**What does NOT belong here:** delivery or acceptance. Clap flags, stdin
acquisition, every rendered line and the exit code stay in
[`cli/`](../../cli/README.md) (`cli/capture.rs`, `cli/distill.rs`); accept/reject
of candidates is workspace-key / console only; server-side extraction, the local
`comemory save` path and sync push/pull belong to
[`domains/sync/`](../sync/README.md); installing the comemory **plugin bundle**
is integrations' (#175), not this capability's.

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `claude_code.rs` | `load_path` / `bash_commands_from_jsonl` | Claude Code JSONL → session metadata + Bash `tool_use` lines |
| `receipt.rs` | `build_receipt` | Redact + SHA-256 digest → Slice 3 wire receipt |
| `client.rs` | `post_session` / `list_sources` | `POST /v1/sessions`, `GET /v1/capture/sources` |
| `run.rs` | `run_capture` / `run_sources` | Capture orchestration + unimplemented-source guard |
| `hook.rs` | `install` / `session_end_target` | The `SessionEnd` contract: install the command, decode the payload it returns |
| `explicit_save.rs` | `extract_explicit_saves` | Recover `comemory save` claims; map engine kinds → product kinds |
| `redact.rs` | `redact_text` / `RedactionAttestation` | Client rule set v1 for receipt and candidate text |
| `rules.toml` | `[[rule]]` | Co-located with `redact.rs` (`include_str!`) |
| `candidates.rs` | `post_candidates` | `POST /v1/sessions/{id}/candidates` batch types + HTTP |
| `distill.rs` | `run` / `requires_credentials` | Orchestrate extract → redact → cap-check → POST (or dry-run) |

## Boundaries worth knowing

`rules.toml` here and [`domains/sync/rules.toml`](../sync/rules.toml) are two
**intentionally different** rule sets, not a duplication to merge: sync scans
memory bodies before a push, capture scans transcript free text before a
receipt, and each carries its own independently versioned attestation.

Credentials and the platform base URL come from `domains::sync` by concrete path
(`domains::sync::{auth_file, client}`); transport errors reuse
`utilities::http_error::map_reqwest` rather than a second client framework.

CLI entries are `src/cli/capture.rs` and `src/cli/distill.rs`, both CLI-only and
listed in `serve::routes::meta::CLI_ONLY` — no `/api/v1` route.

Tests live under `tests/`; fixtures are crate-root
(`tests/common/fixtures/claude-code-session.jsonl`,
`tests/fixtures/claude-code-session-saves.jsonl`). The colocated `tests/`
directory is the suite home allowed by `src.nested["domains/*"]` — it is **not**
a module folder and must **not** appear in `requireReadme`. A test that would
touch the real `~/.claude`, the real machine or a daemon belongs in crate-root
`tests/`, where the harness gives it an isolated `HOME`.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/capture.rs`.
