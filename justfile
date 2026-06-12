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

# Run criterion benches and write a Markdown report to docs/bench/latest.md.
bench:
    bash scripts/bench.sh

build-release:
    cargo build --release

e2e:
    bash scripts/e2e.sh

# >>> comemory claude-code plugin recipes >>>
# Claude Code plugin tests (bats, real binary). Outside the Rust gate; skips if
# bats is absent. Requires `comemory` on PATH (cargo install --path .).
claude-plugin-test:
    bash -n integrations/claude-code/scripts/comemory.sh
    bash -n integrations/claude-code/hooks/session-start.sh
    bash -n integrations/claude-code/scripts/uninstall.sh
    if command -v bats >/dev/null; then bats integrations/claude-code/tests/; else echo "bats not installed — skipping plugin tests"; fi

# Fully remove the plugin FROM THIS REPO: reverts the README link, deletes the
# plugin dir, and removes these recipes. Review with `git status` after. (To
# uninstall as an end user instead, run integrations/.../scripts/uninstall.sh.)
claude-plugin-remove:
    sed -i.bak '/^# >>> comemory claude-code plugin/,/^# <<< comemory claude-code plugin/d' justfile && rm -f justfile.bak
    sed -i.bak '/\[Claude Code plugin\](integrations\/claude-code/{N;d;}' README.md && rm -f README.md.bak
    rm -rf integrations/claude-code
    @echo "comemory Claude Code plugin removed from the repo. Review: git status"
# <<< comemory claude-code plugin recipes <<<

perf:
    bash scripts/build-perf.sh

# Print the cargo-dist plan for a tag without uploading anything.
# Usage: just release-dry-run v0.2.0-rc.1
release-dry-run tag:
    dist plan --tag {{tag}}
