# `comemory doctor`

Runtime health: data dir, schema version, sqlite-vec, tokenizer, embed
probe, repo roots, mirror drift. Never fails the process on a warning
check — those stay in `checks[]` with `status: warn`.

**Runnable tests:** `tests/cli__doctor.rs`, `tests/cli_scenario_getting_started.rs`

**HTTP:** `GET /api/v1/doctor` — covered by `tests/serve_scenario_getting_started.rs`, `tests/serve_scenario_vectors.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

_None besides globals._

## Scenarios

### doctor-01 Fresh dir

- **Flags:** `--json`
- **Setup:** empty data dir
- **Command:** `comemory doctor --json`
- **Expect:** `db_writable=true`; `schema_version` is current; `checks` is
  a non-empty array. Embed probe warns when `COMEMORY_EMBED_CMD` is unset.
- **Covered by:** `tests/cli__doctor.rs::doctor_json_emits_v2_report_shape`

### doctor-02 After a corpus

- **Flags:** `--json`
- **Setup:** saved memories + indexed repo
- **Command:** `comemory doctor --json`
- **Expect:** at least ten healthy checks; tokenizer ok; vec dims 1024 / 768.
- **Covered by:** `tests/cli__doctor.rs::doctor_reports_at_least_ten_healthy_checks_on_a_real_corpus`

### doctor-03 Mirror drift

- **Flags:** `--json`
- **Setup:** a markdown file the mirror does not know about
- **Command:** `comemory doctor --json`
- **Expect:** a warn check; exit still 0.
- **Covered by:** `tests/cli__doctor.rs::doctor_mirror_drift_warns_and_still_exits_zero`

### doctor-04 Newer db

- **Flags:** `--json`
- **Setup:** `schema_meta` stamped newer than this binary
- **Command:** `comemory doctor --json` vs any mutating command
- **Expect:** doctor falls back; other commands exit 70.
- **Covered by:** `tests/cli__doctor.rs::doctor_falls_back_on_a_newer_db_while_every_other_command_still_exits_70`
