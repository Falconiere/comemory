# domains/maintenance/upgrade/

**What belongs here:** the core of `comemory upgrade` — resolving the newest
release, classifying how the running binary was installed, comparing
versions, and delegating the actual swap to the release's own `install.sh`
(or to `brew`). `src/domains/maintenance/upgrade.rs` beside this folder owns
`Request` / `Report` / `Status` and the `run` orchestration.

**What does NOT belong here:** an HTTP client. Fetches shell out to `curl`
(falling back to `wget`), the same tools the installer needs, so the crate
gains no TLS stack. Also not here: an `/api/v1` route — `upgrade` is
CLI-only (`serve::routes::meta::CLI_ONLY`); a server replacing its own
binary mid-request is not a feature.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `channel.rs` | `Channel` | Homebrew / `cargo install` / standalone detection from the resolved `current_exe` path (`Cellar` component or Homebrew prefix; `$CARGO_HOME/.crates.toml` listing) |
| `installer.rs` | `run_script`, `ensure_child`, `verify_ensured`, `DaemonReport`, `brew_binary` | Fetch `<releases>/download/<tag>/install.sh` and run it pinned (`--version --dir --no-modify-path [--quiet]`, `COMEMORY_DATA_DIR` exported); `brew_upgrade`; `installed_version` reads `exe --version` back after the swap; `ensure_child` runs `<exe> --data-dir <dir> sync daemon ensure --json` as a **child process** — the newly installed file, never this (possibly just-replaced) one — and `verify_ensured` requires that JSON to report a ready coordinator whose version and canonical binary path both match what was expected, returning `DaemonReport` (`ready, action, supervisor, version, binary, binary_file, pid, data_dir`); `brew_binary` canonicalizes `$(brew --prefix comemory)/bin/comemory` to the linked Cellar file, since Homebrew's channel checks the new binary, not the running (old) process's path |
| `release.rs` | `latest_tag` | The `<releases>/latest` redirect → tag, and `download` of one asset, via [`crate::utilities::fetch`](../../../utilities/fetch.rs) (`curl`/`wget`); `COMEMORY_RELEASES_URL` (test hook) overrides the GitHub base |
| `version.rs` | `Version` | `MAJOR.MINOR.PATCH[-pre]` parse, `Display`, `tag()`, and ordering (a pre-release sorts below its final) |

Tests live beside the module under `tests/` (`version.rs`, `channel.rs`,
`installer.rs`); the end-to-end path — a loopback stand-in for GitHub
Releases, the real `install.sh`, a private copy of the real binary replaced
in place — is `tests/cli__upgrade.rs`, with the script alone under
`tests/install_script.rs` / `tests/install_script_2.rs`, and the daemon
lifecycle (identity-checked eviction, rollback, pending-op survival) under
`tests/install_daemon.rs`, `tests/install_daemon_2.rs`, and
`tests/upgrade_daemon.rs` (#258).

### Exit 69, the daemon-not-ready code

`upgrade.rs::run` calls `ensure_daemon` after every non-`--check` outcome —
including a same-version no-op, since the point is repairing an absent
coordinator, not only reacting to a swap. `ensure_daemon` maps any
`ensure_child` failure to `Error::Unavailable`, which the CLI dispatcher
turns into exit **69** (`sysexits.h EX_UNAVAILABLE`), the same code
`install.sh` uses for its own post-swap readiness failure and rollback. The
message names whether the binary on disk actually changed
(`binary replaced` / `binary unchanged`) and always ends with the same
recovery hint, `run: comemory sync daemon repair`. `Report.daemon` is `Some`
in every case that reaches `ensure_daemon` and stays `None` only under
`--check`, which returns before any of this runs.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from
`src/domains/maintenance/upgrade.rs` (`pub mod <name>;`) and callers import
concrete paths.
