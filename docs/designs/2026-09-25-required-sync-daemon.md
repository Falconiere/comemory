# Required resident sync daemon — Design

**Date:** 2026-09-25   **Status:** Approved   **Author:** Auto
**Topic:** One always-running, self-healing sync coordinator per data directory
(issue 257, epic 248). Supersedes the opt-in daemon of
`2026-09-14-sync-daemon.md`.

## Problem

The user requires the sync daemon to always run. Today it is opt-in
(`auth login --daemon`), logout stops it, and nothing repairs it. Its loop
(`domains/sync/daemon.rs`) is a blocking poll with a plain sleep. The pull
side of the workspace channel lives in a second loop (`watch.rs`) that only
`comemory watch` runs. Git hooks start a whole in-process pass per commit.
Nothing can tell whether a daemon is alive, which binary it runs, or which
data directory it serves: `status` asks launchd or systemd, not the process.

Epic 248 needs a local service that is always there: before login, after
logout, after a crash, and on hosts with no service manager. Replication
(issues 250–256) is useless without one. The installer work (issue 258) and
the Homebrew work (homebrew-tap 1) call the `ensure` command this issue adds.

## Non-Goals

1. **No command-core change.** `save`, `search`, `sync`, and the other
   commands keep running their cores in-process. They are not converted to
   daemon RPCs. Two commands change from doing the work themselves to asking
   the coordinator: the hook-fired `sync --action auto` (A-5) and `watch`
   (A-9). A local write only adds a best-effort `wake` after its unchanged
   core.
2. **No installer or upgrade wiring.** `install.sh`, `scripts/dev-install.sh`,
   `comemory upgrade`, stable executable paths, and Homebrew belong to issue
   258 and homebrew-tap 1. This issue ships the `ensure` they call.
3. **No real launchd or systemd tests in CI.** Real supervisor lifecycle
   checks on disposable users are issue 258 (`scripts/test-daemon-install.sh`).
   This suite uses the `process` supervisor, which runs the same coordinator.
4. **No platform change.** The workspace channel, its ticket route, and the
   live harness cases stay in `CodaSignal/comemory.io`.
5. **No new sync semantics.** Passes call the existing `auto::run_pass` and
   `drain::drain` (issue 255) unchanged, with two exceptions. A stop check at
   every batch boundary (the barrier, or the coordinator shutting down) ends
   a pass with a new `end: cancelled`. And `watch`'s channel loop hands each
   nudge to a callback instead of pulling itself.
6. **No Windows service.** Unsupported targets report `unsupported` and fail
   `ensure` with a clear message.
7. **No rebuild lock order.** Issue 256 owns that. The coordinator only
   promises not to hold a database connection between passes.

## Architecture

```text
 installer / any CLI command ──preflight──▶ ensure ──▶ probe socket (auth handshake)
                                              │ unhealthy
                                              ▼
                              daemon-ensure.lock ─▶ re-probe ─▶ repair:
                                   stop stale owner · write/refresh unit · start
                                   (launchd | systemd --user | process) · wait ready

 coordinator  (`comemory sync daemon run`, one per canonical data dir, holds daemon.lock)
   ├─ control server (tokio, Unix socket)   status · wake · catch_up · reload · subscribe · shutdown
   ├─ reconciliation ticker (daemon_interval, 5s)  ─┐
   ├─ channel follower (workspace WebSocket nudges) ─┼─▶ pending set (coalesced) ─▶ worker thread
   ├─ hook / save wakes over the socket            ─┘        one pass at a time, under sync.lock:
   ├─ signal handler (SIGTERM/SIGINT graceful, SIGHUP reload)   auto::run_pass (+ verify when due)
   └─ watchdog (data dir gone ⇒ exit 0)
```

**Chosen approach:** one coordinator process per canonical data directory,
started by an OS supervisor where one is usable and by the CLI itself where
not. Every ordinary CLI startup verifies it with an authenticated socket
handshake and repairs it within a fixed bound. The decisive trade-off is
repair-on-startup against supervisor-only liveness. A supervisor alone cannot
notice a hung process, a foreign socket, or a stale binary, and it does not
exist in containers. Checking on every command costs one Unix-socket round
trip (about a millisecond) and makes the service self-healing wherever the
CLI runs.

The coordinator takes `daemon.lock`, binds, and answers `status` before it
reads credentials or makes any network call. Remote or offline
authentication therefore never delays local readiness. It opens a database
connection per pass and closes it afterwards.

**Reused:** `auto::run_pass` (the cycle body), `drain::drain`, `verify::verify`,
`watch`'s channel loop (split so a nudge calls a callback instead of pulling),
`FileLock` (`daemon.lock`, `daemon-ensure.lock`, and the existing `sync.lock`),
`daemon_unit`/`daemon_templates` (the unit rendering, now per data directory),
`utilities::digest::sha256_hex`, and tokio's `UnixListener` and `signal::unix`.
No new dependency.

