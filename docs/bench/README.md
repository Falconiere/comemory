# Benchmarks

`scripts/bench.sh` (`just bench`) runs `cargo bench --all-features` and writes
the criterion output to `docs/bench/latest.md`, plus the HTML report under
`target/criterion/`.

> **Status:** there are currently no criterion bench targets in the crate.
> The v0.1 benches — which measured the old in-process embedder + LanceDB +
> kuzu stack — were dropped in the v0.2 rewrite and have not been re-added, so
> `cargo bench` compiles the crate but reports no measurements. The
> `criterion` dev-dependency and this runner are kept so new benches can be
> wired in under `benches/` without re-plumbing the harness.

When bench targets land, set their heavy fixtures up **once** before the
criterion timer starts (so headline numbers measure work, not init) and add a
"What we track" section here describing each bench against the current
SQLite + `sqlite-vec` store and the `retrieval::fuse` entry points.

## Reproducibility

Numbers vary across hardware and toolchain version. Re-run the bench on the
same host before comparing.
