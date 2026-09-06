# `comemory upgrade`

Move the running binary to the newest GitHub release, or to a pinned one.
Resolves `latest` by following the release redirect, works out how this
binary was installed (standalone / Homebrew / `cargo install`), compares
versions, and hands the swap to the release's own `install.sh` (or to
`brew`). No HTTP client is compiled in — fetches shell out to `curl` or
`wget`. `--json` runs the installer quietly and prints the report object
(`current`, `latest`, `target`, `channel`, `exe`, `status`, `hint`).

**Runnable tests:** `tests/cli__upgrade.rs` (the command), `tests/install_script.rs` (the script alone)

**HTTP:** none — a server must never replace its own binary on request (`transport: "cli-only"` in `GET /api/v1/commands`; asserted by `tests/api__parity.rs`)

Global flags `--json` and `--data-dir` are accepted; `--data-dir` is unused
(nothing here touches the store). See [globals.md](globals.md).

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

### upgrade-03 Upgrade in place

- **Flags:** _(none)_
- **Setup:** fixture `latest` = `v9.9.9` with a staged tarball + `.sha256`
  + `install.sh`; a private copy of the binary in `<tmp>/bin/`
- **Command:** `comemory --json upgrade`
- **Expect:** `status: "upgraded"`, `target: "9.9.9"`; the file at `exe`
  now answers `--version` with `comemory 9.9.9`; no staging leftovers.
- **Covered by:** `tests/cli__upgrade.rs::upgrade_replaces_the_running_binary_in_place`

### upgrade-04 No-op when current

- **Flags:** _(none)_
- **Setup:** fixture `latest` = the running version
- **Command:** `comemory upgrade`
- **Expect:** exit 0, `comemory <v> is up to date`, nothing downloaded.
- **Covered by:** `tests/cli__upgrade.rs::upgrade_is_a_no_op_when_already_on_latest`

### upgrade-05 Pinned older release needs --force

- **Flags:** `--version`
- **Command:** `comemory upgrade --version 0.0.1`
- **Expect:** exit 64, stderr `0.0.1 is older than the running … pass
  --force to downgrade`; nothing installed.
- **Covered by:** `tests/cli__upgrade.rs::pinning_an_older_release_needs_force`

### upgrade-06 Forced downgrade / reinstall

- **Flags:** `--version` `--force`
- **Setup:** fixture `latest` = `v9.9.9`, staged `v0.0.1`
- **Command:** `comemory --json upgrade --version v0.0.1 --force`
- **Expect:** `status: "installed"`, `latest: "9.9.9"`, `target: "0.0.1"`;
  the binary now reports `0.0.1`.
- **Covered by:** `tests/cli__upgrade.rs::force_installs_a_pinned_older_release`

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

## The installer script

`install.sh` (repo root, uploaded to every release by
`release-finalize.yml`) is what `upgrade` runs. Its own contract —
`--version`, `--dir`, `--no-modify-path`, `--quiet`, the env twins,
checksum verification, in-place replacement of the `comemory` already on
`PATH`, the once-only rc-file PATH line — is driven by
`tests/install_script.rs`.
