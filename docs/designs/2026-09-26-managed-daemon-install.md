# Start and verify the required daemon after every managed install or update — Design

**Date:** 2026-09-27   **Status:** Approved   **Author:** Auto
**Topic:** Every managed install/update path ends with a verified, current
sync coordinator for the data directory, or fails honestly (issue 258,
epic 248). Builds on `2026-09-25-required-sync-daemon.md` (issue 257).

## Problem

Issue 257 made the sync coordinator required and gave it an idempotent
`comemory sync daemon ensure`. Nothing calls it at install time:

- `install.sh` proves only that the new file runs `--version`.
- `scripts/dev-install.sh` does the same after `cargo install`.
- `comemory upgrade` swaps the binary, reads `--version` back and reports
  `upgraded`.

After any of these, the old coordinator keeps running the replaced file, or
no coordinator runs until the first ordinary command's preflight starts one.
The user requires the daemon up, on the new binary, before the installer
exits and before authentication or any ordinary command.

Three gaps in 257's identity handling make this worse:

1. **Same-version replacement goes unnoticed.** A same-version reinstall or a
   `cargo install --force` rebuild keeps `(version, canonical path)`.
   `ensure::accept` (`src/domains/sync/daemon/ensure.rs:94-95`) therefore
   accepts the old process that still runs the unlinked inode.
2. **`ensure`'s readiness wait checks no identity.** `wait_or_fail`
   (`ensure.rs:231-243`) accepts any healthy probe. When the graceful stop of
   an old coordinator outlasts the overall deadline (`STOP_GRACE` is 15 s,
   `coordinator.rs:36`), `ensure` can report the *old* coordinator as ready.
3. **Preflight accepts any healthy coordinator.** Its fast path, `quick_probe`
   (`src/cli/daemon_preflight.rs:155-160`), accepts any healthy coordinator
   before `accept` runs. An installer killed between the rename and `ensure`
   leaves the old coordinator running at the same path indefinitely.

Other gaps:

- `comemory upgrade` ignores `--data-dir` (`src/cli/upgrade.rs:50`), so it
  cannot preserve the data-directory binding.
- The launchd/systemd units omit `COMEMORY_DAEMON_SUPERVISOR`
  (`daemon_templates.rs:50-54,79`), so native coordinators report
  `supervisor: foreground`.

## Non-Goals