### Decisions (recorded because nobody answers questions during the epic)

| # | Decision | Reason |
|---|---|---|
| D1 | `COMEMORY_SYNC_DAEMON=0` stays, but only as the **test/CI harness switch**. It skips the *implicit* preflight and login/logout signalling. It does not affect `ensure`, `restart`, `repair` or `run`. `status` and `doctor` report `disabled`. `.cargo/config.toml [env]` sets it for `cargo test`/`nextest`, following the `COMEMORY_INDEXING_AUTO_REINDEX=off` precedent. User docs describe it as a harness variable, not an opt-out. | 117 test files run the real binary against the developer's real `HOME`. Default preflight would register launchd units in the developer's session and race test assertions. Installed binaries never read `.cargo/config.toml`. (Jev 0.75.) |
| D2 | With no usable user service manager (no launchd GUI domain, no systemd user bus, uid 0, or an unwritable unit location), `ensure` starts the coordinator as a detached background process (`supervisor: process`). Every later CLI startup re-verifies it. It reports `supervisor` and the reason in `notes`. `COMEMORY_DAEMON_SUPERVISOR=external` makes `ensure` verify-only and fail with the foreground-`daemon run` instruction. | Satisfies "foreground supervision or actionable failure; never silent opt-out". The container image and CI hosts keep a resident coordinator. (Jev 0.70.) |
| D3 | The hook-fired `sync --action auto` sends an authenticated `wake{checkout}` over the socket and exits when the coordinator is healthy. It keeps the in-process pass only when the harness switch disables the daemon. | This is A-5's "post-commit Unix-socket notification". Installed hook scripts need no rewrite. (Jev 0.72.) |
| D4 | If readiness cannot be established within the bound, an ordinary command fails with exit 69 and names the fix. | A-3 literally. (Jev 0.96.) |
| D5 | Manual `comemory sync` keeps its in-process core, serialized with coordinator passes by the shared `sync.lock`. | "Asks for a pass without racing another uploader", with no RPC conversion. (Jev 0.88.) |
| D6 | The logout barrier is a durable file, `auth.disabled`, checked inside `AuthFile::load`/`load_usable` and `login::status`, and at every drain batch boundary. `auth login` clears it (`AuthFile::save`). | One chokepoint covers every exchange path in every process, including inherited `COMEMORY_API_KEY`. The same check cancels in-flight passes at their next boundary. |
| D7 | Logout clears `auth.json` and the coordinator's in-memory channel/session. Durable cursors and outbox rows stay, stamped with the outgoing key. | Re-login to the same workspace resumes. Keying (issue 255) already stops another workspace from receiving them. |
| D8 | Channel nudges are proven end to end by the live platform cases `propagation` and `lost-nudge` (real workerd). In-repo evidence is the follower's existing unit tests plus the coordinator wiring, driven through the real socket, ticker and hook paths. | The engine hub (`comemory serve`) has no workspace channel, and a canned WebSocket server cannot be the evidence (harness design, issue 249). |
| D9 | The coordinator never creates or migrates the corpus. With `comemory.db` absent it idles (`store: absent`). With migrations pending, or the schema too new, it idles until a writable command opens it. | A-7: a read-only MCP/HTTP preflight must not cause corpus creation or migration. |
| D10 | Launchd `KeepAlive = {SuccessfulExit = false}` and systemd `Restart=on-failure`. A graceful stop exits 0 and is not respawned, while a crash is. The next CLI startup restarts a gracefully stopped coordinator. | `stop`/`uninstall` stay usable by package managers but are never a persistent opt-out (A-8). SIGKILL recovery comes from both the supervisor and preflight. |
| D11 | `auth logout` runs the preflight **best-effort**. If the service cannot be made ready, logout still raises the barrier, clears `auth.json`, exits 0, and reports `daemon.running: false` with a stderr warning. | Failing logout with 69 would leave a usable secret on disk. A-3 targets data commands. (Jev 0.87.) |
| D12 | `doctor` never runs `ensure`. Its `sync daemon` check is `ok` for a verified coordinator, `warn` for `disabled`, and `fail` otherwise, naming `comemory sync daemon ensure`. | Introspection-only (A-7), and it states the required service's health honestly. (Jev 0.86.) |
| D13 | `ensure` replaces a verified coordinator whose **binary identity** — `(version, canonical binary path)` — differs from the calling binary. Preflight does not; it replaces only a coordinator whose `binary` file no longer exists. | One tuple covers upgrade skew and relocation, and tests can produce it with a copied binary. Preflight must not flip-flop between two installed binaries. |

## Interfaces / Schema

### Commands

