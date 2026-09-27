# Start and verify the required daemon after every managed install or update — Design

**Date:** 2026-09-27   **Status:** Draft   **Author:** Auto
**Topic:** Every managed install/update path ends with a verified, current
sync coordinator for the data directory, or fails honestly (issue 258,
epic 248). Builds on `2026-09-25-required-sync-daemon.md` (issue 257).

## Problem

Issue 257 made the sync coordinator required and gave it an idempotent
`comemory sync daemon ensure`. Nothing calls it at install time. `install.sh`
proves only that the new file runs `--version`. `scripts/dev-install.sh` does
the same after `cargo install`. `comemory upgrade` swaps the binary, reads
`--version` back and reports `upgraded`. After any of these, the old
coordinator keeps running the replaced file, or no coordinator runs until
the first ordinary command's preflight starts one. The user requires the
daemon up, on the new binary, before the installer exits, and before
authentication or any ordinary command.

Two gaps in 257's identity rule make this worse. A same-version reinstall
or a `cargo install --force` rebuild keeps `(version, canonical path)`, so
`ensure` accepts the old process that still runs the unlinked inode. And an
installer killed between the rename and `ensure` leaves the old coordinator
running at the same path; preflight accepts it because its binary path still
exists.

## Non-Goals

1. **No Homebrew formula hook.** The `post_install` lifecycle, the
   generated-formula guard in `.github/workflows/release.yml`'s tap step, the
   stable `opt/` path inside the unit, and real `brew install/upgrade/
   uninstall` tests belong to
   [homebrew-tap#1](https://github.com/Falconiere/homebrew-tap/issues/1),
   which is blocked by this issue. This issue only makes `comemory upgrade` run
   `ensure` after `brew upgrade` (D8). The docs must not yet claim that
   Homebrew guarantees immediate readiness.
2. **No new daemon semantics.** Coordinator, protocol, supervisor backends,
   preflight bounds and exit codes stay as issue 257 defined them. There is
   one additive readiness field (`binary_file`) and one identity rule
   extension (D3).
3. **No hook for bare `cargo install` or copied binaries.** These are
   unmanaged binary placement (I-7). Docs name them unmanaged, and ordinary
   startup preflight repairs the daemon.
4. **No store migration in the install transaction.** Nothing an installer
   runs opens the store for writing. The coordinator never migrates (257 D9).
5. **No system-level (root) service install**, and no build-script
   side effects (I-8).
6. **No Windows.** Unsupported targets already fail `ensure` (257).

## Architecture

```text
install.sh  ─ download ─ sha256 ─ extract ─ new --version ─┐  (any failure: old file untouched, exit 1)
                                                            ▼
          take $DIR/.comemory-install.lock (mkdir mutex, stale-pid reclaim)
          keep previous: ln/cp $DIR/comemory → $DIR/.comemory.prev.$$
          atomic rename staged → $DIR/comemory
          "$DIR/comemory" sync daemon ensure --json   (new file; ensure evicts the old coordinator)
             ├─ ready, version == new --version, binary == $DIR/comemory ─▶ drop .prev, completions, PATH, summary
             └─ not ready ─▶ previous existed? rename .prev back (atomic),
                               run restored binary's ensure (best effort), exit 1
                               "rolled back to <old>; daemon: ready|not ready"
                             no previous ─▶ keep the new file, exit 1
                               "installed at <path>, sync daemon not ready: <cause> — <fix>"

scripts/dev-install.sh ─ cargo install --force ─ "$BIN" sync daemon ensure --json ─ require ready+match, else exit 1

comemory upgrade ─ resolve ─┬─ --check ─▶ report only (no ensure, no writes)
                            ├─ up to date / tap lag ─▶ ensure(exe)
                            └─ newer or --force ─▶ swap (install.sh | brew) ─▶ verify --version ─▶ ensure(exe)
              ensure(exe) = run `<exe> sync daemon ensure --json` as a child, require
                            ready && daemon.version == on-disk version && daemon.binary == exe
```

**Chosen approach:** every managed channel runs the **newly installed
file's own** `sync daemon ensure --json` as a child process after the file is
in its final place. It accepts success only when the answering coordinator
proves the expected version and path. `ensure` already evicts a coordinator
with a different identity, drains it gracefully (the shutdown op cancels the
pass at its next batch boundary, bounded), and starts the new one under the
same per-data-directory unit. The decisive trade-off is child process against
in-process: `comemory upgrade`'s own process is the *old* build. An in-process
`ensure` would compare against the wrong compiled-in version and spawn
whatever file is now at its path. The child is the new build by construction.

**Reused:** `sync daemon ensure` (257: identity eviction, `daemon-ensure.lock`,
supervisor fallback, bounds), `upgrade::installer::run_tool`, `install.sh`'s
staged rename, `tests/common/release_server.rs`, `tests/common/daemon_support.rs`
(`DaemonHome`: private `HOME`, forced `process` supervisor, cleanup), and
`replica_daemon_5.rs`'s hub fixture and `seed_local_pending`. No new crate.

### Decisions (recorded because nobody answers questions during the epic)

| # | Decision | Reason |
|---|---|---|
| D1 | On readiness failure after the swap, `install.sh` **restores the previous file** by atomic rename. It then runs the restored file's `sync daemon ensure --json` (best effort; a pre-257 binary has no `ensure`, and that is reported, not fatal) and exits 1. A first-time install keeps the new file and exits 1 with the repair command. | I-3/I-4: the previous binary stays usable, and rollback is always pre-migration because the transaction opens no store (Non-Goal 4). (Jev choice `restore` p=0.75.) |
| D2 | `install.sh` verifies `ensure`'s JSON with POSIX tools (`sed`), not `jq`. Success requires `"ready": true`, `daemon.version` equal to the new file's `--version`, and `daemon.binary` equal to the physical path of `$DIR/comemory` (`cd -P` + `pwd -P`). | I-1 "report the daemon's responding version/path". The installer has no jq dependency today. |
| D3 | Readiness gains `binary_file: "<dev>:<ino>"`, the coordinator's own executable file identity captured at start. `ensure` also replaces a coordinator whose `binary_file` differs from `stat(<its binary path>)`. Preflight replaces a coordinator only when the caller *is* the on-disk file (the caller's own `binary_file` equals `stat(path)`) and the coordinator runs a different file at that same path. A missing field (pre-258 coordinator) counts as "differs" for `ensure` and "unknown, keep" for preflight. | Same-version reinstall and `cargo install --force` rebuilds must restart on the new file (I-2). An installer killed after the rename is repaired by the next command (I-5). An old process still running after the swap never evicts a newer coordinator. Protocol stays 1 (additive field). (Jev choice `file_identity` p=0.99.) |
| D4 | `comemory upgrade` runs `ensure` in every non-`--check` outcome: `up_to_date`, the Homebrew tap-lag hint, `upgraded` and `installed`. On failure it returns exit 69 (`Error::Unavailable`) with a message that says whether the binary was replaced. The success JSON is emitted only for a verified coordinator. | I-2 ("repairs an absent daemon even when already up to date") and I-3. |
| D5 | The `Report` JSON gains `daemon: {ready, action, supervisor, version, binary, pid, data_dir}`. With `--check` the field is absent: no probe, no write. | I-1 reporting, and I-2 keeps `--check` non-mutating. |
| D6 | `install.sh` serializes on `$DIR/.comemory-install.lock`, a `mkdir` mutex holding the owner pid. It reclaims the lock when that pid is dead and waits at most 60 s otherwise. At start it removes its own leftover `.comemory.new.*`/`.comemory.prev.*` files whose pid is dead. | "Race two installers": one swap and one `ensure` at a time. `ensure`'s own lock already guarantees one coordinator. A rollback cannot clobber a racer's newer file. |
| D7 | `spawn` (the `process` supervisor) removes `COMEMORY_API_KEY` from the child environment. launchd/systemd units already carry only `COMEMORY_DATA_DIR` and `COMEMORY_DAEMON_SUPERVISOR`. | I-9: an installer shell's exported key must not become the resident daemon's credential. Protected `auth.json` is the only credential path. |
| D8 | On the Homebrew channel, `upgrade` runs `ensure` via `$(brew --prefix comemory)/bin/comemory`, the stable `opt` link. Real brew verification is homebrew-tap#1's. | I-6 split. The unit's stable-path rewrite belongs to the formula work. |
| D9 | No flag or env var skips `ensure` in either installer. `--quiet` hides progress only, and `--no-completions`/`--no-modify-path` are unrelated. With `COMEMORY_DAEMON_SUPERVISOR=external`, `ensure` is verify-only, so an installer without an operator-run `sync daemon run` fails with that instruction. | I-8. |
| D10 | Native lifecycle evidence runs on disposable GitHub Actions runners. A new `daemon-install` job in `test.yml` runs on `macos-14` (launchd, `gui/<uid>`) and `ubuntu-22.04` (systemd `--user` via `loginctl enable-linger`, plus a headless pass with the bus variables unset). It runs `bash scripts/test-daemon-install.sh`. Offline cargo tests use the `process` supervisor through `DaemonHome`. Local iteration uses Colima (Linux, headless). The script refuses native mode unless `CI=true` or `COMEMORY_DISPOSABLE_ENV=1`. | Orchestrator decision 2026-09-27 (Jev ci 1.0). The issue forbids running against the developer's session. |
| D11 | The script's **old** binary is the real published `v0.50.0` artifact, fetched and checksum-verified from GitHub Releases, which carries 257's `ensure`. Its **new** binaries are two real branch builds with versions `X.Y.(Z+1)-lifecycle.1` and `-lifecycle.2`, set by a temporary `Cargo.toml` version in a scratch copy. v0.50.0 → lifecycle.1 exercises the old client's `upgrade` running the new `install.sh`. lifecycle.1 → lifecycle.2 exercises the new `upgrade` code. | "Exercise old/new actual binaries". Cargo tests cannot build two versions offline. (Jev noul 0.5: accepted with the cross-version cases kept in the required CI job, never skipped.) |
| D12 | The release-finalize smoke test additionally asserts `install.sh`'s daemon line and the `ensure` JSON of the published binary on `ubuntu-22.04` (headless ⇒ `process`), then runs `sync daemon uninstall`. | I-6: "generated release artifacts retain the hook" is checked on the real published asset every release. |