1. **No Homebrew formula hook.** The following belong to
   [homebrew-tap#1](https://github.com/Falconiere/homebrew-tap/issues/1), which
   is blocked by this issue:
   - the `post_install` lifecycle;
   - the generated-formula guard in `.github/workflows/release.yml`'s tap step;
   - the stable `opt/` path inside the unit;
   - real `brew install/upgrade/uninstall` tests.

   This issue only makes `comemory upgrade` run `ensure` after `brew upgrade`
   (D8, whose real-brew evidence is tap#1's H-1–H-3). The docs must not yet
   claim that Homebrew guarantees immediate readiness.
2. **Daemon changes are limited to these, all intentional exceptions to
   257:**
   - one additive readiness field, `binary_file`;
   - the identity rule extension (D3), which amends 257 D13's
     "preflight replaces only a missing binary";
   - an identity check in `ensure`'s readiness wait (D3);
   - the unit templates gain `COMEMORY_DAEMON_SUPERVISOR` (D12);
   - the `process` spawn drops `COMEMORY_API_KEY` (D7).

   The protocol stays `1`. Coordinator loop, bounds and exit codes are
   unchanged.
3. **No hook for bare `cargo install` or copied binaries.** These are
   unmanaged binary placement (I-7). Docs and in-product hints name them
   unmanaged and point to the finalizing command. Ordinary startup preflight
   repairs the daemon.
4. **No store migration in the install transaction.** The installer runs only
   the new file's `--version`, `completions --install` (preflight-exempt) and
   `sync daemon ensure` (exempt, and the coordinator never migrates, 257 D9).
5. **No system-level (root) service install**, and no build-script side
   effects (I-8).
6. **No Windows.** Unsupported targets already fail `ensure` (257).

## Architecture

```text
install.sh ─ download ─ sha256 ─ extract ─ new --version ─┐   (any failure: old file untouched, exit 1)
                                                           ▼
     lock $DIR/.comemory-install.lock (D6) ─ keep previous as $DIR/.comemory.prev.$$ (hard link; cp -p fallback)
     atomic rename staged → $DIR/comemory
     "$DIR/comemory" sync daemon ensure --json      (the new file; ensure evicts a mismatched coordinator,
                                                     returns ready only for its own identity, D3)
        ├─ exit 0, "ready":true, version == new --version, binary == physical $DIR/comemory
        │      ─▶ drop .prev ─ completions ─ PATH ─ summary        (exit 0)
        └─ otherwise ─▶ previous existed? rename .prev back (atomic); "$DIR/comemory" sync daemon ensure --json
                         (restored file, best effort); exit 69 "rolled back to <old>; sync daemon: ready|not ready"
                        first install ─▶ keep the new file; exit 69 "<path> installed; sync daemon not ready …"

scripts/dev-install.sh ─ cargo install --force ─ "$BIN" sync daemon ensure --json ─ same checks, else exit 1

comemory [--data-dir D] upgrade ─ resolve ─┬─ --check ─▶ report only (no probe, no ensure, no writes)
                                           ├─ up to date / tap lag ─▶ ensure_child(exe, D)
                                           └─ newer or --force ─▶ swap (install.sh with COMEMORY_DATA_DIR=D | brew)
                                                                 ─▶ verify --version ─▶ ensure_child(exe, D)
   ensure_child = spawn `<exe> --data-dir D sync daemon ensure --json`; require exit 0, ready,
                  daemon.version == on-disk version, daemon.binary == canonical(exe)
```

**Chosen approach:** every managed channel runs the **newly installed
file's own** `sync daemon ensure --json` as a child process after the file
is in its final place. It accepts success only for a coordinator whose
identity is that file's. `ensure` already does the lifecycle work:

- evicts a coordinator with a different identity;
- drains it gracefully (the `shutdown` op cancels the pass at the next batch
  boundary);
- starts the new one under the same per-data-directory unit.

The decisive trade-off is child process against in-process. The
`comemory upgrade` process is the *old* build: an in-process `ensure` would
compare against the wrong compiled-in version and spawn whatever file now
sits at its path. The child is the new build by construction.

On the Standalone channel `upgrade` ensures twice: once inside the new
`install.sh`, once in `ensure_child`. The second call is the idempotent
verification, and its `action` is usually `none`. Evidence therefore asserts
pid change and identity, never `action` (D4).

**Reused:**

- `sync daemon ensure`: eviction, `daemon-ensure.lock`, supervisor fallback,
  bounds.
- `upgrade::installer::run_tool`.
- `install.sh`'s staged rename.
- `tests/common/release_server.rs`.
- `tests/common/daemon_support.rs` (`DaemonHome`: private `HOME`, forced
  `process` supervisor, cleanup).
- `tests/common/exchange_support.rs::Hub`.

No new crate.

### Decisions (recorded because nobody answers questions during the epic)

| # | Decision | Reason |
|---|---|---|
| D1 | On any post-swap readiness failure, `install.sh` **restores the previous file** by atomic rename and runs the restored file's `sync daemon ensure --json` (best effort). A binary predating 257 has no `ensure`, which is reported, not fatal. It exits **69**. A first-time install keeps the new file and exits 69 with `<DIR>/comemory sync daemon ensure` as the fix (the absolute path, since PATH setup did not run). Rollback cannot introduce a schema incompatibility: the transaction never opens the store (Non-Goal 4). If the restored coordinator's `store` reads `too_new`, that state predates this install; the message then adds the forward-recovery line (`keep the newer comemory; run comemory doctor`). | I-3/I-4. (Jev choice `restore` p=0.75.) |
| D2 | `install.sh` parses `ensure`'s compact JSON (`src/cli/output/json.rs:15`) with whitespace-tolerant `sed`. The patterns are `"ready":[[:space:]]*true`, and `"version":"…"`/`"binary":"…"` inside the `daemon` object. Success requires: exit 0, `ready` true, `daemon.version` equal to the new file's `--version` number, and `daemon.binary` equal to `$(cd -P "$DIR" && pwd -P)/comemory`. A path containing `"` or `\` fails loudly as a mismatch and is never trusted. Identity beyond version and path is enforced by `ensure` itself (D3), so the shell needs no `stat` dialects. | I-1: report the daemon's responding version/path. No `jq` dependency. |
| D3 | Readiness gains `binary_file: "<dev>:<ino>"`, captured when the coordinator starts from `metadata(current_exe())`. `BinaryIdentity` gains the same `file` for the caller. Rules: (a) `ensure::accept` for `Ensure`/`Restart`/`Repair` requires version, path **and** file to match the caller. (b) `wait_or_fail` accepts only a coordinator passing (a) for those intents; an old coordinator still answering at the deadline is `not ready: previous coordinator did not stop`. (c) Preflight, in both `quick_probe` and `accept`, replaces a coordinator when its `binary` is the caller's own canonical path, its `binary_file` differs, and the caller's `file` equals `stat(path)` now; that is, the caller *is* the on-disk file. A missing `binary_file` (pre-258 coordinator, e.g. v0.50.0) counts as "differs" for (a). For (c), it is replaced when its `binary` is the caller's own path, its `version` differs from the caller's, and the caller is the on-disk file. Otherwise it is kept. | Same-version reinstall and rebuild restart on the new file (I-2). A kill after the rename is repaired by the next command (I-5). A still-running old process never evicts a newer coordinator. Amends 257 D13. (Jev choice `file_identity` p=0.99.) |
| D4 | `comemory upgrade` runs `ensure_child` in every non-`--check` outcome: `up_to_date`, the Homebrew tap-lag hint, `upgraded`, `installed`. On failure it exits 69 (`Error::Unavailable`), stating `binary replaced` or `binary unchanged`. It prints no success JSON. When `install.sh` itself exits 69 (D1), `run_script` maps it to `Error::Unavailable` with the script's rollback tail; other script failures keep `Error::Other` (70). | I-2 "repairs an absent daemon even when already up to date", and I-3. |
| D5 | The `Report` JSON gains `daemon: {ready, action, supervisor, version, binary, binary_file, pid, data_dir}`. `supervisor` is `ensure`'s top-level value. With `--check` the field is absent: no probe, no write. | I-1 reporting. I-2 keeps `--check` non-mutating. |
| D6 | `install.sh` serializes on `$DIR/.comemory-install.lock`, a **symlink whose target is the owner pid**. `ln -s "$$" lock` is atomic and carries the pid, so there is no window between creation and pid write. **Liveness:** `ps -p <pid>` (works for other users' pids; any process with that pid counts as alive). **Reclaim of a dead pid P:** first claim the one-shot right with `ln -s "$$" lock.reclaim.P`. Only one racer per P can create it; losers wait. The winner re-reads `lock`; if it still names P, it replaces it atomically (`ln -s "$$" lock.new.$$ && mv -f lock.new.$$ lock`). Nobody else can take a lock that names a dead P, because its owner never releases it and every reclaim of P is gated by the marker. **Release:** `rm -f lock lock.reclaim.*`, only while `lock` still names `$$`. **Wait:** poll every 0.5 s for up to 60 s, then exit 1 with `another install into <dir> is running (pid N)`. **Interrupted reclaim:** `lock` and `lock.reclaim.P` both naming dead pids means a reclaimer died mid-reclaim. Auto-sweeping it would reopen the theft race, so the installer exits 1 immediately: `a previous install into <dir> was interrupted while taking its lock; remove <dir>/.comemory-install.lock* and retry`. At start the installer also removes `.comemory.new.<pid>`/`.comemory.prev.<pid>` leftovers whose pid is dead; the sweep never touches lock or reclaim files. | "Race two installers": one swap and one `ensure` at a time. A rollback cannot clobber a racer's newer file. (Round-2 review: a mkdir+pid-file lock could steal a live lock and wedge on a pid-less dir.) |
| D7 | `spawn` (the `process` supervisor) removes `COMEMORY_API_KEY` from the child environment. Units carry only `COMEMORY_DATA_DIR` and `COMEMORY_DAEMON_SUPERVISOR` (D12). | I-9: an installer shell's exported key never becomes the resident daemon's credential. Protected `auth.json` stays the only path. Only `COMEMORY_API_KEY` is a credential variable (`src/config/env.rs:311`). |
| D8 | On the Homebrew channel, `upgrade` runs `ensure_child` with `$(brew --prefix comemory)/bin/comemory`. The expected `binary` is the **canonicalized** result of that path (the new Cellar file), not the old `exe`. Real brew evidence is tap#1's. | The I-6 split. Without canonicalizing, the check could never pass (reviewer finding). |
| D9 | No flag or env var skips `ensure` in either installer. `--quiet` hides only progress; `--no-completions` and `--no-modify-path` are unrelated. **External supervisor:** `ensure` evicts a mismatched operator-run coordinator gracefully, then waits for the operator's restart policy (docker `restart:`, systemd `Restart=always`) to relaunch `sync daemon run` from the stable path, which is now the new file. Without a restart within the bound, the result is not ready, so the install rolls back (D1). The message names `restart your comemory sync daemon run`. Documented in `docs/configuration.md`, with the limitation that a container whose main process is `sync daemon run` restarts as a whole. Inside such a container, install from the image build, not via `docker exec`. | I-8, and an explicit failure with a supported foreground path. (Jev choice `evict_and_wait` p=0.92.) |
| D10 | Native lifecycle evidence runs on disposable GitHub Actions runners. A new `daemon-install` job in `test.yml` runs on every ready PR and is added to the required `test` facade's `needs`, with test-suite's release-plz skip handling. Its matrix: `macos-14` (launchd), `ubuntu-22.04` native (systemd `--user`), `ubuntu-22.04` headless. **macOS:** the first step runs `launchctl print gui/$(id -u)` and **fails** the job if absent. **Ubuntu native:** it runs `sudo loginctl enable-linger "$USER"`, exports `XDG_RUNTIME_DIR=/run/user/$(id -u)` and `DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus`, waits for `systemctl --user is-system-running` to answer, and fails if it does not. Native mode uses the runner's **real HOME**, so the user manager reads `~/.config/systemd/user` and the LaunchAgents directory. The script asserts readiness `supervisor` equals the native kind, so a silent `process` fallback fails the run. **Headless:** unsets both bus variables and expects `process`. Local iteration uses Colima (Linux, headless). The script refuses native mode unless `CI=true` or `COMEMORY_DISPOSABLE_ENV=1`. | Orchestrator decision 2026-09-27 (Jev ci 1.0). The issue forbids the developer's session. The repo is public, so runner minutes are free. (Jev noul 0.53 on every-PR; chosen because daemon behavior can regress from any `src/` change.) |
| D11 | The script's binaries: **old** is the real published `v0.50.0` artifact (carries 257's `ensure`); **broken-new** is the real `v0.49.1` artifact (no `ensure`). Both are fetched with their published `.sha256`. **New** is two real branch builds, `X.Y.(Z+1)-lifecycle.1` and `-lifecycle.2`. They are built from a scratch `git worktree` whose `Cargo.toml` version is rewritten and whose `Cargo.lock` is refreshed by `cargo update -p comemory --offline`. They are built with `--profile release-quick`, the profile `dev-install.sh` uses, and share the job's `CARGO_TARGET_DIR`, so only the `comemory` crate recompiles. v0.50.0 → lifecycle.1 exercises the old client's `upgrade` running the new `install.sh`. lifecycle.1 → lifecycle.2 exercises the new `upgrade` code. | "Exercise old/new actual binaries". Cargo tests cannot build two versions offline. |
| D12 | `render_launch_agent_plist`/`render_systemd_unit` add `COMEMORY_DAEMON_SUPERVISOR=launchd|systemd`. | Readiness then reports the real kind, which 257's design already promised. |
| D13 | I-7 in-product text: `cargo_hint` (`upgrade.rs:196-200`) and `install.sh`'s unsupported-platform hints (`install.sh:98,102`) recommend `scripts/dev-install.sh` (the finalizing wrapper). A bare `cargo install` recipe is followed by `&& comemory sync daemon ensure`. | Supported source-install instructions must use the wrapper. |
| D14 | The release-finalize smoke additionally asserts the `Sync daemon  ready` line and a matching `ensure --json` from the published binary on `ubuntu-22.04` (headless, so `process`), then runs `sync daemon uninstall`. | I-6: generated release artifacts keep the hook, checked on the real published asset. |

## Interfaces / Schema

### `install.sh`

Flags are unchanged. `comemory upgrade` still passes
`--version --dir --no-modify-path --quiet`, and now also exports
`COMEMORY_DATA_DIR`.

The installer must stay within the 300-line guardrail. The budget is about
70 added code lines, and if it overflows, cut in this order:

1. one-line error hints;
2. merge `say`/`step` variants;
3. fold the leftover sweep into the lock helper.

Usage text gains no flag. `.prev` is a hard link (same directory, same
filesystem), so AC-4(a)'s same-pid assertion holds. The `cp -p` fallback,
used only where hard links fail, gives a new inode: the restored `ensure`
then replaces the coordinator and still reports ready. A new progress line follows
"Installed":

```text
  ✓ Sync daemon  ready (0.50.1, pid 4242, launchd)  → /Users/u/.comemory
```

The exit codes are:

- **0:** only after D2's checks.
- **1:** download, checksum, extract, run or lock failures, all before the
  swap.
- **69:** post-swap readiness failure.

```text
error: sync daemon not ready after installing comemory v0.50.1: <ensure error>
    rolled back to comemory 0.50.0 at /u/.local/bin/comemory (sync daemon: ready)
error: sync daemon not ready after installing comemory v0.50.1: <ensure error>
    comemory v0.50.1 is installed at <path>; fix the cause, then run: <path> sync daemon ensure
error: another install into <dir> is running (pid N); retry when it finishes
```

The summary's "installed" headline prints only on the success path.

### `scripts/dev-install.sh`

After `cargo install`, it runs `"$BIN_PATH" sync daemon ensure --json` with
the same D2 checks, then logs `log_ok install "sync daemon ready (vX, pid N,
<supervisor>)"`. On failure it runs `die install "cargo replaced $BIN_PATH, but
the sync daemon is not ready: <cause> — run: $BIN_PATH sync daemon repair"`
(exit 1). There is no skip flag.

### `comemory upgrade --json`

```json
{ "current": "0.50.0", "latest": "0.50.1", "target": "0.50.1",
  "channel": {"kind": "standalone", "dir": "/u/.local/bin"},
  "exe": "/u/.local/bin/comemory", "status": "upgraded",
  "daemon": { "ready": true, "action": "none", "supervisor": "launchd",
              "version": "0.50.1", "binary": "/u/.local/bin/comemory",
              "binary_file": "16777232:9123456", "pid": 4242,
              "data_dir": "/u/.comemory" } }
```

Failure exits 69 with nothing on stdout. Stderr is
`error: <exe> is now <v> (binary replaced|binary unchanged), but the sync daemon is not ready: <cause> — run: comemory sync daemon repair`,
or the D1 rollback tail for a Standalone swap.

### Readiness (`protocol: 1`, additive)

The field is `"binary_file": "16777232:9123456"`. Its source is
`std::os::unix::fs::MetadataExt::{dev, ino}` of `current_exe()` at coordinator
start. A rename-replace gives the path a new inode on APFS and ext4; the
running process keeps the old one.

### `upgrade` library

`upgrade::installer::ensure_child(exe: &Path, data_dir: &Path, expect_version: &Version) -> Result<DaemonReport>`
has one call site. `Request` gains `data_dir: PathBuf`, filled from the
resolved `--data-dir`/`COMEMORY_DATA_DIR`.

### Harness / manifest

- **`scripts/test-daemon-install.sh --native|--headless`** fetches or builds
  the binaries (D11). It stages a loopback release server
  (`scripts/lib/daemon_install/release_server.py`, the same routes as
  `release_server.rs`) and runs the scenarios in AC-8. It prints one
  `PASS|FAIL <scenario>` line each, exits 1 on any failure, and never prints
  `SKIP`. Scenario bodies live in `scripts/lib/daemon_install/*.sh`, each
  under the 300-line cap. Under uid 0 the readiness fault is the `external`
  supervisor, never `chmod`.
- **`scripts/test-replication-e2e.sh`** gains case `install`:
  `cargo nextest run --test install_script --test install_daemon --test cli__upgrade --test upgrade_daemon`,
  then `bash scripts/test-daemon-install.sh --headless`.
- **`scripts/replication/coverage.json`** maps `I-1` … `I-9` to `["install"]`.
- **Test fixtures:** `tests/common/daemon_hub_support.rs` receives
  `write_auth`/`save_pending`/`seed_local_pending`, moved out of
  `tests/replica_daemon_5.rs`, which then imports them. The move adds no
  duplicate pair. `release_server.rs` gains `stage_real_release(root, tag)`,
  which packages the real `cargo_bin("comemory")` with `XZ_OPT=-0 -T0` (about
  4 s for the 100 MB debug binary). It is cached per binary
  `(len, mtime)` under `$CARGO_TARGET_TMPDIR` behind a `FileLock`.

## Failure modes and edge cases

| Situation | Observable behavior | Propagation |
|---|---|---|
| Download 404, checksum mismatch, corrupt archive, new file fails `--version` | Dies before the lock and swap. Old file byte-identical; old coordinator keeps the same pid and instance | exit 1 |
| Installer killed before the rename | Old file intact. The next install removes the dead pid's `.comemory.new.<pid>` | recovered |
| Installer killed after the rename, before `ensure` | New file in place; the old coordinator runs the unlinked inode. The next ordinary command's preflight replaces it (D3c, `quick_probe`). The next install removes `.prev.<pid>` | recovered |
| Installer killed during `ensure` | The kernel releases `daemon-ensure.lock`. The next command re-probes and repairs | recovered |
| New release lacks `ensure` or cannot start (`v0.49.1` archive, `--version`-only stub) | D1: restored; the restored coordinator was never evicted, so it answers with the same pid. The rollback line says `ready` | exit 69 |
| Readiness blocked for every binary (data dir `chmod 0500` as non-root, `external` with nobody running `daemon run`) | D1 restore, restored ensure also not ready: the line says `not ready` and names the cause. A first install keeps the file | exit 69 |
| `ensure` ready but on the wrong version or path (a racer, a foreign coordinator) | Not ready (D2) | exit 69 |
| Old coordinator slower to stop than the bound | `ensure` not ready (D3b), never "ready" for the old one | exit 69 |
| Same-version reinstall, `--force`, `cargo install --force` rebuild | `binary_file` differs, so it is replaced. New pid; old pid gone | — |
| Relocation (install into another `--dir`) | The path differs, so it is replaced. The unit is rewritten in place under the same `<id>`; there is never a second unit | — |
| Two installers racing into one dir | D6 serializes. The second waits (at most 60 s) and then swaps. One unit and one coordinator per data dir | — |
| Two installers into different dirs, one data dir | `daemon-ensure.lock` serializes. The last `ensure` wins. One coordinator | — |
| `comemory --data-dir X upgrade` | install.sh and `ensure_child` target X, and the default dir is never created. Other data dirs are repaired lazily by their next preflight (D3c) | — |
| Pending outbox and cursors during upgrade | `shutdown` cancels at a batch boundary. Rows stay in `comemory.db`, untouched by the transaction. The new coordinator drains them | — |
| Store migration pending after the new binary lands | The coordinator idles as `migration_pending` (257 D9). The first write command migrates | — |
| Restored older binary sees `too_new` store | A pre-existing condition (Non-Goal 4). The rollback message adds forward-recovery guidance; no automatic further action | exit 69 |
| Headless Linux (no user bus) or uid 0 | `process` supervisor with a `notes` entry: ready, or a clear failure | — |
| `COMEMORY_API_KEY` exported in the installer shell | Absent from units (257) and from the `process` child's environment (D7). `auth.state` follows `auth.json` only | — |
| `upgrade --check` | No `ensure`, probe, file or unit write. The JSON has no `daemon` | — |
| `upgrade` on a `cargo install` build | Newer target: `Unsupported` with D13's wrapper hint. Up to date: `ensure_child` runs | — |
| Package uninstall (`comemory sync daemon uninstall`) | Removes this data dir's unit. The coordinator exits gracefully, removing `daemon.sock`/`daemon.json`. Kept: `daemon.token`, logs, `comemory.db`, memories, and other data dirs' units and coordinators | — |
| Login/logout | Owns no installation: `auth login --daemon` is a hidden no-op since 257 (`replica_daemon_4.rs:214` proves it) | — |

## Acceptance criteria

- **AC-1:** (I-1) Setup: a logged-out isolated `HOME`, the `process`
  supervisor, and a staged release of the real branch binary. Running
  `sh install.sh --dir <tmp>/bin --no-modify-path` (and again with
  `--quiet --no-completions`) exits 0. A verified coordinator then answers
  with `version == CARGO_PKG_VERSION`, `binary == canonical(<tmp>/bin/comemory)`,
  `binary_file == stat(<tmp>/bin/comemory)` and `auth.state == logged_out`.
  Non-quiet stdout carries the `Sync daemon  ready` line with that pid.
  - In CI, `scripts/dev-install.sh` does the same with a real
    `cargo install`.
  - Under `COMEMORY_DAEMON_SUPERVISOR=external`, dev-install.sh exits
    non-zero with `the sync daemon is not ready`.
- **AC-2:** (I-2) Setup: a coordinator started by an installed real binary,
  with pid P recorded.
  - `<bin>/comemory upgrade --force --version v<cur> --json` reports
    `status: installed` and a `daemon` whose `pid ≠ P`, with P gone, and
    `binary_file == stat(<bin>/comemory)`.
  - With no coordinator running, `comemory upgrade --json` (latest ==
    current) reports `up_to_date` with `daemon.ready: true`.
  - `comemory upgrade --check` leaves no `daemon.token`, `daemon.json`,
    socket or coordinator process.
  - `comemory --data-dir <X> upgrade --force …` leaves the coordinator bound
    to canonical `<X>`, and the default data dir absent.
- **AC-3:** (I-2 preservation) Setup: N pending operations saved against a
  stopped `exchange_support::Hub`. After AC-2's forced upgrade:
  - the outbox op ids and count are unchanged;
  - `data_dir`, the cursor rows and the unit `<id>` are unchanged.

  When the hub starts, the hub's feed holds exactly those N ops (ids equal
  to the preserved outbox ids), and the client's outbox is empty. The
  coordinator answering is the new pid.
