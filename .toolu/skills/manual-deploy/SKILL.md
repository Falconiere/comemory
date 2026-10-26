---
name: manual-deploy
description: Use when releasing, deploying or shipping a comemory version from this machine while GitHub Actions are disabled — runs scripts/deploy.sh to publish a stable vX.Y.Z.
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

## Procedure

1. Prerequisites: Apple Silicon Mac, Docker running with free disk space, clean
   tracked files, `gh api user` succeeds. If `GH_TOKEN` is set but stale,
   prefix every command below with `env -u GH_TOKEN`.
2. `git fetch --tags origin && git switch main && git pull --ff-only`.
3. Pick the version from the conventional commits since the last tag:
   `git log --oneline "$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' --exclude '*-*')"..HEAD`.
   Any `!` or `BREAKING CHANGE` → minor while the major is 0 (release-plz's
   rule), else any `feat:` → minor, otherwise patch.
4. Write the change log from those commits in the CHANGELOG buckets
   (`### Added` for feat, `### Fixed` for fix, `### Changed` for
   refactor/perf, `### Removed`, `### Security`), one `- ` line per change with
   its PR link. Skip `docs:`, `chore:`, `ci:`, `test:`, `build:` commits.
   `## [Unreleased]` in CHANGELOG.md must be empty; fold any entry there into
   this text instead.
5. Optional rehearsal (bump, three builds, archives, checks; no gates, nothing
   committed to a branch or pushed):
   `bash scripts/deploy.sh --tag v0.58.0 --change-log "$notes" --dry-run`
6. Release: `bash scripts/deploy.sh --tag v0.58.0 --change-log "$notes"`.
   Pass multi-line notes with `$'...'` or a variable, never a literal `\n`.
   Run it in the background: it first runs every quality gate, and the
   emulated x86_64 Linux build takes tens of minutes on a cold cache.
7. Report the release URL (the last stdout line) and the version to the user.

## Pitfalls

- Exit `1` means nothing was pushed: read the `[deploy:<phase>] FAIL` line, fix,
  and re-run with the same tag.
- Exit `2` means the tag is public but a later step failed. The output ends
  with the rollback commands; ask the user before running any of them, and
  never delete a tag or release on your own.
- Do not bypass git hooks: lefthook pre-push runs `check-all.sh` on the push.
  The script pushes with `RUSTUP_TOOLCHAIN=1.95.0`, CI's clippy pin, because a
  newer default clippy rejects `main`. A push that exits 141 after that slow
  hook did not land: the run exits 1, so re-run with the same tag.
- Never re-use a tag, even a deleted one: pick the next patch.
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
  and `bash scripts/test-deploy.sh rehearsal smoke`.
