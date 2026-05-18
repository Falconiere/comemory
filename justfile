default: check

check:
    bash scripts/check-all.sh

test:
    bash scripts/test-run.sh

qa:
    bash scripts/check-all.sh
    bash scripts/deny-check.sh
    bash scripts/dup-check.sh

bench:
    cargo bench --all-features

build-release:
    cargo build --release

e2e:
    bash scripts/e2e.sh