- **AC-4:** (I-3/I-4 rollback)
  - (a) Installing a release whose binary cannot become ready (the stub
    archive in cargo tests; the real `v0.49.1` archive in the script) over a
    working install exits 69. The file is byte-identical (sha256) to the
    previous one. The *same* coordinator (pid, instance) still answers. Stderr
    says `rolled back … (sync daemon: ready)`. Stdout has no "installed"
    headline.
  - (b) Under a fault that blocks every binary (`chmod 0500` of the data dir
    as non-root; `external` otherwise), the rollback says
    `sync daemon: not ready` and names the cause.
  - (c) A first-time install under (b) keeps the new file, exits 69, and
    names `<path> sync daemon ensure`.
  - (d) `comemory upgrade` over (a) exits 69 with no stdout JSON, and the
    binary is unchanged.
- **AC-5:** (I-4 pre-swap) A truncated archive, a wrong `.sha256` or a
  missing tag each exits 1. The previous file stays byte-identical and the
  previous coordinator answers with the same pid and instance.
- **AC-6:** (I-5)
  - (a) Two concurrent `install.sh` runs into one dir both exit 0, or one
    reports the lock. Afterwards `coordinator_pids_for(data_dir)` has exactly
    one pid; under native mode exactly one unit file exists for the data dir.
  - (b) Re-installing into a second `--dir` leaves one coordinator with the
    new `binary`, and the old pid gone. Under native mode, the same unit
    `<id>` holds the new path.
  - (c) Simulated kill after the rename: `mv` of a freshly built copy over
    `<bin>/comemory`, with no `ensure`. The next `comemory stats`, run from
    `<bin>/comemory`, replaces the coordinator with one whose `binary_file`
    matches the file. The test polls the probe for up to 30 s, because a
    preflight's `wait_or_fail` may still see the old coordinator stopping.
    The script adds the same case from a real v0.50.0 coordinator (no
    `binary_file`). The D3c rule is a pure function over readiness and
    caller identity. A colocated unit test runs it on the metadata of two
    real files. It proves that a caller whose own `file` differs from
    `stat(path)`, i.e. an older process still running after the swap, never
    evicts.