## Interfaces / Schema

### `install.sh` (flags unchanged; `comemory upgrade` still passes `--version --dir --no-modify-path --quiet`)

New progress line after "Installed":
`✓ Sync daemon  ready (v0.50.1, pid 4242, launchd) → /Users/u/.local/bin/comemory`.
Success exit 0 only after D2's checks. New failure outputs, exit 1:

```text
error: sync daemon not ready after installing comemory v0.50.1: <ensure error>
    rolled back to comemory 0.50.0 at /u/.local/bin/comemory (sync daemon: ready|not ready)
error: sync daemon not ready after installing comemory v0.50.1: <ensure error>
    comemory v0.50.1 is installed at <path>; fix the cause, then run: comemory sync daemon ensure
error: another install into <dir> is running (pid N); retry when it finishes
```

The summary's "installed" headline is printed only on the success path.

### `scripts/dev-install.sh`

After `cargo install`: `"$BIN_PATH" sync daemon ensure --json`, the same D2
checks, then `log_ok install "sync daemon ready (vX, pid N, <supervisor>)"`.
Otherwise `die install "cargo replaced $BIN_PATH, but the sync daemon is not
ready: <cause> — run: comemory sync daemon repair"`. There is no skip flag.

### `comemory upgrade --json`

```json
{ "current": "0.50.0", "latest": "0.50.1", "target": "0.50.1",
  "channel": {"kind": "standalone", "dir": "/u/.local/bin"},
  "exe": "/u/.local/bin/comemory", "status": "upgraded",
  "daemon": { "ready": true, "action": "replaced", "supervisor": "launchd",
              "version": "0.50.1", "binary": "/u/.local/bin/comemory",
              "pid": 4242, "data_dir": "/u/.comemory" } }
```

