# Sync daemon + exhaustive post-login sync — Design

**Date:** 2026-09-14   **Status:** Approved (amended near-RT)   **Author:** Auto
**Topic:** Move continuous auto-sync onto a required user-level OS daemon
(launchd / systemd --user), and make `auth login`'s first sync drain the
org outbox instead of a single bounded page.

## Problem

Login already runs a first sync, but it is capped at 500 pull + 500 push.
A machine with a large org corpus or a large local outbox still needs a
follow-up `comemory sync`. Continuous sync today is in-process
`[sync] after_save` / `pull_before_context_after` — fragile in short-lived
CLI processes, invisible to operators, and easy to confuse with “cloud is
always up to date.” There is no launchd/systemd unit.

Personal / unbound memories stay local (`skipped_personal`); that rule does
not change (#86 is out of scope).

## Non-Goals

1. **No personal cloud sync** (#86).
2. **No Windows service.**
3. **No filesystem watcher** on `~/.comemory/memories` — the daemon drains
   on an interval (pull-based model). A single `sync.wake` touch file for
   local wake-on-save is not a memories-tree watcher.
4. **No real-time fan-out** / multi-device push notify (no platform SSE/WS
   broker). Near-real-time is **local poll + wake** only — peer devices still
   discover changes on their next daemon cycle.
5. **No replacement of manual `comemory sync`.** It must keep working with
   the daemon stopped.
6. **No engine pin / hosted-serve change** — the user daemon is a laptop
   concern; `comemory serve` on a VPS does not install it.

## Architecture

**Hybrid:** exhaustive sync immediately after `auth login`, then continuous
auto-sync only via a **user-level OS daemon**.

```
auth login
  → write auth.json
  → install+start daemon (unless --no-daemon)
  → full pull then push (loop until empty pages)
daemon (KeepAlive / Restart=always)
  → every daemon_interval (default 5s): pull then push; honor verify_every
  → interruptible sleep: sync.wake ends the wait early
save (labelled repo)
  → local write + touch sync.wake (best-effort; not in-process push)
context
  → local only (no in-process pull-before-context)
```

### Daemon contract

| Piece | Choice |
|-------|--------|
| Units | macOS LaunchAgent `io.comemory.sync`; Linux systemd `--user` `comemory-sync.service` |
| Binary | same `comemory` → `comemory sync daemon run` (foreground; supervisor restarts) |
| Loop | sleep `daemon_interval` (default **5s**): `run_pull` then `run_push`; occasional `verify` per `verify_every` |
| Wake | After successful save of a sync-eligible (non-empty `repo`) memory, touch `$DATA_DIR/sync.wake` so interruptible sleep ends and one pull→push runs immediately |
| Install | plist/unit under user dirs; `WorkingDirectory` + `COMEMORY_DATA_DIR` from the login data dir |
| Login | install if missing → start → full initial sync |
| Logout | **stop** daemon (leave unit installed; next login restarts) |
| Escape | `auth login --no-daemon`; `comemory sync daemon uninstall` |

### Config (`[sync]`)

| Knob | Default | Role |
|------|---------|------|
| `daemon_interval` | `"5s"` | Sleep between daemon pull+push cycles (wake can end early) |
| `verify_every` | `"7d"` | Daemon (and operators) verify interval hint |
| `after_save` | `false` | **Deprecated path** — no longer wired; prefer daemon |
| `pull_before_context_after` | `""` (off) | **Deprecated path** — no longer wired; empty/`0` means off |
| `skip_repos` | `[]` | unchanged |

### Status surfaces

`comemory sync daemon status`, `comemory sync --action status`,
`comemory auth status`, and `comemory doctor` report whether the daemon is
installed/running when credentials are present, and **warn** when linked but
the daemon is not running (“auto-sync inactive”).

## Interfaces

```text
comemory sync daemon install|uninstall|start|stop|status|run
comemory auth login [--no-daemon] [--api-url URL]
```

`run_initial_sync` loops `run_pull` / `run_push` until a page moves nothing
(still pull-before-push; still best-effort at the login CLI boundary).
Login reports `skipped_personal` / `skipped_config` in the sync summary.

## Amendment 2026-09-14 — near-real-time (poll + wake)

**Decision:** default `daemon_interval` **60s → 5s**; add **local wake-on-save**
via `$DATA_DIR/sync.wake`. Keep pull-then-push, org allowlist, silent LWW by
server `seq`, hooks demoted. **No** platform SSE/WS and **no** new
`/v1/sync/*` routes. Near-RT poll+wake ≠ broker (Non-Goal 4 unchanged).

**Latency:** peer lag ≈ one interval + RTT (typically ≤ ~5–10s); local writer
lag ≈ wake latency (sub-second to a few seconds).

## Acceptance

- **AC-login-sync:** Large outbox (>500) drains on one login (or reports
  remaining only on hard failure); personal stays local in skip counts.
- **AC-daemon-required:** Daemon stopped → save does not push; daemon
  running → peer sees it within ≤ one interval + sync latency.
- **AC-near-rt / AC-wake:** Default interval is 5s; labelled save wakes the
  daemon so push starts before a full sleep elapses.
- **AC-login-daemon:** Login without `--no-daemon` leaves daemon status
  = running (macOS or Linux).
- **AC-logout:** Logout stops the daemon; credentials gone.
- **AC-manual:** `comemory sync` works with daemon stopped.
- **AC-docs:** `docs/guides/cloud-sync.md` describes the daemon as the
  auto-sync path (incl. 5s + wake).

## Open questions

None — decisions locked in the sync-daemon-hybrid plan + near-RT amendment.
