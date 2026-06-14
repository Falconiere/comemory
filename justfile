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
