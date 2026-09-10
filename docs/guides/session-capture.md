# Session capture

Passive capture of coding-session **receipts** (metadata + redaction
attestation). Raw transcripts stay on the machine; only a small receipt is
posted to the platform (`POST /v1/sessions`).

Requires `comemory auth login` and **per-source consent** granted in the
console (or with a workspace API key). This CLI can only *read* consent.

## Capture one session

```bash
# Dry-run against an explicit Claude Code JSONL
comemory capture session --path ~/.claude/projects/.../<session>.jsonl --dry-run --json

# Look up by session id under ~/.claude/projects
comemory capture session --session-id 8e9f54e3-a984-46ab-8403-135ee920cbca

# POST (fails locally if redaction found secrets unless --allow-secret)
comemory capture session --path ./session.jsonl --allow-secret
```

## Consent

```bash
comemory capture sources
```

Document indexing still uses `comemory sources` / `comemory index` — different domain.

## SessionEnd hook

```bash
comemory capture install-hook
# or: comemory capture install-hook --settings /path/to/.claude/settings.json
```

Installs a Claude Code `SessionEnd` command that runs
`comemory capture session --from-hook`. Failures in the hook should not block
session end (best-effort).

## Redaction

Client rule-set version `1` (attested as `redaction.version`). Rules include
the platform format-anchored catalogue plus client-only `generic-high-entropy`.
The digest is SHA-256 of the **redacted** transcript bytes.