On failure the exit code is 69, with stderr
`error: <exe> is now <v> (binary replaced|binary unchanged), but the sync daemon is not ready: <cause> — run: comemory sync daemon repair`.
There is no stdout JSON. A Standalone upgrade whose `install.sh` rolled back
fails through the existing `install.sh exited with …: <tail>` path, and the
tail carries D1's rollback line.

### Readiness (`protocol: 1`, additive)

`"binary_file": "16777232:9123456"` is captured with `std::fs::metadata(current_exe)`
(`dev`, `ino`) when the coordinator starts. `identity::BinaryIdentity`
gains `file: Option<String>` computed the same way for the caller.

### Harness / manifest

- `scripts/test-daemon-install.sh [--native|--headless] [--new-bin P1 --next-bin P2]`
  builds or takes the two lifecycle binaries, stages a loopback release
  server (a `python3` handler with the same routes as `release_server.rs`),
  runs the scenario table in AC-8 and prints one `PASS|FAIL <scenario>` line
  each. Exit 1 on any failure. It never prints `SKIP`.
- `scripts/test-replication-e2e.sh` gains case `install`: the cargo suites
  `tests/install_daemon.rs` and `tests/upgrade_daemon.rs` plus
  `bash scripts/test-daemon-install.sh --headless`.
