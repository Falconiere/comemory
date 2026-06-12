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

# Claude Code plugin tests (bats, real binary). Outside the Rust gate; skips if
# bats is absent. Requires `comemory` on PATH (cargo install --path .).
claude-plugin-test:
    bash -n integrations/claude-code/scripts/comemory.sh
    bash -n integrations/claude-code/hooks/session-start.sh
    if command -v bats >/dev/null; then bats integrations/claude-code/tests/; else echo "bats not installed — skipping plugin tests"; fi

perf:
    bash scripts/build-perf.sh

# Print the cargo-dist plan for a tag without uploading anything.
# Usage: just release-dry-run v0.2.0-rc.1
release-dry-run tag:
    dist plan --tag {{tag}}