- **AC-7:** (I-9)
  - A `process`-supervised coordinator started by `install.sh` with
    `COMEMORY_API_KEY` exported (a split literal, safe for gitleaks) has no
    such variable in its environment (`ps eww` / `/proc/<pid>/environ`). It
    reports `auth.state: logged_out`, and no file under the data dir or unit
    directory contains the value.
  - `sync daemon uninstall` removes the unit, the socket at the path
    readiness reported in `socket` (it may be the fallback directory), and
    `daemon.json`; `comemory.db` still returns a saved memory; a second data
    dir's coordinator still answers.
  - Native units contain only the two documented env keys.
- **AC-8:** (native, CI) `bash scripts/test-daemon-install.sh` passes in
  three modes: `--native` on `macos-14` (launchd), `--native` on
  `ubuntu-22.04` (systemd `--user`), and `--headless` on `ubuntu-22.04`. The
  scenarios, all with real binaries:
  - fresh logged-out install;
  - v0.50.0 → lifecycle.1 upgrade by the old client, with memories and outbox
    tables (`sqlite3 .dump`) row-identical;
  - lifecycle.1 → lifecycle.2 by the new `upgrade`;
  - same-version reinstall;
  - relocation;
  - racing installers;
  - kill-after-rename recovery;
  - v0.49.1 broken-new rollback;
  - uninstall;
  - `dev-install.sh` success and external-supervisor failure.

  Each asserts pid, version, binary path, `binary_file`, unit count, and the
  supervisor kind equal to the mode.