```text
comemory sync daemon ensure   [--json]  idempotent: verify, repair, start; exit 0 ready, 69 not ready
comemory sync daemon status   [--json]  live readiness + supervisor view; starts nothing
comemory sync daemon restart  [--json]  graceful shutdown of the live coordinator, then ensure
comemory sync daemon repair   [--json]  rewrite this data dir's service definition, clear stale
                                        runtime files, restart unless a verified current one runs
comemory sync daemon run                foreground coordinator — the supervisor entry (headless/container)
comemory sync daemon stop     [--json]  graceful shutdown; the next comemory command restarts it
comemory sync daemon uninstall [--json] package-removal hook: stop + remove this data dir's unit;
                                        the next comemory command reinstalls it
comemory sync daemon install|start      hidden deprecated aliases of repair|ensure
comemory auth login --daemon            hidden deprecated no-op; stderr warning
```

No `--no-daemon` flag exists on any command.

### `ensure --json` (stdout; also the success shape of `restart`/`repair`)

```json
{ "ready": true,
  "action": "none | started | restarted | replaced",
  "supervisor": "launchd | systemd | process | external",
  "notes": ["systemd --user unavailable: <reason>; using process supervision"],
  "daemon": { "...readiness below..." } }
```

On failure: stdout `{"ready": false, "supervisor": "...", "error": "..."}`
(with `--json`), stderr `error: sync daemon not ready: <cause> — <fix>`,
exit 69.

### Readiness (the `status` op result; never contains a secret or the token)

```json
{ "protocol": 1, "version": "0.49.0", "binary": "/abs/path/comemory",
  "pid": 4242, "instance": "9f0c1d2e3a4b5c6d", "started_at": "2026-09-25T10:00:00Z",
  "data_dir": "/Users/u/.comemory", "socket": "/Users/u/.comemory/daemon.sock",
  "supervisor": "launchd | systemd | process | external | foreground",
  "store": "absent | ready | migration_pending | too_new | error",
  "auth": { "state": "logged_out | authenticated | logged_out_barrier | unusable",
            "api_url": "…", "organization_slug": "…", "workspace_id": "…", "key_prefix": "cmk_ab" },
  "sync": { "state": "idle | running | stopping", "interval_secs": 5, "passes": 12,
            "queued": false, "channel": "off | connecting | connected | backoff",
            "last_pass": { "trigger": "tick | wake | hook | channel | catch_up | reload",
                           "finished_at": "…", "end": "caught_up", "pulled": 0,
                           "pushed": 1, "error": null },
            "last_verify_at": "…" } }
```

### `status --json`

```json
{ "state": "running | not_running | stale | disabled | unsupported",
  "detail": "one line", "version_matches": true,
  "supervisor": { "kind": "launchd", "unit_path": "…", "installed": true },
  "daemon": { "...readiness, when state = running..." } }
```

`stale` covers something at the lock or socket path that fails the handshake,
answers for another directory, speaks another protocol, or stops answering
within the probe bound. `disabled` means `COMEMORY_SYNC_DAEMON=0`.
`unsupported` means the OS has no support.

### Control protocol (`protocol: 1`)

Newline-delimited JSON over a Unix stream socket. A frame is at most 64 KiB.
Every connection authenticates both ways against the 64-hex token in
`<data_dir>/daemon.token` (mode 0600, created once, never sent over the
socket):

1. C→S `{"hello":1,"nonce":NC}`
2. S→C `{"hello":1,"nonce":NS,"proof":sha256("comemory-daemon-server\0"+NC+"\0"+TOKEN)}`
   The client verifies this before it sends anything else.
3. C→S `{"proof":sha256("comemory-daemon-client\0"+NS+"\0"+TOKEN),"op":OP}`
4. S→C `{"ok":true,"result":…}` / `{"ok":false,"error":"…"}`. `subscribe`
   then streams `{"event":…}` lines.

| `op` | Effect | Result |
|---|---|---|
| `{"status":{}}` | none (in-memory state only) | readiness |
| `{"wake":{"checkout":null\|"/path","reason":"hook\|save"}}` | merge into pending set | `{"queued":true}` |
| `{"catch_up":{}}` | queue a pass. Answer when a pass that *started after* this request ends without `more` | last pass summary |
| `{"reload":{}}` | re-read auth/config. Restart or stop the channel. Queue a catch-up | readiness |
| `{"subscribe":{}}` | stream `pass_started`, `pass_finished{pulled,pushed,end,error}`, `channel{state}` | event lines |
| `{"shutdown":{}}` | graceful exit (bounded) | `{"stopping":true}` |

### Runtime files (under the canonical data directory)

| File | Owner | Meaning |
|---|---|---|
| `daemon.lock` | coordinator (flock, lifetime) | one coordinator per data dir |
| `daemon-ensure.lock` | ensure/repair (flock, bounded) | at most one repair at a time |
| `daemon.token` | first ensure or coordinator | handshake secret, 0600 |
| `daemon.json` | coordinator | runtime record `{pid, instance, socket, version, binary, started_at}`. Only used for socket discovery; identity comes from the handshake |
| `daemon.sock` | coordinator | socket, 0600, when the path is ≤ 100 bytes |
| `auth.disabled` | `auth logout` | durable barrier `{"at":…,"reason":"logout"}` |
| `sync-verify.last` | coordinator | RFC 3339 time of the last successful verify |
| `logs/sync-daemon.{out,err}.log` | supervisor/process spawn | coordinator stdio |

