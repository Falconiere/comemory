# capture/

**What belongs here:** client-side session capture and distillation against the
comemory.io platform (Slices 3–4). Transcripts never leave the machine; this
module reads them, redacts free text with a versioned attestation, and posts
receipts / candidate batches over the device-key credential.

**What does NOT belong here:** accept/reject of candidates (workspace key /
console only), server-side extraction, the local `comemory save` path, or
sync push/pull (`sync/`).

## Contents

| File | Primary item | Purpose |
| --- | --- | --- |
| `claude_code.rs` | `load_path` / `bash_commands_from_jsonl` | Claude Code JSONL → session metadata + Bash `tool_use` lines |
| `receipt.rs` | `build_receipt` | Redact + SHA-256 digest → Slice 3 wire receipt |
| `client.rs` | `post_session` / `list_sources` | `POST /v1/sessions`, `GET /v1/capture/sources` |
| `run.rs` | `run_capture` / `run_sources` | Capture orchestration + unimplemented-source guard |
| `hook.rs` | `install` | Claude Code `SessionEnd` → `capture session --from-hook` |
| `explicit_save.rs` | `extract_explicit_saves` | Recover `comemory save` claims; map engine kinds → product kinds |
| `redact.rs` | `redact_text` / `RedactionAttestation` | Client rule set v1 for distill candidate text |
| `rules.toml` | `[[rule]]` | Co-located with `redact.rs` (`include_str!`) |
| `candidates.rs` | `post_candidates` | `POST /v1/sessions/{id}/candidates` batch types + HTTP |
| `distill.rs` | `run` | Orchestrate extract → redact → cap-check → POST (or dry-run) |

CLI entries: `src/cli/capture.rs` and `src/cli/distill.rs` (both CLI-only;
listed in `serve::routes::meta::CLI_ONLY`).

Tests live under `tests/`; fixtures are crate-root
(`tests/common/fixtures/claude-code-session.jsonl`,
`tests/fixtures/claude-code-session-saves.jsonl`). The colocated `tests/`
directory is the suite home allowed by `src.nested["*"]` — it is **not** a
module folder and must **not** appear in `requireReadme`.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/capture.rs`.
