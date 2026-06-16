# Release runbook (comemory)

> **TL;DR — cutting a stable release is one command:**
>
> ```bash
> just release 0.11.0
> ```
>
> That runs the preflight, bumps the version, drafts the changelog,
> commits, dry-runs `dist plan`, tags, and pushes. The sections below
> are the long-form fallback + the things `just release` doesn't (and
> shouldn't) automate: secret setup, post-tag verification, rollback.

Single-tag-triggered pipeline via cargo-dist. Push `vX.Y.Z` → GitHub
Actions runs `release.yml` → builds the single `aarch64-apple-darwin`
target → uploads the tarball + shell installer to GitHub Releases →
pushes the formula to `Falconiere/homebrew-tap` (on stable tags only).
A second hand-maintained workflow, `release-finalize.yml`, runs after
the release is published to smoke-test the artifact, curate the release
body from `CHANGELOG.md`, and (optionally) sign the `SHA256SUMS`.

---

## 1. One-time setup

- [ ] The `Falconiere/homebrew-tap` repo exists.
- [ ] The `HOMEBREW_TAP_TOKEN` secret is set on `Falconiere/comemory`:
  ```bash
  gh secret list --repo Falconiere/comemory | grep HOMEBREW_TAP_TOKEN
  ```
  If missing, create a fine-scoped PAT (`contents: write` on
  `Falconiere/homebrew-tap` only) and add it as a repo secret.
- [ ] `cargo install cargo-edit --locked` (one-time host tool used by
  `just release` for `cargo set-version`).
- [ ] *(Optional, for signed releases)* Set up the minisign keypair per
  [`keys/README.md`](../keys/README.md): generate the keypair, store
  the private half in 1Password, set `MINISIGN_KEY` +
  `MINISIGN_PASSPHRASE` as repo secrets, commit `keys/comemory.pub`.

---

## 2. Cutting a stable release

The one-liner: **`just release X.Y.Z`**. The seven steps inside:

### Step 1 — Preflight

`scripts/validate-release.sh X.Y.Z` runs four hard checks:

1. Working tree clean (modified + staged; untracked is fine).
2. Current branch is `main` (override with `RELEASE_BRANCH=...`).
3. `Cargo.toml` `version = "X.Y.Z"`.
4. `CHANGELOG.md` has a `## [X.Y.Z] - YYYY-MM-DD` heading dated today.

Plus soft warnings: dirty `Cargo.lock`, unset `git user.email`, the
latest CI run on the tip commit not green. Any hard check fails → exit
1, nothing changes.

### Step 2 — `cargo set-version`

Bumps `Cargo.toml` + `Cargo.lock`. Requires `cargo-edit` (see one-time
setup). Re-run preflight if you need to verify.

### Step 3 — Draft the CHANGELOG section

`just changelog` prints a Keep-a-Changelog-formatted draft of the
conventional commits since the last tag. Paste it under
`## [Unreleased]` in `CHANGELOG.md`, edit the bucket names, then
rename the heading to `## [X.Y.Z] - YYYY-MM-DD`. The recipe pauses for
the edit (read the prompt).

### Step 4 — Re-validate

Same four hard checks as step 1, run after the CHANGELOG edit. Catches
typos in the heading (date, version, bracket placement).

### Step 5 — Commit the bump

`Cargo.toml` + `Cargo.lock` + `CHANGELOG.md`, message
`chore(release): X.Y.Z`. If the recipe reports "nothing staged", the
CHANGELOG edit didn't save — `git restore` and redo step 3.

### Step 6 — `dist plan` dry-run

`just release-dry-run vX.Y.Z`. Read the printed plan: it should show
**one** local build job (`aarch64-apple-darwin`), one global job, and
the version matches. Reject and fix if anything is off (e.g.
`Cargo.toml` version drifted, target list regressed).

### Step 7 — Tag + push

`git tag vX.Y.Z` + `git push origin main vX.Y.Z` in a single push. The
push triggers `release.yml`. The recipe prints the workflow URL.