**Long paths.** If `<data_dir>/daemon.sock` is over 100 bytes, the socket goes
to `<base>/comemory-<uid>/<id>.sock`. `<base>` is `$XDG_RUNTIME_DIR`, else
`$TMPDIR`, else `/tmp`. `<id>` is the first 16 hex of sha256(canonical data
dir). The `comemory-<uid>` directory is created 0700 and must be owned by the
data directory's owner with mode exactly 0700; otherwise the result is an
actionable failure. A path still over 100 bytes fails with "set TMPDIR to a
shorter directory". Clients use `daemon.json`'s `socket` when the primary path
is not the one in use. Before connecting, the client checks that the socket is
owned by the same uid as `daemon.token` and that its parent directory is not
group- or world-writable.

### Service definitions (per data directory)

- Identity: `<id>` = first 12 hex of sha256(canonical data dir).
  launchd label `io.comemory.sync.<id>` in
  `~/Library/LaunchAgents/io.comemory.sync.<id>.plist`. systemd unit
  `comemory-sync-<id>.service` in `~/.config/systemd/user/`.
- Arguments: `<exe> --data-dir <canonical> sync daemon run`. Environment:
  `COMEMORY_DATA_DIR`, `COMEMORY_DAEMON_SUPERVISOR=<kind>`. No inherited
  credentials are copied.
- A legacy `io.comemory.sync` / `comemory-sync.service` naming the same data
  directory is booted out and removed by `ensure`.

### Configuration and environment

| Name | Change |
|---|---|
| `[sync] daemon_interval` | Now the reconciliation interval (default `5s`, minimum `1s`). Re-read every tick |
| `[sync] verify_every` | Scheduled verify interval, measured from `sync-verify.last` |
| `[sync] push_on_save`, `push_on_save_timeout` | Only govern the inline push. When the inline push does not finish the outbox, the CLI sends `wake{reason:"save"}`. Neither disables the daemon |
| `COMEMORY_SYNC_DAEMON=0` | Harness switch (D1) |
| `COMEMORY_DAEMON_SUPERVISOR` | `launchd`/`systemd`/`process`/`external`. Unset means auto-detect |

### Preflight classification (exhaustive `match` in `cli::daemon_preflight`)

- **Exempt:** `serve`, `completions`, `upgrade` (incl. `--check`), `doctor`,
  `auth status`, `sync --action status`, and every `sync daemon` subcommand.
  `--help` and `--version` exit inside clap before dispatch.
- **Preflight (fail-closed, D4):** every other command, including `mcp`
  (control plane only, D9), `auth login`, `sync --action auto`, and `watch`.
- **Preflight (best-effort, D11):** `auth logout`.

**Bounds.** The probe (connect, handshake, `status`) has 2 s. Preflight
repair has 10 s in total, including the readiness wait. `ensure`, `restart`
and `repair` have 20 s. Preflight writes nothing to stdout (`mcp`'s stdout is
the JSON-RPC stream); its notes go to `tracing` on stderr. Preflight and
`ensure` may create the data directory itself (mode 0700) and the runtime
files listed above. They never create `comemory.db` or `memories/`.

### Command sequences and output changes

| Command | Sequence | Output change |
|---|---|---|
| `auth login` | preflight → device flow → `AuthFile::save` (writes `auth.json`, then removes `auth.disabled`) → `reload` op → the existing inline first sync | JSON `daemon: {"running": bool, "skipped": false, "instance": str\|null}`. TTY `daemon: running (pid N, vX)` or the not-ready reason |
| `auth logout` | best-effort preflight → raise `auth.disabled` (fsync) → wait for `sync.lock` up to `request_timeout + 5s` → `forget` (stamp owed rows, remove `auth.json`) while holding it → `reload` op | JSON `{"logged_out": true, "daemon": {"running": bool, "in_flight": "none \| drained \| timed_out"}}` (replaces `daemon_stopped`). TTY `logged out (credentials removed; sync daemon still running)` |
| `auth status` | exempt; `status` probe only | the `daemon` field becomes the `status --json` object |
| `sync --action status` | exempt; `status` probe only | same `daemon` object |
| `sync --action auto` | preflight → `wake{checkout, reason:"hook"}` → exit 0 | JSON `{"action":"auto","delivered":"daemon","instance":"…"}`. With the harness switch, the existing in-process pass and its JSON |
| `save` / `delete` / other writes | unchanged core → inline push (if enabled) → `wake{reason:"save"}` when the inline push was disabled, skipped, failed or ended with `more` | none |
| `watch [--once]` | preflight → `subscribe`. `--once` also sends `catch_up` and exits after its result | unchanged lines: `{"event":"connected","pulled":0}`, `{"event":"pulled","pulled":N}`. A catch-up that ended on the network exits 69 |
| `doctor` | exempt (D12) | the `sync daemon` check follows D12 |