- `scripts/replication/coverage.json`: `I-1` … `I-9` → `["install"]`.

## Failure modes and edge cases

| Situation | Observable behavior | Propagation |
|---|---|---|
| Download 404, checksum mismatch, corrupt archive, new file fails `--version` | Dies before the lock and the swap. Old file byte-identical, old coordinator untouched (same pid) | exit 1 |
| Installer killed before the rename | Old file intact. The dead pid's `.comemory.new.<pid>` is removed by the next install | recovered |
| Installer killed after the rename, before `ensure` | New file in place, old coordinator running the unlinked inode. The next ordinary command's preflight replaces it (D3). `.prev.<pid>` is removed by the next install | recovered |
| Installer killed during `ensure` | `daemon-ensure.lock` is released by the kernel. The next command re-probes and repairs | recovered |
| `ensure` not ready (unwritable data dir, `external` supervisor with nobody running `daemon run`, supervisor start failure) | D1: previous restored and its ensure attempted, or a first install keeps the file. Exit 1 names the cause and the fix. No "installed" headline | exit 1 |
| `ensure` ready but on the wrong version or path (a racer, a foreign coordinator) | Treated as not ready (D2) | exit 1 |
| Same-version reinstall / `--force` / `cargo install --force` rebuild | New `binary_file` differs, so `ensure` replaces. New pid, old pid gone | — |
| Relocation (install into a different `--dir`) | Path differs, so `ensure` replaces. The unit is rewritten in place under the same `<id>`, never a second unit | — |
| Two installers racing into one dir | D6 serializes. The second waits, then swaps and ensures. One unit and one coordinator per data dir | — |
| Two installers into different dirs, one data dir | `daemon-ensure.lock` serializes. The last `ensure` wins its identity. One coordinator | — |
| Pending outbox and cursors during upgrade | The shutdown op cancels at a batch boundary. Rows stay in `comemory.db` untouched by the transaction. The new coordinator drains them later | — |
| Store needs migration after the new binary lands | The coordinator idles `migration_pending` (257 D9). The first write command migrates. A later downgrade meets `too_new` and uses existing forward-recovery guidance, never an automatic old-binary launch | converted |
| Headless Linux (no user bus) or uid 0 | `process` supervisor with a `notes` entry. Ready, or a clear failure | — |
| `COMEMORY_API_KEY` exported in the installer shell | Not in the unit file (257), and removed from the `process` child's environment (D7). Readiness `auth.state` follows `auth.json` only | — |
| `upgrade --check` | No `ensure`, no probe, no file or unit write. The JSON has no `daemon` | — |
| `upgrade` on a `cargo install` build with a newer target | Unchanged `Unsupported` error. When up to date, `ensure` still runs | — |
| Pre-257 binary restored by rollback (no `ensure` subcommand) | Its `ensure` exits non-zero. The rollback line says `sync daemon: not ready (restored binary predates the required daemon)` | exit 1 |
| Package uninstall | `comemory sync daemon uninstall` removes this data dir's unit only. Memories, `comemory.db` and other data dirs' units are untouched (257 behavior; documented as the uninstall step) | — |

