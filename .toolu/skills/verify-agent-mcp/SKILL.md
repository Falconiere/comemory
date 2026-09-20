---
name: verify-agent-mcp
description: Use when changing MCP transport, embedded agent hooks, or native host installation in comemory.
metadata:
  toolu:
    origin: agent
    created: 2026-09-20T02:17:24Z
---
## When to Use

When changing comemory's MCP transport, embedded agent hooks, or native host installation.

## Procedure

1. Recall relevant repo knowledge, inspect the diff, and identify changed tool contracts and lifecycle hooks. Keep test data and host configurations in temporary directories.
2. Run focused Rust regressions: `cargo nextest run --all-features --lib --test cli_scenario_mcp --test mcp__parity -E 'test(mcp)|test(feedback)|test(simultaneous_first_opens)|test(contended_save)'`.
3. Rebuild with `cargo build --bin comemory` before `bash scripts/test-agent-install.sh`. This harness installs into isolated Claude Code and Codex configurations and exercises shipped hooks. Both native CLIs must be available. The source-only mode (`COMEMORY_TEST_SOURCE_HOOKS_ONLY=1`) is useful during iteration, but does not validate the embedded bundle.
4. Check native MCP discovery with each host's `mcp list` in its isolated configuration. Inspect actual stdio `initialize`, `tools/list`, and `tools/call` results if the catalog or schemas changed.
5. Preserve simultaneous initialization against an absent database in the store tests and real-process regressions in `tests/cli_scenario_mcp.rs`: concurrent distinct and identical saves, and an external rebuild between calls of a still-running client. When changing startup, also launch several stdio clients simultaneously against an absent database. Require one creation for identical content, intact bodies, and visibility from old and new clients.
6. Check mixed memory/code feedback: failed code identity resolution must roll back memory verdicts, and concurrent successful calls must increment counters exactly once each.
7. Update README, AGENTS, the integration guide, and affected folder indexes. Run `bash scripts/check-all.sh` and the Rust/shell suites appropriate to the changed surfaces. Use `--no-fail-fast` for a full nextest sweep so one failure does not leave scenarios unrun. Report unrun checks explicitly.

## Pitfalls

- On macOS, run the repository gates with Bash 4 or newer on `PATH` (for Homebrew: `PATH="/opt/homebrew/bin:$PATH" bash scripts/check-all.sh`). The store and migration checks use `mapfile`, which `/bin/bash` 3 lacks; a failure there is a tooling prerequisite, not evidence of a Rust regression.
- Hooks are compiled into the binary. An old binary tests old hooks even after source edits.
- A valid manifest alone does not prove host discovery or successful MCP calls.
- Recall status aggregates a repository/time window; it cannot attribute activity to one agent. Injected untracked hints have no query ID.
- `context` returns full bodies; `k` is not a token budget. Report measured payload characters/bytes separately from tokenizer counts.
- A drift check alone cannot detect commands the reference generator never emits. Keep the independent clap-tree section check in `tests/cli_scenario_catalog.rs` green.
- Rebuild-between-calls coverage does not establish safety during active writes.
- On macOS, process startup during a full sweep can exceed embed-command deadlines. Reproduce the shell fixture outside comemory, then rerun affected tests after the sweep exits. Report both results; do not raise production timeouts just to make the tests pass.

## Verification

Require passing Rust and native-host runs, documented compatibility limits, and no unexplained changes to real user configuration. See `docs/scenarios/mcp.md` and `docs/guides/agent-integration.md` for current contracts.