- **AC-9:** (I-6/I-7 artifacts) The release-finalize smoke asserts the daemon
  line and `ensure` identity for the published asset. `upgrade`'s
  `cargo install` hint (asserted at runtime in `cli__upgrade.rs` on a real
  `cargo install` layout: the binary copied into `$CARGO_HOME/bin`, a
  `.crates.toml` listing comemory in `$CARGO_HOME`, and `CARGO_HOME`
  exported, the layout `channel::detect` reads) names `scripts/dev-install.sh`. install.sh's
  unsupported-platform hints are checked as **text**
  (`install_script.rs::unsupported_platform_hints_name_the_wrapper` reads
  `install.sh`), because no supported CI host reaches that branch without
  faking `uname`.

## Acceptance evidence

| AC | Real input | Expected | Boundary / failure | Check |
|---|---|---|---|---|
| AC-1 | `stage_real_release` of `cargo_bin("comemory")` in a real `.tar.xz` plus sha256, served by `ReleaseServer`; `DaemonHome`-style env (`HOME`, short `TMPDIR`, `COMEMORY_DAEMON_SUPERVISOR=process`) | `client::probe` readiness matches the installed file | `--quiet --no-completions` still ensures; dev-install external failure (CI) | `cargo nextest run --test install_daemon`; CI job `daemon-install` |
| AC-2 | Installed real binary with a running coordinator | pid change; `binary_file` match; `up_to_date` repairs; `--check` inert; `--data-dir` bound | `--check`, custom data dir | `cargo nextest run --test upgrade_daemon` |
| AC-3 | Real `save::run` pending ops with the `exchange_support::Hub` stopped, then started | Outbox ids equal before/after; the hub receives exactly N | Hub down across the upgrade | `cargo nextest run --test upgrade_daemon` |
| AC-4 | Stub archive (a real script with no `ensure`); real `chmod`; real `external`; real `v0.49.1` in the script | sha256 restore, same instance, exit 69, stderr text | ready vs not-ready restore; first install; upgrade path | `install_daemon`, `upgrade_daemon`, script scenario `rollback` |
| AC-5 | Truncated archive, wrong sidecar, missing tag on the loopback server | Old hash and old instance unchanged | Each of the three | `cargo nextest run --test install_daemon` |
| AC-6 | Two concurrent real `sh install.sh`; a second `--dir`; `mv` without `ensure` | One pid (`coordinator_pids_for`); one unit (native); replacement on `stats` | Race, relocation, older-process non-eviction | `install_daemon`; script `race`, `relocate`, `kill-after-rename` |
| AC-7 | Exported key-shaped value; two data dirs | Absent from env and files; uninstall scope | Second data dir survives | `install_daemon`; script `uninstall` |
| AC-8 | Real v0.50.0 and v0.49.1 artifacts from GitHub; two real branch builds | Every scenario `PASS` in three modes | Headless without a bus; launchd `gui/<uid>` probe fails the job when absent | `test.yml` job `daemon-install` (a facade `needs`) |
| AC-9 | Published release asset; real `cargo install` layout; install.sh text | Smoke passes; hints name the wrapper (install.sh hint is a labelled text check) | — | release-finalize smoke; `cargo nextest run --test cli__upgrade --test install_script` |

