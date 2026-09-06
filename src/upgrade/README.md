# upgrade/

**What belongs here:** the core of `comemory upgrade` — resolving the newest
release, classifying how the running binary was installed, comparing
versions, and delegating the actual swap to the release's own `install.sh`
(or to `brew`). `src/upgrade.rs` beside this folder owns `Request` /
`Report` / `Status` and the `run` orchestration.

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
| `installer.rs` | `run_script` | Fetch `<releases>/download/<tag>/install.sh` and run it pinned (`--version --dir --no-modify-path [--quiet]`); `brew_upgrade`; `installed_version` reads `exe --version` back after the swap |
| `release.rs` | `latest_tag` | The `<releases>/latest` redirect → tag, and `download` of one asset, via `curl`/`wget`; `COMEMORY_RELEASES_URL` (test hook) overrides the GitHub base |
| `version.rs` | `Version` | `MAJOR.MINOR.PATCH[-pre]` parse, `Display`, `tag()`, and ordering (a pre-release sorts below its final) |

Tests live beside the module under `tests/` (`version.rs`, `channel.rs`);
the end-to-end path — a loopback stand-in for GitHub Releases, the real
`install.sh`, a private copy of the real binary replaced in place — is
`tests/cli__upgrade.rs`, with the script alone under `tests/install_script.rs`.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/upgrade.rs` (`pub mod
<name>;`) and callers import concrete paths.
