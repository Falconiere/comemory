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
| `claude_code.rs` | `bash_commands_from_jsonl` | Claude Code JSONL → Bash `tool_use` command lines with timestamps |
| `explicit_save.rs` | `extract_explicit_saves` | Recover `comemory save` claims; map engine kinds → product kinds |
| `redact.rs` | `redact_text` / `RedactionAttestation` | Client rule set v1: redact matches and attest `{version,findings}` |
| `rules.toml` | `[[rule]]` | Co-located with `redact.rs` for `include_str!("rules.toml")` (platform wire ids + high-entropy heuristic) |
| `candidates.rs` | `post_candidates` | `POST /v1/sessions/{id}/candidates` batch types + HTTP |
| `distill.rs` | `run` | Orchestrate extract → redact → cap-check → POST (or dry-run) |

Tests live under `tests/`; the real six-save fixture is
`tests/fixtures/claude-code-session-saves.jsonl` (crate-root; guardrails
forbids a `fixtures/` nested folder under `src/`).

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/capture.rs`.
