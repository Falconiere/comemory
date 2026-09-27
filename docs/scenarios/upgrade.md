# `comemory upgrade`

Move the running binary to the newest GitHub release, or to a pinned one.
Resolves `latest` by following the release redirect, works out how this
binary was installed (standalone / Homebrew / `cargo install`), compares
versions, and hands the swap to the release's own `install.sh` (or to
`brew`). No HTTP client is compiled in — fetches shell out to `curl` or
`wget`. Every outcome except `--check` then runs the installed binary's own
`sync daemon ensure` and requires a ready coordinator on that binary (exit
69 otherwise). `--json` runs the installer quietly and prints the report
object (`current`, `latest`, `target`, `channel`, `exe`, `status`, `hint`,
`daemon`).

**Runnable tests:** `tests/cli__upgrade.rs` and `tests/upgrade_daemon.rs` (the command), `tests/install_script.rs`, `tests/install_script_2.rs`, `tests/install_daemon.rs` and `tests/install_daemon_2.rs` (the script alone), `scripts/test-daemon-install.sh` (real old/new builds)

**HTTP:** none — a server must never replace its own binary on request (`transport: "cli-only"` in `GET /api/v1/commands`; asserted by `tests/api__parity.rs`)

Global flags `--json` and `--data-dir` are accepted; `--data-dir` names the
data directory whose sync daemon is ensured (the store itself is never
opened). See [globals.md](globals.md).

Every scenario points `COMEMORY_RELEASES_URL` at a loopback stand-in for
GitHub Releases (`tests/common/release_server.rs`) and, where a swap
happens, runs a private copy of the binary — never `target/debug/comemory`.

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--check` | off | Report the running and latest versions; install nothing |
| `--version` | latest | Install this release (`0.19.0` or `v0.19.0`) instead of the latest |
| `--force` | off | Proceed when the target is not newer (reinstall or downgrade) |

## Scenarios

### upgrade-01 Check, newer release published

- **Flags:** `--check`
- **Setup:** fixture `latest` = `v9.9.9`
- **Command:** `comemory --json upgrade --check`
- **Expect:** `status: "available"`, `current` = the build's version,
  `latest`/`target` = `9.9.9`, `channel.kind: "standalone"`, a `hint`;
  the binary is untouched.
- **Covered by:** `tests/cli__upgrade.rs::check_reports_an_available_release_without_installing`

### upgrade-02 Check, already current

- **Flags:** `--check`
- **Setup:** fixture `latest` = the running version
- **Command:** `comemory upgrade --check` (TTY and `--json`)
- **Expect:** `status: "up_to_date"`, no `hint`; TTY says `is up to date`.
- **Covered by:** `tests/cli__upgrade.rs::check_reports_up_to_date_when_latest_is_the_running_build`

### upgrade-03 Reinstall in place

- **Flags:** `--force` `--version`
- **Setup:** fixture `latest` = the running version, staged as a real
  tarball of this build + `.sha256` + `install.sh`; the binary installed by
  `install.sh` into a private `<tmp>/bin/`
- **Command:** `comemory --json upgrade --force --version v<current>`
- **Expect:** `status: "installed"`; `daemon.binary` is the file at `exe`;
  shell completions are refreshed through `install.sh`; no staging,
  rollback or lock leftovers. (A cross-version upgrade between two real
  builds is proven by `scripts/test-daemon-install.sh`.)
- **Covered by:** `tests/cli__upgrade.rs::upgrade_replaces_the_running_binary_in_place`

### upgrade-04 No-op when current

- **Flags:** _(none)_
- **Setup:** fixture `latest` = the running version
- **Command:** `comemory upgrade`
- **Expect:** exit 0, `comemory <v> is up to date`, nothing downloaded,
  and a `sync daemon ready` line for the installed binary.
- **Covered by:** `tests/cli__upgrade.rs::upgrade_is_a_no_op_when_already_on_latest`

### upgrade-05 Pinned older release needs --force

- **Flags:** `--version`
- **Command:** `comemory upgrade --version 0.0.1`
- **Expect:** exit 64, stderr `0.0.1 is older than the running … pass
  --force to downgrade`; nothing installed.
- **Covered by:** `tests/cli__upgrade.rs::pinning_an_older_release_needs_force`

### upgrade-06 Forced release that cannot become ready

- **Flags:** `--version` `--force`
- **Setup:** staged `v0.0.1` whose binary answers only `--version`
- **Command:** `comemory --json upgrade --version v0.0.1 --force`
- **Expect:** exit 69, no stdout JSON; stderr carries `install.sh`'s
  `rolled back to comemory …`; the binary still reports the running version.
- **Covered by:** `tests/cli__upgrade.rs::a_forced_release_that_cannot_become_ready_is_rolled_back_with_69`

### upgrade-07 Installer failure is relayed

- **Flags:** _(none)_
- **Setup:** staged release whose `.sha256` sidecar is tampered
- **Command:** `comemory --json upgrade`
- **Expect:** exit 70; stderr carries `install.sh exited with …` and the
  script's own `checksum mismatch` line; the binary is untouched.
- **Covered by:** `tests/cli__upgrade.rs::installer_failure_is_relayed_and_leaves_the_binary_alone`

### upgrade-08 Release host unreachable

- **Flags:** `--check`
- **Setup:** `COMEMORY_RELEASES_URL=http://127.0.0.1:1`
- **Command:** `comemory upgrade --check`
- **Expect:** exit 69, stderr `could not fetch http://127.0.0.1:1/latest`.
- **Covered by:** `tests/cli__upgrade.rs::unreachable_release_host_exits_unavailable`

