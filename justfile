default: check

check:
    bash scripts/check-all.sh

# Apply rustfmt formatting in place.
fmt:
    bash scripts/fmt-fix.sh

test:
    bash scripts/test-run.sh

qa:
    bash scripts/check-all.sh
    bash scripts/deny-check.sh
    bash scripts/dup-check.sh
    bash scripts/machete-check.sh

# Run the test suite under cargo-llvm-cov; enforce coverage-floor.txt if present.
coverage:
    bash scripts/coverage-check.sh

# Full-crate mutation run (slow; nightly CI job). Survivor => non-zero exit.
mutation:
    bash scripts/mutation-check.sh full

# Score lexical retrieval against the frozen golden corpus.
eval:
    bash scripts/eval-check.sh

# Run criterion benches and write a Markdown report to docs/bench/latest.md.
bench:
    bash scripts/bench.sh

build-release:
    cargo build --release

e2e:
    bash scripts/e2e.sh

perf:
    bash scripts/build-perf.sh

# Print the cargo-dist plan for a tag without uploading anything.
# Usage: just release-dry-run v0.2.0-rc.1
release-dry-run tag:
    dist plan --tag {{tag}}

# Bare preflight check. Useful when you want to test the state without
# actually doing a release. Same checks as `just release` step 1.
# Usage: just release-validate 0.11.0
release-validate ver:
    bash scripts/validate-release.sh {{ver}}

# MANUAL FALLBACK. The primary release path is the release-plz bot: push to
# main, then review + merge the auto-opened release PR. Use this recipe only
# when the bot is unavailable. Cuts a stable release end-to-end. Steps:
#   1. Preflight (4 hard checks; fails loud on any miss)
#   2. cargo set-version (requires `cargo install cargo-edit`)
#   3. Write the CHANGELOG section by hand (under ## [Unreleased])
#   4. Re-validate
#   5. Commit the bump
#   6. dist plan dry-run (read the output, then continue)
#   7. Tag + push (triggers release.yml + release-finalize.yml)
# Usage: just release 0.11.0
release ver:
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ver}}"
    printf '\n=== step 1/7: preflight ===\n'
    bash scripts/validate-release.sh "$ver"

    printf '\n=== step 2/7: cargo set-version ===\n'
    if ! cargo set-version --help >/dev/null 2>&1; then
        echo "error: 'cargo set-version' not found." >&2
        echo "install with: cargo install cargo-edit --locked" >&2
        exit 1
    fi
    cargo set-version "$ver"

    printf '\n=== step 3/7: write the CHANGELOG section ===\n'
    printf '\nManual fallback (release-plz normally authors this). By hand, add a\n'
    printf '"## [%s] - %s" heading under "## [Unreleased]" in CHANGELOG.md,\n' \
        "$ver" "$(date -u +%Y-%m-%d)"
    printf 'bucketed Added / Changed / Fixed / Removed / Security / Internal.\n'
    read -r -p "press enter when CHANGELOG.md is ready (or Ctrl-C to abort)... "

    printf '\n=== step 4/7: re-validate ===\n'
    bash scripts/validate-release.sh "$ver"

    printf '\n=== step 5/7: commit the bump ===\n'
    git add Cargo.toml Cargo.lock CHANGELOG.md
    if git diff --cached --quiet; then
        echo "error: nothing staged after the bump — did you forget to edit CHANGELOG.md?" >&2
        exit 1
    fi
    git commit -m "chore(release): $ver"

    printf '\n=== step 6/7: dist plan dry-run ===\n'
    just release-dry-run "v$ver"
    read -r -p "press enter to tag + push (or Ctrl-C to abort)... "

    printf '\n=== step 7/7: tag + push ===\n'
    git tag "v$ver"
    git push origin main "v$ver"
    echo
    echo "tag v$ver pushed. watch the workflows at:"
    echo "  https://github.com/Falconiere/comemory/actions?query=workflow%3ARelease"
    echo "  https://github.com/Falconiere/comemory/actions?query=workflow%3A%22Release+Finalize%22"

# Like `release`, but for a release candidate. The Homebrew tap is NOT
# updated (cargo-dist marks the GH release as pre-release; the formula
# publish step is gated off by `publish-prereleases = false` in
# [workspace.metadata.dist]). The version must include the suffix, e.g.
# 0.11.0-rc.1.
# Usage: just release-rc 0.11.0-rc.1
release-rc ver:
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ver}}"
    if [[ "$ver" != *-* ]]; then
        echo "error: release-rc requires a pre-release suffix (e.g. 0.11.0-rc.1)" >&2
        echo "  for a stable release, use 'just release' instead." >&2
        exit 1
    fi
    printf '\n*** RC release: this will create a PRE-RELEASE on GitHub. ***\n'
    printf '*** The Homebrew tap (Falconiere/homebrew-tap) will NOT be updated. ***\n\n'
    read -r -p "press enter to continue (or Ctrl-C to abort)... "
    just release "$ver"
