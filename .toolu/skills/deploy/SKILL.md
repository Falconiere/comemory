---
name: deploy
description: Use when releasing a new version of the comemory CLI from this Mac while GitHub Actions are disabled — runs scripts/deploy.sh to publish a stable vX.Y.Z. Not for deploying services.
metadata:
  toolu:
    origin: agent
    created: 2026-10-26T17:04:13Z
---
## When to Use

When asked to release, deploy, ship or cut a version of comemory while GitHub
Actions are disabled (`gh api repos/Falconiere/comemory/actions/permissions`
answers `"enabled": false`). A pushed tag builds nothing then, so the release is
built and published from this Mac by `scripts/deploy.sh`: a stable `vX.Y.Z`
from an up-to-date `main`, with the full local gate set, `chore(release):
vX.Y.Z` landed on `main`, the GitHub Release marked latest and the Homebrew tap
updated. Production releases only — there are no dev or RC builds.

A release is permanent (tags are never reused), so the user decides. Start
only when the user asked for a release; if this skill loaded on a hint, ask
before step 1. Every run stops at step 5 for the user's yes on tag and notes.

## Procedure

1. Prerequisites: the main checkout, not a linked worktree (`main` is checked
   out there and `deploy.sh` refuses any other branch; leave a worktree session
   first, or work in `$(git rev-parse --path-format=absolute --git-common-dir)/..`),
   Apple Silicon Mac, the pinned Rust (1.99.1) installed through rustup (`deploy.sh` puts
   its `bin` first on `PATH` itself, so a Homebrew `cargo` is harmless), Docker
   running with free disk space, clean tracked files, `gh api user` succeeds.
   If `GH_TOKEN` is set but stale, prefix every command below with
   `env -u GH_TOKEN`.
2. `git fetch --tags origin && git switch main && git pull --ff-only`.
3. Rehearse: `bash scripts/deploy.sh --dry-run`, in the background. release-plz
   (the pinned 0.3.169 with cargo-semver-checks 0.51.0) computes the version
   and writes the CHANGELOG section from the commits since the last tag, exactly
   as the release-PR bot did: on 0.x a `feat:` is a patch, and only an
   incompatible library API (or a breaking `!`) makes it minor. Then three
   builds, archives and checks; no gates, nothing committed to a branch or
   pushed. It prints `release-plz computed vX.Y.Z; its CHANGELOG section:`
   and the section. "nothing to release" means no commit since the last tag
   renders a changelog line (`docs:`/`chore:`/`ci:`/`test:`/`build:` only):
   tell the user, do not force one. `## [Unreleased]` in CHANGELOG.md must be
   empty.
4. Show the user that tag and that section verbatim. Never edit the notes
   here: a wrong line is fixed in the commit subject's PR, then re-run.
5. Wait for an explicit yes in this conversation. Typing `/deploy` asks for a
   release; it approves neither the version nor the notes. No yes, no release.
6. Release with the approved tag as an assertion:
   `bash scripts/deploy.sh --tag vX.Y.Z`. If main moved and release-plz now
   computes something else, the script refuses before anything happens; go
   back to step 3. Run it in the background: it bumps first (about 90 s, so
   "nothing to release" refuses early), then runs every quality gate, the
   macOS build and one Linux build of both targets.
7. Report the release URL (the last stdout line) and the version to the user.

## Pitfalls

- Exit `1` means nothing was pushed: read the `[deploy:<phase>] FAIL` line, fix,
  and re-run with the same tag.
- Exit `2` means the tag is public but a later step failed. Fix the cause the
  `[deploy:publish] FAIL` line names, then finish with `bash scripts/deploy.sh
  --resume vX.Y.Z` from the same checkout: it reuses `target/deploy/vX.Y.Z/`
  and skips every step already done. It stops when a release asset differs
  from the local file; report that to the user instead of replacing it. The
  rollback commands at the end are only for withdrawing a bad release: ask the
  user before running any of them, and never delete a tag or release on your
  own.
- Do not bypass git hooks: lefthook pre-push runs `check-all.sh` on the push.
  The script pushes with the pinned toolchain's `bin` first on `PATH`, so the
  hook lints on the same Rust as the gates even when a Homebrew `cargo` is
  first. A push that exits 141 after that slow hook did not land: the run
  exits 1, so re-run with the same tag.
- Never re-use a tag, even a deleted one: pick the next patch.
- Preflight refuses a missing or other-version release-plz or
  cargo-semver-checks and prints the exact `cargo install --locked … --version …`
  command; run it (ask first if the user has not asked you to install tools).
- The script refuses anything but `main` equal to `origin/main`; do not work
  around it with a branch named main.
- A Docker build failing inside `apt-get` with "invalid signature" usually
  means the Docker disk is full (`docker system df`); ask the user before
  pruning anything — the VM is shared with other projects.
- No GHCR image and no minisign signature are produced while Actions are off.

## Verification

- `gh release view <tag> --repo Falconiere/comemory` lists three
  `comemory-<target>.tar.xz` archives with `.sha256` files, `sha256.sum`,
  `install.sh`, `comemory-installer.sh` and `comemory.rb`.
- `curl -fsSLI -o /dev/null -w '%{url_effective}' https://github.com/Falconiere/comemory/releases/latest`
  ends with the tag, and `Falconiere/homebrew-tap` `Formula/comemory.rb` shows
  the new version.
- The script's own smoke step already installed the release through its
  `install.sh` into a temp dir; its `[deploy:smoke] OK` line is the evidence.
- After changing the script itself: `bash scripts/test-deploy.sh` (fast cases)
  and `bash scripts/test-deploy.sh rehearsal smoke plan resume`.
- After changing this skill or its `.claude/skills/deploy` link (what makes it
  `/deploy`): `bash scripts/test-deploy.sh skill`.