**Harness switch (`COMEMORY_SYNC_DAEMON=0`).** Preflight is skipped.
`sync --action auto` runs its in-process pass. `watch` runs the existing
in-process channel follower. Login and logout send no ops, and logout
reports `daemon.running: false`. Writes send no wake. `status` and `doctor`
report `disabled`.

### Library surface (`src/domains/sync/daemon/`)

| Module | Primary items |
|---|---|
| `control.rs` | `Request`, `Op`, `Response`, `Readiness`, `PROTOCOL` |
| `handshake.rs` | `server_proof`, `client_proof`, token load/create |
| `socket_path.rs` | `resolve(paths) -> SocketPlan`, ownership checks |
| `client.rs` | `probe(paths, bound) -> Probe`, `wake`, `catch_up`, `reload`, `subscribe`, `shutdown` |
| `server.rs` | accept loop and op dispatch |
| `state.rs` | shared coordinator state and events |
| `worker.rs` | coalesced pending set and the pass thread |
| `coordinator.rs` | `run(paths) -> Result<()>`: lock, bind, tasks, signals, watchdog |
| `ensure.rs` | `ensure(paths, Intent) -> Result<Ensured>` (`Intent::{Preflight, Ensure, Restart, Repair}`) |
| `supervisor.rs` | backend detection and start/stop/remove per backend |
| `spawn.rs` | detached `process` spawn |
| `barrier.rs` | `raise`, `active`, `clear` for `auth.disabled` |
| `runtime_record.rs` | `daemon.json` read/write |

The existing `daemon_unit.rs`/`daemon_templates.rs` keep unit I/O and
rendering, parameterized by `<id>`. `store` gains a read-only
`store::readiness::probe(db_path) -> StoreReadiness` (open read-only, compare
applied markers with expected ones), so the coordinator never migrates.

## Failure modes and edge cases

| Situation | Observable behavior | Propagation |
|---|---|---|
| No coordinator; many CLIs start at once | Each probe fails. They serialize on `daemon-ensure.lock` and re-probe; one repair starts one coordinator. A second `daemon run` that races in exits 75 on `daemon.lock` | recovered |
| Coordinator SIGKILLed | Stale socket and record remain. Supervisor restart or the next preflight binds after unlinking the socket (it now holds `daemon.lock`) | recovered |
| Socket path answered by a foreign process, another directory's coordinator, or a symlink | Handshake proof or `data_dir` mismatch counts as `stale`. Repair replaces it once `daemon.lock` is free; otherwise `stale` with the owner pid, and exit 69 after the bound | converted |
| Coordinator alive but not answering within 2 s (hung) | `stale`. Explicit `restart`/`repair` and preflight send SIGTERM to the `daemon.json` pid only if that pid is alive and its executable name is `comemory`; SIGKILL after 5 s | recovered or 69 |
| Different binary answering (upgrade skew, relocation) | Preflight accepts any verified same-protocol coordinator whose `binary` file still exists. `ensure`/`restart`/`repair` replace one whose identity differs (D13) | converted |
| `daemon.json` names a live pid of another executable | That pid is never signalled; identity comes only from the handshake | converted |
| Socket file unlinked or replaced while the coordinator runs ("broken socket") | Each watchdog tick checks that the socket path is still its own inode and re-binds if not. Clients see `not_running` for at most one tick | recovered |
| Data directory removed | Watchdog exits 0 within one tick, removing socket and record | recovered |
| Logout while the service cannot be made ready | Logout proceeds (D11): barrier raised, credentials removed, `daemon.running: false`, warning on stderr, exit 0 | converted |
| Long data-dir path; runtime dir unsafe | Fallback socket directory; an unsafe or over-long path fails with a named fix | 69 |
| No service manager / unit location unwritable | `process` supervision plus a `notes` entry; `external` fails verify-only with the `daemon run` instruction | converted |
| Logged out, never logged in | Coordinator healthy; `auth.state=logged_out`; ticks refresh hooked repos only | — |
| Logout during a pass | Barrier written first. The pass stops at its next batch boundary (`end: cancelled`). Logout waits for `sync.lock` up to `request_timeout + 5s`, reports `in_flight: none, drained or timed_out`, clears credentials, and sends `reload`. The coordinator stays up | bounded |
| `COMEMORY_API_KEY` inherited after logout | Barrier makes every `AuthFile` load and `auth status` report logged out; no request leaves | converted |
| Org switch | Existing keying stamps owed rows with the old key. The coordinator loads auth fresh at every pass and cancels a pass whose key changed on `reload` | recovered |
| DB absent / migration pending / too new | Coordinator idles with `store` state; no file created or migrated | — |
| Pass blocked on network | Socket answers from in-memory state; the pass ends by `request_timeout`/`pass_budget`. No SQLite transaction is open during HTTP, so a CLI `save` completes | — |
| >2,000 pending ops; upstream down then back | Ticks keep draining without a nudge; status answers throughout | recovered |
| Laptop suspend (SIGSTOP/SIGCONT) | Clients time out while stopped; after resume the coordinator answers and the next tick drains | recovered |
| SIGTERM / SIGINT | Stop accepting, cancel the pass at its boundary, wait up to 15 s, remove socket and record, exit 0 | — |
| SIGHUP | Treated as `reload` | — |
| Config file invalid | Ticks log and keep the last good interval; readiness reports `last_pass.error` | converted |
| Hook fires while coordinator unreachable | Preflight repairs first; if that fails, exit 69 (the hook discards output) | 69 |
| `watch` with coordinator restarting | Subscription drops; `watch` re-runs preflight and re-attaches with backoff | recovered |
| Unsupported OS | `status` reports `unsupported`; `ensure` exits 69 | 69 |

