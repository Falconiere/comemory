# Release runbook (comemory)

> **TL;DR — releasing is a pull request you merge:**
>
> 1. Land your changes on `main` as usual (conventional-commit messages).
> 2. The **release-plz** bot opens/updates a **release PR** that bumps the
>    version and writes the `CHANGELOG.md` section from your commits.
> 3. Review the PR (sanity-check the version bump + changelog), then **merge it**.
> 4. Merging makes release-plz push the `vX.Y.Z` tag, which fires the cargo-dist
>    pipeline. Verify the workflows are green (§4).
>
> No `just release`, no hand-edited version, no babysitting. The manual
> `just release` flow (§3) survives only as a fallback for when the bot is down.

The release-plz bot (`.github/workflows/release-plz.yml`, config
`release-plz.toml`) owns **version + CHANGELOG + tag**. cargo-dist owns
everything after the tag: push `vX.Y.Z` → `release.yml` builds the
`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, and
`aarch64-unknown-linux-gnu` targets → uploads the tarballs + shell installer to GitHub
Releases → pushes the formula to `Falconiere/homebrew-tap` (stable tags only).
A second hand-maintained workflow, `release-finalize.yml`, runs after the
release is published to smoke-test the artifact, curate the release body from
`CHANGELOG.md`, and (optionally) sign the `sha256.sum`.

```
push to main ──> release-plz ──> [release PR] ──merge──> push vX.Y.Z tag
                                                              │
                            release.yml (build + GitHub Release + Homebrew)
                                                              │
                            release-finalize.yml (smoke + curate notes + sign)
```

---

## 1. One-time setup

- [ ] **The publish App is installed on `Falconiere/comemory` itself** with
  **Contents: read and write** + **Pull requests: read and write**. A tag pushed
  with the default `GITHUB_TOKEN` does **not** trigger downstream workflows, so
  the bot must push the tag with its own token, or `release.yml` never fires.
  Both release-plz jobs mint a short-lived installation token from
  `HOMEBREW_APP_ID` + `HOMEBREW_APP_PRIVATE_KEY` — the same App the tap step
  below uses, so this is the *same* installation with `comemory` added
  alongside `homebrew-tap`, not a second one. Unlike the `RELEASE_PLZ_TOKEN` PAT
  it replaces, an App key is not tied to a person's account and cannot silently
  expire.

  The mint steps pass no `owner`/`repositories`, unlike the tap step which has to
  cross repos, so each token is scoped to `comemory` alone.

  Note that a GitHub App has one installation per account with a single shared
  permission grant, so adding **Pull requests: read and write** here also exposes
  that permission on `homebrew-tap` and on git-better, which need only
  `Contents`. Accepted knowingly: one already-synced key beats a second key to
  install, sync and rotate.

  If a `v*` tag-protection ruleset exists, add the App as a bypass actor (branch
  protection on `main` does not cover tags).
- [ ] **Enable the bot.** Set the repo **variable** `RELEASE_PLZ_ENABLED=true`
  (Actions → Variables). Both release-plz jobs are gated behind it so merging
  the workflow doesn't start cutting releases before the App is installed. It
  must be a variable, not a secret — the `vars` context cannot read secrets, so a
  secret of the same name leaves both jobs silently skipping.
  ```bash
  gh variable set RELEASE_PLZ_ENABLED --body true --repo Falconiere/comemory
  ```
- [ ] The `Falconiere/homebrew-tap` repo exists.
- [ ] The homebrew-publish GitHub App (`HOMEBREW_APP_ID` +
  `HOMEBREW_APP_PRIVATE_KEY` secrets, synced onto `Falconiere/comemory` by
  Infisical) is installed on `Falconiere/homebrew-tap` with
  **Contents: read and write**. The `publish-homebrew-formula` job mints a
  short-lived installation token from it via `actions/create-github-app-token`
  — no expiring PAT to rotate. Verify the secrets exist:
  ```bash
  gh secret list --repo Falconiere/comemory | grep -E 'HOMEBREW_APP_ID|HOMEBREW_APP_PRIVATE_KEY'
  ```
  If the App is not installed on the tap, the job fails with "not installed";
  install it from the App's settings page.
- [ ] `cargo install cargo-edit --locked` (only for the manual `just release`
  fallback, which uses `cargo set-version`).
- [ ] *(Optional, for signed releases)* Set up the minisign keypair per
  [`keys/README.md`](../keys/README.md): generate the keypair, store
  the private half in 1Password, set `MINISIGN_KEY` +
  `MINISIGN_PASSPHRASE` as repo secrets, commit `keys/comemory.pub`.

---

## 2. Cutting a release (the bot)

The normal path — no local commands:

1. **Merge your work to `main`** with conventional-commit subjects
   (`feat:`, `fix:`, `refactor:`, `docs:`, …). These drive both the next
   semver and the changelog buckets.
2. **release-plz opens/updates the release PR** (job `release-plz-pr`). It bumps
   `Cargo.toml` + `Cargo.lock` and rewrites the `CHANGELOG.md` section from the
   commits since the last tag. Each new push to `main` refreshes the same PR.
3. **Review the PR.** Confirm the computed version is what you expect (breaking
   `feat!:`/`fix!:` → major-ish bump, `feat:` → minor, `fix:` → patch) and the
   changelog reads well. Edit the PR branch directly if you want to reword.
4. **Merge it.** Job `release-plz-release` then pushes the `vX.Y.Z` annotated
   tag with the App token, which triggers `release.yml`. Proceed to §4.

The changelog buckets (feat→Added, fix→Fixed, refactor/perf/style→Changed,
revert→Removed, `!`→BREAKING, docs/chore/ci/test/build→Internal, security→
Security) are defined in `release-plz.toml`'s `[changelog]` section. The heading
date is stamped when the PR is (re)built, not at merge — that's why
`validate-release.sh` accepts any ISO date.

---

## 3. Manual fallback — `just release X.Y.Z`

Use this only when the bot is unavailable. It does by hand what release-plz
automates. The seven steps inside:

### Step 1 — Preflight

`scripts/validate-release.sh X.Y.Z` runs four hard checks:

1. Working tree clean (modified + staged; untracked is fine).
2. Current branch is `main` (override with `RELEASE_BRANCH=...`).
3. `Cargo.toml` `version = "X.Y.Z"`.
4. `CHANGELOG.md` has a `## [X.Y.Z] - YYYY-MM-DD` heading (any ISO date).

