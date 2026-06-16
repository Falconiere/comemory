# Release runbook (comemory)

Single-tag-triggered pipeline via cargo-dist. Push `vX.Y.Z` → GitHub
Actions runs `release.yml` → builds the single `aarch64-apple-darwin`
target → uploads the tarball + shell installer to GitHub Releases →
pushes the formula to `Falconiere/homebrew-tap` (on stable tags only).

## Pre-flight (every release)

1. Confirm `main` is clean and green:

   ```bash
   git status
   bash scripts/check-all.sh
   ```

2. Confirm prereqs (one-time): tap repo exists, PAT secret set.

   ```bash
   gh secret list --repo Falconiere/comemory | grep HOMEBREW_TAP_TOKEN
   ```

## Cut a release

1. Bump `Cargo.toml` `version = "X.Y.Z"`.
2. Run `just changelog` to draft a section from the conventional commits
   since the last tag. Paste it under `## [Unreleased]` in `CHANGELOG.md`,
   edit the bucket names, then move it under a dated `## [X.Y.Z] — YYYY-MM-DD`
   heading (Keep-a-Changelog 1.1.0 format).
3. Commit on `main`:

   ```bash
   git commit -am "release: vX.Y.Z"
   ```

4. Local dry-run:

   ```bash
   just release-dry-run vX.Y.Z
   ```

   Inspect printed plan. Reject and fix if Cargo.toml/tag/CHANGELOG mismatch.

5. Tag + push:

   ```bash
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

6. Watch `release.yml` in GitHub Actions. Verify:

   - plan-gate green
   - 4 build jobs green
   - host job uploaded artifacts to GH Release `vX.Y.Z`
   - publish-homebrew job updated `Falconiere/homebrew-tap` (stable tag) or skipped (RC tag)

## RC dry-run

Use tag suffix `-rc.N`. Cargo.toml version must include the suffix
(e.g. `version = "0.2.0-rc.1"`). cargo-dist marks the GH Release as
pre-release and does NOT touch the tap.

## Rollback

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

Then re-tag once `main` is fixed.

## Supported targets

- `aarch64-apple-darwin`

Linux and Windows users fork the repo and `cargo install --path .`. The
single-target build is by design — see [README § Platform support](../README.md#platform-support).

## End-user install paths

```bash
# macOS / Linuxbrew — prebuilt aarch64-apple-darwin
brew install Falconiere/tap/comemory

# Linux / Windows — fork and build from source
git clone https://github.com/<you>/comemory && cd comemory
cargo install --path .
```

After install, run `comemory doctor` to verify.