## Acceptance criteria

- **AC-1:** For a real data directory, `sync daemon ensure --json` yields one
  coordinator whose readiness names protocol `1`, this binary's version and
  canonical path, its pid, instance and start time, the canonical data dir,
  and its sync/auth/store state. The readiness pid is a live process whose
  command line is `… --data-dir <dir> sync daemon run`. Neither `status
  --json`, `daemon.json` nor the coordinator logs contain the credential
  secret, the `COMEMORY_API_KEY` value, or the `daemon.token` value. A second
  `daemon run` on the same directory exits 75 while the first keeps
  answering. Addressing the directory through a symlink or a relative path
  reports the same instance and the canonical `data_dir`. Two directories get
  two coordinators. A client that presents a wrong proof gets no readiness,
  and the connection is closed.
- **AC-2:** `ensure` is idempotent (`action: none` on the second run, same
  instance). Eight concurrent `ensure` calls on a fresh directory all report
  the same instance, and exactly one coordinator process exists for that
  directory. A verified coordinator started from a copy of the binary at
  another path is replaced by `ensure` from the original (`action:
  replaced`, new instance, `binary` = the caller's canonical path). Preflight
  from the original leaves that same coordinator running.
- **AC-3:** After the coordinator is SIGKILLed, an ordinary command
  (`list --json`) restores a verified coordinator before returning. With
  `COMEMORY_DAEMON_SUPERVISOR=external` and nothing running, the same command
  exits 69 within 12 s, with a message naming `comemory sync daemon run`.
  Unreachable platform credentials (hub stopped) never fail local readiness
  or `search`.
- **AC-4:** A never-logged-in coordinator stays healthy and idle: over
  several ticks it reports `auth.state = logged_out`, `channel: off`, and no
  exchange pass. Writing a credential (as `auth login` does) plus `reload`
  flips `auth.state` to `authenticated`, and a catch-up pulls hub data with no
  other command. During an in-flight pass (proxy stalls a request),
  `auth logout` returns within `request_timeout + 5s` and reports the
  in-flight outcome. Afterwards the same instance is running with
  `auth.state = logged_out_barrier` and `channel: off`, `auth.json` is gone,
  and `auth.disabled` exists. With `COMEMORY_API_KEY` set for the coordinator
  and CLI, no request reaches the hub across later ticks, and still none
  after the coordinator restarts, until `AuthFile::save` clears the barrier.
  A second workspace logged in afterwards never receives the first
  workspace's memory.
- **AC-5:** A real commit in a repository with installed hooks makes the
  hook-fired `sync --action auto --path <repo>` return after queuing a wake,
  and the coordinator indexes that checkout (`repos --json` shows the new
  HEAD). If the test holds `sync.lock` while 50 wakes arrive, then releases
  it, exactly 2 passes run: the one blocked on the lock and one coalesced
  successor. Subscribed `pass_started`/`pass_finished` events strictly
  alternate. With no config, readiness reports `interval_secs: 5`. A hub
  write reaches the client by the reconciliation tick (`daemon_interval =
  1s`) with no nudge and no command. Against an engine hub, which serves no
  workspace channel, readiness reports `channel: backoff` while ticks keep
  draining. Channel nudges end to end are the live `propagation` and
  `lost-nudge` cases (D8). The socket answers `status` in under 1 s while a
  pass is blocked on a stalled upstream, and that pass ends with a network
  end within `request_timeout` plus 5 s.
- **AC-6:** SIGTERM exits 0 and removes the socket and record. Each of these
  is rejected by identity and then repaired: a stale regular file at the
  socket path, a symlink to another directory's coordinator, a foreign
  listener at the socket path, and a `daemon.json` naming a live non-comemory
  pid (that pid survives). A foreground `daemon run` started immediately
  after a SIGKILL, as a supervisor restart does, binds over the leftover
  socket and answers. If the socket file is unlinked while running, the
  coordinator re-binds within one tick. A data directory whose socket path
  exceeds 100 bytes gets a working coordinator through the private runtime
  directory. If that runtime directory already exists with mode 0777,
  `ensure` refuses it with a named fix and binds nothing there. A changed
  `daemon_interval` is reported after the next tick. While the fault proxy
  holds one of the coordinator's upstream requests open, three things hold on
  the client database: a separate connection with `busy_timeout = 0` gets
  `BEGIN IMMEDIATE` at once, `PRAGMA wal_checkpoint(TRUNCATE)` reports
  `busy = 0` (no read or write transaction is open across the HTTP call), and
  a CLI `save` completes within 5 s.
- **AC-7:** `--help`, `--version`, `completions zsh`, `upgrade --check`,
  `doctor`, `auth status`, `sync --action status`, `sync daemon status`, and a
  running `serve` start no coordinator and write no unit file. `mcp
  --read-only` and a coordinator ticking over an empty data directory never
  create `comemory.db`.
- **AC-8:** No command accepts `--no-daemon`. `auth login --daemon` parses,
  warns that it is deprecated, and changes nothing. With `push_on_save =
  false`, a saved memory still reaches the hub through the coordinator. After
  `sync daemon stop` and after `sync daemon uninstall`, the next ordinary
  command brings a verified coordinator back. `restart` yields a new
  instance; `repair` yields a verified one.
- **AC-9:** `watch --once --json` attached to the coordinator prints
  `connected` and then `pulled` with the count of one complete catch-up, and
  the pulled memory exists locally. `watch` without `--once` prints a
  `pulled` event for a later coordinator pass. A manual `comemory sync`
  concurrent with coordinator passes leaves every operation accepted once and
  the outbox empty. `daemon run` in the foreground is a working coordinator.
- **AC-10:** Scenario: 2,100 pending local operations with the hub stopped.
  `status` still answers in under 1 s and the last pass reports a network
  end. After the hub restarts, the coordinator drains every operation (hub
  feed count and an empty outbox) with no further command. After SIGSTOP and
  SIGCONT of the coordinator, it answers `status` again and later passes
  continue. Logged in with `verify_every = 2s`, the coordinator writes
  `sync-verify.last` and reports `last_verify_at`. A restarted coordinator
  inside the interval does not verify again until the interval elapses.
- **AC-11:** With the platform's native backend forced
  (`COMEMORY_DAEMON_SUPERVISOR=launchd` on macOS, `systemd` on Linux) and its
  unit directory made read-only under the test `HOME`, `ensure` still
  reports `ready: true` with `supervisor: process` and a note naming the
  write failure. No `launchctl`/`systemctl` call happens, because the write
  fails first. On Linux with auto-detection and no user bus
  (`DBUS_SESSION_BUS_ADDRESS` and `XDG_RUNTIME_DIR` unset), `ensure` chooses
  `process` and says so. The `status` states `running`, `not_running`,
  `stale` and `disabled` are each produced for real.

## Acceptance evidence

All tests run real `comemory` processes, real SQLite, and real `comemory
serve` hubs behind the real fault proxy (`tests/common/exchange_support.rs`).
They use `COMEMORY_DAEMON_SUPERVISOR=process`, an isolated `HOME`, and
`COMEMORY_SYNC_DAEMON` removed. Each fixture stops its coordinator on drop and
deletes the data directory, which also triggers the watchdog. Suite:
`cargo nextest run --all-features --test replica_daemon --test replica_daemon_2
--test replica_daemon_3 --test replica_daemon_4 --test replica_daemon_5`, and harness case
`bash scripts/test-replication-e2e.sh --case daemon`. Coverage rows
`R-1`…`R-11` map to case `daemon` in `scripts/replication/coverage.json`.
`R-5` also lists the live `propagation` and `lost-nudge` cases.
Hook-firing tests run with a sandboxed `HOME` and
`PATH=<cargo bin dir>:/usr/bin:/bin`, so the hook finds the binary under test
and never the host's Homebrew `comemory`. Credentials are real engine session
tokens written through `AuthFile::save`, the call `auth login` makes. That
call also clears the barrier.

| AC | Real input | Expected result | Boundary / failure | Check |
|---|---|---|---|---|
| AC-1 | fresh data dir; auth.json with known secret; `COMEMORY_API_KEY` set | readiness fields exact; secret/token absent from `status --json`, `daemon.json`, logs | second `daemon run` → 75; two dirs → two pids | `replica_daemon::readiness_*`, `second_run_*` |
| AC-2 | fresh dir, 8 parallel `ensure` | one instance id; one process with `--data-dir <dir>` in `ps` | coordinator from a copied binary replaced by `ensure`, kept by preflight | `replica_daemon::concurrent_ensure_*`, `ensure_replaces_*` |
| AC-3 | SIGKILL coordinator; 16 parallel `list --json` | all succeed; one new instance; `ps` count 1 | `external` → exit 69 message; hub stopped → `search` still 0 | `replica_daemon_2::preflight_*` |
| AC-4 | hub + client; logout mid-pass (proxy stall) | readiness transitions; proxy log unchanged after logout with env key | org switch: hub B feed lacks A's memory | `replica_daemon_2::auth_lifecycle_*`, `logout_barrier_*` |
| AC-5 | real git repo with installed hooks and a real commit; 50 wakes behind a held `sync.lock`; hub write | new HEAD indexed; exactly 2 passes; events alternate; tick pull | stalled upstream → status < 1 s | `replica_daemon_3::triggers_*` |
| AC-6 | SIGTERM; stale file; symlinked socket; foreign listener; `daemon.json` naming a `sleep` pid; unlinked socket; 120-byte dir; config edit | exit 0 + cleanup; rejections then repair; `sleep` still alive; re-bind; long-path socket; new interval | during a held request: `BEGIN IMMEDIATE` with no wait, checkpoint busy = 0, `save` < 5 s | `replica_daemon_3::lifecycle_*`, `no_transaction_spans_http` |
| AC-7 | each exempt command; `serve` spawned; `mcp --read-only` on empty dir | no `daemon.lock` holder, no unit files under the test `HOME`, no `comemory.db` | coordinator over empty dir for 3 ticks | `replica_daemon_4::exempt_*`, `no_corpus_*` |
| AC-8 | `auth login --no-daemon`; `--daemon`; `push_on_save=false` config | usage error; deprecation warning; hub receives memory | stop/uninstall then `list` → running | `replica_daemon_4::compat_*` |
| AC-9 | hub with memory; `watch --once --json`; `watch` follow; manual `sync` | `connected` + `pulled N`; follow event; hub feed has each op once, outbox empty | coordinator restart during `watch` re-attaches | `replica_daemon_4::watch_*`, `manual_sync_*` |
| AC-10 | 2,100 pending ops seeded through the store API the #255 backlog suite uses; hub stop/restart; SIGSTOP/SIGCONT; `verify_every = 2s` | outbox drains with no command; status < 1 s; `sync-verify.last` written; no re-verify inside the interval | hub down → network end, nothing lost | `replica_daemon_5::backlog_*`, `verify_*` |
| AC-11 | read-only `Library/LaunchAgents` or `.config/systemd/user` under the test `HOME`; Linux without a user bus | `supervisor: process` with a note; each status state produced | no supervisor CLI invoked | `replica_daemon_5::supervision_*`, `status_states_*` |

Existing suites stay green: `cargo nextest run --all-features`,
`bash scripts/check-all.sh`, `bash scripts/dup-check.sh`, and `just e2e`,
which gains a real-binary daemon smoke under the `process` supervisor and an
isolated `HOME`.

## Documentation impact

- `README.md`: the required daemon, lifecycle commands, and status states.
- `AGENTS.md`: Key Commands, Module Map (`domains/sync/daemon/`,
  `cli/daemon_preflight.rs`), Environment Variables (`COMEMORY_SYNC_DAEMON`
  as a harness switch, `COMEMORY_DAEMON_SUPERVISOR`).
- `docs/guides/cloud-sync.md`: the daemon lifecycle, login/logout barrier,
  triggers, and headless/container supervision.
- `docs/configuration.md`: `daemon_interval`, `verify_every`, `push_on_save`
  semantics.
- `docs/cli-reference.md` (regenerated); `docs/scenarios/sync.md`, `auth.md`,
  `watch.md`; `docs/container-image.md` (`daemon run` sidecar or `process`
  supervision); `docs/guides/replication-e2e.md` (case `daemon`).
- `src/domains/sync/README.md` and the new `src/domains/sync/daemon/README.md`;
  `docs/designs/2026-09-17-domain-first-migration-inventory.md` rows for every
  new file.
- `docs/designs/2026-09-14-sync-daemon.md`: a "Superseded by" header.

## Open Questions

1. **Platform harness pin bump** (owner: issue 258 / comemory.io 185,
   non-blocking). When `apps/api/replication-engine.sha` moves past this
   change, the live harness must set `COMEMORY_DAEMON_SUPERVISOR=process`
   (or `COMEMORY_SYNC_DAEMON=0` for cases that assert explicit syncs).
   Otherwise its temp-`HOME` clients start coordinators on the developer's
   launchd.
2. **Stable executable path for units** (owner: issue 258, non-blocking).
   Units record `current_exe()`. Preflight treats a coordinator whose binary
   file vanished as stale, so a Homebrew Cellar path self-heals. Issue 258
   picks the stable path.