Plus soft warnings: dirty `Cargo.lock`, unset `git user.email`, the
latest CI run on the tip commit not green. Any hard check fails → exit
1, nothing changes.

### Step 2 — `cargo set-version`

Bumps `Cargo.toml` + `Cargo.lock`. Requires `cargo-edit` (see one-time
setup). Re-run preflight if you need to verify.

### Step 3 — Write the CHANGELOG section

By hand, add a `## [X.Y.Z] - YYYY-MM-DD` heading under `## [Unreleased]`
in `CHANGELOG.md`, bucketed Added / Changed / Fixed / Removed / Security /
Internal. The recipe pauses for the edit (read the prompt). (The old
`just changelog` draft helper was retired when release-plz took over
changelog generation.)

### Step 4 — Re-validate

Same four hard checks as step 1, run after the CHANGELOG edit. Catches
typos in the heading (date, version, bracket placement).

### Step 5 — Commit the bump

`Cargo.toml` + `Cargo.lock` + `CHANGELOG.md`, message
`chore(release): X.Y.Z`. If the recipe reports "nothing staged", the
CHANGELOG edit didn't save — `git restore` and redo step 3.

### Step 6 — `dist plan` dry-run

`just release-dry-run vX.Y.Z`. Read the printed plan: it should show
**three** local build jobs (`aarch64-apple-darwin`,
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`), one global job,
and the version matches. Reject and fix if anything is off (e.g.
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

## 3a. Cutting a release candidate

For pre-release tags (`X.Y.Z-rc.N`), use **`just release-rc X.Y.Z-rc.N`**
(release-plz drives stable bumps only; RC cutting stays manual). The RC variant:

- Refuses plain semver (`0.11.0`) — only accepts versions with a
  pre-release suffix.
- Prints a banner before continuing: "this will create a PRE-RELEASE
  on GitHub; the Homebrew tap will NOT be updated."
- Everything else (preflight, bump, changelog, commit, dry-run, tag,
  push) is the same as the stable manual flow.

The Homebrew tap is gated off by `publish-prereleases = false` in
`[workspace.metadata.dist]`, and the release-finalize smoke test
treats the RC tarball identically to a stable one.

---

## 4. Post-tag verification

- [ ] `release.yml` is green: plan → build → host (the dist-generated
  workflow). 3 local build jobs (macOS aarch64, Linux x86_64, Linux
  aarch64), 1 global job, 1 host job.
  `https://github.com/Falconiere/comemory/actions?query=workflow%3ARelease`
- [ ] `release-finalize.yml` is green: smoke test + curated notes + (if
  configured) signature.
  `https://github.com/Falconiere/comemory/actions?query=workflow%3A%22Release+Finalize%22`
- [ ] The GitHub Release body matches the curated `## [X.Y.Z]` section
  from `CHANGELOG.md` (not cargo-dist's auto-blob).
  `https://github.com/Falconiere/comemory/releases/tag/vX.Y.Z`
- [ ] `sha256.sum` is attached (cargo-dist's name — there is no `SHA256SUMS`),
  along with a `<archive>.sha256` beside each tarball. Only the archives and
  `source.tar.gz` get one; `comemory-installer.sh`, `comemory.rb` and
  `dist-manifest.json` are unchecksummed. If minisign is configured,
  `sha256.sum.minisig` is also attached.
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
| release PR never opens | `RELEASE_PLZ_ENABLED` unset, or the `release-plz-pr` job failed | Check the variable is `true`; read the Release-plz workflow logs |
| PR merged but `release.yml` never runs | tag was pushed with the default `GITHUB_TOKEN`, not the App token | Confirm the `release` job's `GITHUB_TOKEN` is `steps.app-token.outputs.token` |
| release-plz jobs fail `403 Resource not accessible` | the App is not installed on this repo, or a permission is read-only | Install it here with Contents + Pull requests read/write; the secrets existing is not sufficient, the sync is repo-wide but the install is per-repo |
| both release-plz jobs report `skipped` with no steps | `RELEASE_PLZ_ENABLED` was set as a *secret*, not a variable | `gh variable set RELEASE_PLZ_ENABLED --body true`; `vars` cannot read secrets |
| `plan` job fails on "Validate release preflight" | One of the 4 hard checks failed | Read the `validate-release` step log; the failing check is named |
| `host` job fails mid-upload | cargo-dist couldn't upload to the release (network, perms) | Retry the workflow run; if it persists, check the job's `GITHUB_TOKEN` permissions |
| `release-finalize.yml` smoke test fails | The tarball is malformed (missing binary, too small) | Delete the release, investigate the build artifacts (downloaded to the `artifacts-*` workflow run artifacts) |
| `release-finalize.yml` signs step warns "minisign not installed" | Maintainer hasn't configured signing | Expected — release is published unsigned. To enable, see one-time setup |
| Homebrew tap didn't update | Stable tag was pushed but the formula didn't land | Re-run the `publish-homebrew-formula` job from the Actions UI; confirm the GitHub App is installed on the tap with Contents: write |

---

## 6. Platform support

macOS aarch64 and Linux (`x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`) are prebuilt — see
[README § Platform support](../README.md#platform-support). `dist plan`
picks native GitHub runners for each (`macos-14`, `ubuntu-22.04`,
`ubuntu-22.04-arm`), so no cross-compilation or container is involved;
`release.yml` itself needs no per-target edits since its build matrix is
computed at CI runtime from `[workspace.metadata.dist].targets`. Windows
users still fork the repo and `cargo install --path .`. To add or drop a
target, edit `targets = [...]` in `Cargo.toml`'s
`[workspace.metadata.dist]` — no other file needs to change.
