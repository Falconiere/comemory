# Global flags

Every subcommand inherits two long flags from `Cli`. They may appear
**before or after** the subcommand name.

## `--json`

Emit machine-readable JSON on stdout instead of the TTY view. The exact
envelope is command-specific (a `Page`, a search hit list, a doctor
report, …). Stderr still carries warnings.

- **Default:** off (TTY).
- **Covered by:** `tests/cli_scenario_globals.rs`
  `json_flag_before_and_after_subcommand_match` — `--json list` and
  `list --json` produce identical objects on the same store.

## `--data-dir <DATA_DIR>`

Override the data root (`memories/`, `comemory.db`, `config.toml`).
Honors `COMEMORY_DATA_DIR` when the flag is omitted. The **flag wins**
when both are set.

- **Default:** `$HOME/.comemory`.
- **Covered by:** `tests/cli_scenario_globals.rs`
  `data_dir_flag_wins_over_env_and_isolates_the_store`.

Every scenario in this catalog runs against a throwaway directory via
the env var or the flag. Never point these tests at a real home store.

## `--help` / `--version`

Clap builtins. `comemory --version` is the smoke test in `tests/smoke.rs`
and `scripts/e2e.sh`. `comemory <cmd> --help` must contain an `Examples:`
block — `tests/cli_help_examples.rs` walks every subcommand.

## Usage exits

Unknown subcommands and missing required args exit **2** (clap usage).
Covered by `tests/cli_scenario_globals.rs`
`unknown_subcommand_and_missing_args_are_usage_errors`. Other failures
follow `sysexits.h` (`64` usage/config-ish clap, `65` data, `69`
unavailable, `70` software, `78` config).

## HTTP twins

Every command file above names its `/api/v1` route(s) on an `**HTTP:**`
line, taken from the live `GET /api/v1/commands` inventory (the catalog
test spawns a real `comemory serve` and diffs the two). The global CLI flags
have no HTTP twin — the server's data dir is its own, and every response is
already JSON — so the HTTP-wide contracts live here instead:

- **Token:** every `/api/v1/*` request carries `X-Comemory-Token`; a
  missing token is an enveloped `401`. Covered by
  `tests/serve__routes__mod.rs::v1_health_without_token_is_an_enveloped_401`.
- **Envelope:** `{ok, data, meta}` on success, `{ok, error: {code, message}, meta}`
  on failure. Covered by
  `tests/serve__routes__mod.rs::v1_health_with_token_returns_the_envelope`.
- **Read-only:** `serve --read-only` answers `405 read_only` on every
  mutating route and keeps every read route working. Covered by
  `tests/serve__routes__mod.rs::ac4_every_mutating_route_405s_on_a_read_only_server`,
  `tests/serve__routes__mod.rs::ac4_read_routes_stay_functional_on_a_read_only_server`.
- **Confirm gate:** a destructive route without `confirm` is
  `400 confirmation_required` (`?confirm=true` on `DELETE`, `"confirm": true`
  in a `POST` body). Covered by
  `tests/serve__routes__mod.rs::require_confirm_false_maps_to_400_confirmation_required`
  and, end to end, `tests/serve_scenario_memory_lifecycle.rs`.
- **Jobs:** long-running routes answer `202` with a `job_id`; a journey
  polls `GET /jobs/{id}` to `done` through `tests/common/serve_bin.rs`.
  Lifecycle and SSE contracts: `tests/serve__routes__jobs.rs`.