### upgrade-09 Malformed --version

- **Flags:** `--version`
- **Command:** `comemory upgrade --version next`
- **Expect:** exit 64, stderr ``invalid version `next` ``.
- **Covered by:** `tests/cli__upgrade.rs::a_malformed_pinned_version_is_a_usage_error`

### upgrade-10 Forced reinstall restarts the coordinator (#258)

- **Flags:** `--version` `--force`
- **Setup:** a real binary installed by the real `install.sh`, its coordinator
  already verified ready
- **Command:** `comemory --json upgrade --force --version <current>`
- **Expect:** `status: "installed"`; `daemon.ready: true`, `daemon.pid`
  different from the coordinator running before the upgrade (which is gone),
  `daemon.binary_file` matching the new file's `(dev, ino)`.
- **Covered by:** `tests/upgrade_daemon.rs::a_forced_reinstall_restarts_the_coordinator_on_the_new_file`

### upgrade-11 Already up to date still repairs an absent daemon

- **Flags:** _(none)_
- **Setup:** installed binary, its coordinator stopped (`sync daemon stop`)
  before the upgrade
- **Command:** `comemory --json upgrade`
- **Expect:** `status: "up_to_date"`; `daemon.ready: true` and a new pid — the
  no-op swap still runs the installed binary's own `sync daemon ensure`.
- **Covered by:** `tests/upgrade_daemon.rs::an_up_to_date_upgrade_repairs_an_absent_coordinator`

### upgrade-12 `--check` starts and writes nothing

- **Flags:** `--check`
- **Command:** `comemory --json upgrade --check`
- **Expect:** the report has no `daemon` field at all; no `daemon.token`,
  `daemon.json` or socket is written; no coordinator process exists
  afterward.
- **Covered by:** `tests/upgrade_daemon.rs::check_starts_nothing_and_writes_nothing`

### upgrade-13 `--data-dir` binds which coordinator is ensured

- **Flags:** `--data-dir` (global) `--version` `--force`
- **Command:** `comemory --data-dir <other> upgrade --force --version <current>`
- **Expect:** `daemon.data_dir` equals the canonicalized `<other>`; a
  coordinator answers ready there; the default data directory is never
  created.
- **Covered by:** `tests/upgrade_daemon.rs::the_data_dir_flag_binds_the_coordinator_it_ensures`

### upgrade-14 A release whose daemon never comes up is rolled back with 69

- **Flags:** `--version` `--force`
- **Setup:** a release archive that installs but whose binary cannot bring up
  a coordinator (the stub used in cargo tests; the real `v0.49.1` archive in
  the shell harness)
- **Command:** `comemory --json upgrade --force --version <that release>`
- **Expect:** exit 69; stdout empty (no success JSON); stderr contains
  `rolled back`; the binary on disk is byte-identical (sha256) to before the
  attempt.
- **Covered by:** `tests/upgrade_daemon.rs::a_release_that_cannot_become_ready_fails_with_69_and_keeps_the_binary`, `tests/cli__upgrade.rs::a_forced_release_that_cannot_become_ready_is_rolled_back_with_69`

### upgrade-15 `cargo install` build points at the finalizing wrapper

- **Flags:** `--version`
- **Setup:** a `cargo install` layout: the binary under `$CARGO_HOME/bin`,
  `.crates.toml` listing comemory, `CARGO_HOME` exported
- **Command:** `comemory upgrade --version v9.9.9`
- **Expect:** non-zero exit; stderr names `bash scripts/dev-install.sh` — the
  wrapper that builds, installs, and ensures the daemon in one step (a bare
  `cargo install` recipe must be followed by
  `comemory sync daemon ensure`).
- **Covered by:** `tests/upgrade_daemon.rs::a_cargo_install_build_is_pointed_at_the_finalizing_wrapper`

### upgrade-16 Pending operations cross the upgrade intact

- **Flags:** `--version` `--force`
- **Setup:** five pending outbox operations saved against a stopped
  `exchange_support::Hub`, then the hub started after the upgrade
- **Command:** `comemory --json upgrade --force --version <current>`
- **Expect:** the outbox operation ids are unchanged by the swap; once the
  hub is reachable again the same five ids drain and reach the hub's feed
  exactly once each; the coordinator answering afterward is the new pid.
- **Covered by:** `tests/upgrade_daemon.rs::pending_operations_cross_the_upgrade_and_reach_the_hub_once_it_returns`

## The installer script

`install.sh` (repo root, uploaded to every release by
`release-finalize.yml`) is what `upgrade` runs. Its own contract —
`--version`, `--dir`, `--no-modify-path`, `--no-completions`, `--quiet`, the
env twins, checksum verification, in-place replacement of the `comemory`
already on `PATH`, the new binary's own `sync daemon ensure` right after the
swap (restoring the previous binary and exiting 69 when it never becomes
ready, before any completion or PATH step), then completion installation
and the once-only rc-file PATH line — is driven by `tests/install_script.rs`
and `tests/install_daemon.rs`.
