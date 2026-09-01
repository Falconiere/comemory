# api/doctor/

**What belongs here:** the individual `comemory doctor` health probes and
the pass that runs all of them — split out of `src/api/doctor.rs` once the
`checks: Vec<Check>` array (console-compat step 8) pushed that file toward
the 300-line ceiling.

**What does NOT belong here:** the command's entry point, `Request` /
`Report` shape, the writability-first probe ordering, and the
forward-compat `SchemaTooNew` fallback, which stay in `src/api/doctor.rs`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `checks.rs` | `run_all` | ten named probes (`data dir writable`, `mirror parity`, `schema version`, `migration backup`, `fts5 tokenizer`, `sqlite-vec`, `repo roots`, `embed command`, `markdown/db counts`, `data dir layout`) plus `Check` (the `{name, status, detail}` row shape) and `Extras` (every scalar the probes derive, merged onto `Report`'s original six fields by `doctor.rs`'s `assemble`). Every probe only reads its connection, so the same pass runs against the primary read-write connection or the forward-compat read-only fallback |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api/doctor.rs`
(`pub mod checks;`) and callers import concrete paths.