```bash
# Manual fallback (everything in one push so the tag can't be
# created before the commit is on main):
git tag vX.Y.Z
git push origin main vX.Y.Z
```

---

## 3. Cutting a release candidate

For pre-release tags (`X.Y.Z-rc.N`), use **`just release-rc X.Y.Z-rc.N`**
instead of `just release`. The RC variant:

- Refuses plain semver (`0.11.0`) — only accepts versions with a
  pre-release suffix.
- Prints a banner before continuing: "this will create a PRE-RELEASE
  on GitHub; the Homebrew tap will NOT be updated."
- Everything else (preflight, bump, changelog, commit, dry-run, tag,
  push) is the same as the stable flow.

The Homebrew tap is gated off by `publish-prereleases = false` in
`[workspace.metadata.dist]`, and the release-finalize smoke test
treats the RC tarball identically to a stable one.

---

## 4. Post-tag verification

- [ ] `release.yml` is green: plan → build → host (the dist-generated
  workflow). 1 build job, 1 global job, 1 host job for a single-target
  release.
  `https://github.com/Falconiere/comemory/actions?query=workflow%3ARelease`
- [ ] `release-finalize.yml` is green: smoke test + curated notes + (if
  configured) signature.
  `https://github.com/Falconiere/comemory/actions?query=workflow%3A%22Release+Finalize%22`
- [ ] The GitHub Release body matches the curated `## [X.Y.Z]` section
  from `CHANGELOG.md` (not cargo-dist's auto-blob).
  `https://github.com/Falconiere/comemory/releases/tag/vX.Y.Z`
- [ ] `SHA256SUMS` is attached. If minisign is configured,
  `SHA256SUMS.minisig` is also attached.
- [ ] `Falconiere/homebrew-tap` formula updated (skipped for RC).
  `https://github.com/Falconiere/homebrew-tap/commits/main`
- [ ] Smoke test on a clean machine:
  ```bash
  brew update && brew install Falconiere/tap/comemory
  comemory --version      # should print X.Y.Z
  comemory doctor         # should report a healthy data dir
  ```

---

## 5. Rollback

If a bad tag was pushed:

```bash
git push --delete origin vX.Y.Z
git tag --delete vX.Y.Z
gh release delete vX.Y.Z --repo Falconiere/comemory --yes
```

If the tap formula was already pushed for a bad stable tag, manually
revert the commit in `Falconiere/homebrew-tap`:

```bash
gh repo clone Falconiere/homebrew-tap /tmp/htap
cd /tmp/htap
git revert HEAD
git push origin main
```

If `release-finalize.yml`'s smoke test fails *after* the release was
created, the dist-generated `host` job already uploaded the artifact.
Delete the release (the command above) and the artifacts go with it;
re-tag once `main` is fixed.

### Failure-mode quick reference

| Symptom | Cause | Fix |
|---|---|---|
| `plan` job fails on "Validate release preflight" | One of the 4 hard checks failed | Read the `validate-release` step log; the failing check is named |
| `host` job fails mid-upload | cargo-dist couldn't upload to the release (network, perms) | Retry the workflow run; if it persists, check `HOMEBREW_TAP_TOKEN` |
| `release-finalize.yml` smoke test fails | The tarball is malformed (missing binary, too small) | Delete the release, investigate the build artifacts (downloaded to the `artifacts-*` workflow run artifacts) |
| `release-finalize.yml` signs step warns "minisign not installed" | Maintainer hasn't configured signing | Expected — release is published unsigned. To enable, see one-time setup step 4 |
| Homebrew tap didn't update | Stable tag was pushed but the formula didn't land | Re-run the `publish-homebrew-formula` job from the Actions UI; check the PAT scopes |

---

## 6. Platform support

macOS aarch64 is the only prebuilt target by design — see
[README § Platform support](../README.md#platform-support). Linux and
Windows users fork the repo and `cargo install --path .`. The
single-target build keeps the release CI matrix to one job, which is
the reason for the choice; pre-built Linux/Windows artifacts can be
re-enabled in a fork by editing `targets = [...]` in
`[workspace.metadata.dist]` and the corresponding GitHub runner
matrix.