## Acceptance criteria

- **AC-1 (I-1):** Given a logged-out, isolated `HOME` and a staged release of the real branch binary, `sh install.sh --version v<cur> --dir <tmp>/bin` exits 0. A verified coordinator then answers with `version == <cur>`, `binary == <tmp>/bin/comemory` and `auth.state == logged_out`, and the output contains the `Sync daemon  ready` line with that pid. The same holds for `scripts/dev-install.sh` (CI job, real `cargo install`).
- **AC-2 (I-2):** With a coordinator running from an installed binary at the same version, `comemory upgrade --force --version v<cur>` reports `status: installed` and `daemon.action: replaced`. The new pid differs, the old pid is gone, and `binary_file` equals the on-disk file. With no coordinator running, `comemory upgrade` at the latest version reports `up_to_date` with `daemon.ready: true`. `comemory upgrade --check` leaves no coordinator, no `daemon.token` and no unit.
- **AC-3 (I-2 preservation):** N pending operations saved while the hub is down stay in the outbox across AC-2's upgrade, with the same count and ids, and the same `data_dir`, cursors and unit `<id>`. When the hub returns, the new coordinator drains exactly those N to the hub.
- **AC-4 (I-3/I-4 rollback):** With a working install present, installing a release whose coordinator cannot become ready exits 1. Readiness is broken by really denying access: the data dir is made read-only (`chmod 0500`), or `COMEMORY_DAEMON_SUPERVISOR=external` is set with nobody running `daemon run`. The binary at `$DIR/comemory` is then byte-identical to the previous one, stderr names the rollback and the cause, and stdout has no "installed" headline. A first-time install under the same fault keeps the new file and exits 1 with `comemory sync daemon ensure` in the message. `comemory upgrade` under the fault exits 69 with no success JSON.
- **AC-5 (I-4 pre-swap):** A corrupt archive, a wrong `.sha256` or a 404 leaves the previous file byte-identical, and the previous coordinator answers with the same pid and instance.
- **AC-6 (I-5):** (a) Two `install.sh` runs racing into one dir both exit 0 or one reports the lock. Afterwards exactly one coordinator process serves the data dir, and exactly one unit file exists under native mode. (b) Relocating the install to a second `--dir` rewrites the same unit `<id>` to the new path, and the old coordinator is gone. (c) After a simulated kill between the rename and `ensure` (a new file renamed in, `ensure` not run), the next ordinary command (`comemory stats`) replaces the old coordinator with one whose `binary_file` matches the on-disk file.
- **AC-7 (I-9):** A process-supervised coordinator started by `install.sh` with `COMEMORY_API_KEY=cmk_…` exported has no such variable in its environment (`ps eww`/`/proc/<pid>/environ`). Its readiness reports `logged_out`, and no unit file contains the key. `sync daemon uninstall` leaves `comemory.db` and a second data dir's coordinator untouched.
- **AC-8 (native, CI):** `bash scripts/test-daemon-install.sh --native` on `macos-14` (launchd) and on `ubuntu-22.04` (systemd `--user`), and `--headless` on `ubuntu-22.04`, pass these scenarios with real binaries: fresh logged-out install; v0.50.0 → lifecycle.1 upgrade by the old client with a pending op preserved; lifecycle.1 → lifecycle.2 by the new `upgrade`; same-version reinstall; relocation; racing installers; kill-after-rename recovery; readiness-failure rollback. Each asserts pid, version, binary path, the unit count for the data dir and the supervisor kind.
- **AC-9 (I-6/I-7/I-8 docs and artifacts):** `install.sh` and `dev-install.sh` contain no path that exits 0 without D2's check (enforced by AC-1/AC-4 tests with `--quiet` and `--no-completions`). The release-finalize smoke asserts the daemon line on the published asset. Docs name bare `cargo install`/copied binaries as unmanaged and Homebrew as pending homebrew-tap#1.