Every cargo test above is added red first on current code, which produces no
daemon at install time, no `daemon` field, and no same-version replacement.

**Fate of existing tests.** Every converted test gets the `DaemonHome`
environment, so no test touches the developer's launchd.

- **`install_script.rs`** (12 tests):
  - Switch to `stage_real_release` and real version assertions, then stop the
    coordinator at drop: `installs_latest_into_dir_and_reports_each_step`,
    `env_pins_version_and_dir`, `rerun_replaces_the_comemory_already_on_path`,
    `without_dir_it_falls_through_to_an_existing_cargo_bin`,
    `without_dir_or_cargo_bin_it_uses_local_bin`,
    `path_line_is_appended_to_the_shell_rc_once`,
    `no_modify_path_skips_the_path_line_but_keeps_completion_setup`,
    `installs_shell_completions_by_default`,
    `no_completions_skips_completion_installation`.
  - Unchanged (they fail before the swap): `checksum_mismatch_aborts_and_installs_nothing`,
    `missing_release_names_the_url`, `help_and_unknown_option_exit_codes`.
- **`cli__upgrade.rs`**:
  - Unchanged (no install happens): `check_reports_an_available_release_without_installing`,
    `check_reports_up_to_date_when_latest_is_the_running_build`,
    `pinning_an_older_release_needs_force`,
    `installer_failure_is_relayed_and_leaves_the_binary_alone`,
    `unreachable_release_host_exits_unavailable`,
    `a_malformed_pinned_version_is_a_usage_error`.
  - Becomes the real-binary `--force --version v<cur>` swap:
    `upgrade_replaces_the_running_binary_in_place`.
  - Real binary; asserts `daemon`: `upgrade_is_a_no_op_when_already_on_latest`.
  - Becomes AC-4(d), the stub rollback with exit 69:
    `force_installs_a_pinned_older_release`.

No stub answers `ensure`.

## Documentation impact

- `README.md` (install section), `docs/getting-started.md`,
  `docs/guides/upgrading.md`, `docs/scenarios/install.md`,
  `docs/scenarios/upgrade.md`.
- `docs/cli-reference.md`, regenerated for the upgrade JSON `daemon`.
- `docs/configuration.md`: the daemon section covers unmanaged placement,
  the external-supervisor install behavior (D9), the uninstall step, and
  that `COMEMORY_API_KEY` is not inherited.
- `src/domains/maintenance/upgrade/README.md`.
- `AGENTS.md` Distribution section (lines 718-727) and the install/upgrade
  contract.
- `install.sh` header comment and hints (D13).
- The release-finalize smoke comment.
- `CHANGELOG.md` via release-plz.

## Open Questions

None blocking. Homebrew lifecycle and real-brew evidence belong to
homebrew-tap#1 (owner: that issue's worker; blocked by this issue). It is
non-blocking here because D8 keeps `comemory upgrade` correct on that channel,
and the docs claim no more.
