default: check

check:
    bash scripts/check-all.sh

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

perf:
    bash scripts/build-perf.sh

# Print the cargo-dist plan for a tag without uploading anything.
# Usage: just release-dry-run v0.2.0-rc.1
release-dry-run tag:
    dist plan --tag {{tag}}