## Acceptance evidence

| AC | Real input | Expected | Boundary / failure | Check |
|---|---|---|---|---|
| AC-1 | `release_server` staging the **real** `cargo_bin("comemory")` in a real `.tar.xz` + sha256; `DaemonHome` env | readiness via `client::probe` matches `<tmp>/bin/comemory` + current version | `--quiet --no-completions` still ensures | `cargo nextest run --test install_daemon`; CI job runs `scripts/dev-install.sh` then `test-daemon-install.sh` |
| AC-2 | Installed real binary + running coordinator | pid change, `binary_file` match, `up_to_date` repairs, `--check` inert | `--check` leaves no files | `cargo nextest run --test upgrade_daemon` |
| AC-3 | Real `save::run` pending ops against `replica_daemon_5`'s hub, hub stopped then started | outbox ids equal before/after; hub receives exactly N | hub down during the upgrade | `cargo nextest run --test upgrade_daemon` |
| AC-4 | Real read-only data dir / real `external` supervisor | byte-identical restore (sha256 of file), exit 1 / 69, stderr text | first install vs rollback; pre-257 restored binary in CI (v0.50.0 has `ensure`; v0.49.1 used for the "predates" line) | `install_daemon`, `upgrade_daemon`, script scenario `rollback` |
| AC-5 | Truncated archive, wrong sidecar, missing tag on the real loopback server | old file hash and old coordinator instance unchanged | each of the three faults | `cargo nextest run --test install_daemon` |
| AC-6 | Two concurrent real `sh install.sh`; a second `--dir`; manual `mv` of the new file without `ensure` | one coordinator (`coordinator_pids_for`), one unit (native), replacement on `comemory stats` | race and relocation | `install_daemon`; script scenarios `race`, `relocate`, `kill-after-rename` |
| AC-7 | Exported fake-shaped key (split literal, gitleaks-safe) | absent from the child env and unit files; `logged_out` | second data dir survives uninstall | `install_daemon`; script scenario `uninstall` |
| AC-8 | Real v0.50.0 artifact from GitHub; two real branch builds | every scenario `PASS` on three runner modes | headless without bus; launchd `gui/<uid>` | `test.yml` job `daemon-install` (macOS + Ubuntu) |
| AC-9 | Real published asset (release-finalize) and repo docs | smoke passes; docs sections present | — | release-finalize smoke; `bash scripts/cli-docs-check.sh`; `bash scripts/check-replication-coverage.sh` |

Every cargo test above is new and is added red first. Current `main`
produces no daemon at install time, reports no `daemon` field, and never
replaces a same-version coordinator. Existing `install_script.rs` and
`cli__upgrade.rs` cases that swap in the `--version`-only stub archive now
fail readiness, which is real behavior. They are converted to assert D1/D4's
failure outcome, or moved to the real-binary staging. No stub answers
`ensure`.

## Documentation impact

`README.md` (install section), `docs/getting-started.md`,
`docs/guides/upgrading.md`, `docs/scenarios/install.md`,
`docs/scenarios/upgrade.md`, `docs/cli-reference.md` (regenerated: the
upgrade JSON `daemon`), the daemon section in `docs/configuration.md`
(unmanaged placement, uninstall step, `COMEMORY_API_KEY` not inherited),
`src/domains/maintenance/upgrade/README.md`, `AGENTS.md` (the install/upgrade
contract row), `CHANGELOG.md` via release-plz, and the `install.sh` usage
text.

## Open Questions

None blocking. Homebrew lifecycle belongs to homebrew-tap#1 (owner: that
issue's worker). It is non-blocking here because D8 keeps `comemory upgrade`
correct on that channel, and the docs do not claim more.
