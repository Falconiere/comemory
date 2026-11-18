---
name: verify-quality-gates
description: Verify quality-gate changes with focused real-data checks, timing evidence, and the CI toolchain.
metadata:
  toolu:
    origin: agent
    created: 2026-11-18T16:43:37Z
---
## When to Use

Changing quality-gate scheduling, build profiles, or release-history scans.

## Procedure

1. Select the CI toolchain and modern Bash before measuring:
   `export PATH="$(dirname "$(rustup which --toolchain 1.99.1 cargo)"):/opt/homebrew/bin:$PATH"`.
2. Read `docs/build-perf.md` and time the affected gate. Distinguish cold
   compilation from warm checks; concurrent builds affect wall-clock timings.
3. Keep Clippy over all targets/features as the umbrella's type-check. Reuse
   `build_dev_binary` for docs/eval. Never edit vendored `scripts/guardrails/`.
4. For migration scans, run `bash scripts/test-migration-check.sh`. Preserve
   first-release anchors, legacy/current path priority, and the union of every
   shipped migration, including files removed from later tags.
5. Run ShellCheck on changed shell scripts. Verify failure handling against a
   disposable real input; use a real Cargo project for artifact-path checks.
6. Run `bash scripts/check-all.sh` once after the changes. If the shared build
   helper changed, also run `bash scripts/eval-check.sh` and focused CLI tests
   with `cargo nextest run --locked --all-features --test cli__save --test cli_help_examples`.

## Pitfalls

- Do not remove behavior tests to improve a timing. Remove duplicate work first.
- Do not add tests that pin script spelling, private calls, or arbitrary gate order.
- Scoped structural checks apply file ceilings differently to generated snapshots.
- Keep fault injection outside the working tree, or restore the exact original bytes.

## Verification

Require a green umbrella, the affected real-data regressions, and elapsed-time
receipts. Record what was not run; a local pass does not establish a CI result.
