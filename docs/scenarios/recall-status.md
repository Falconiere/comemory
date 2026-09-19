# `comemory recall-status`

Report tracked queries, verdicts, saves and the still-pending recalls (a
`find`/`search`/`search-code`/`context` query with no `feedback` verdict yet)
for the window `[since, now)`, optionally scoped to one repo. The shared
middle is `domains::learning::recall_status`, reused verbatim by the HTTP
route and the MCP `recall_status` tool. Must **not** create the database on a
fresh data dir (like `comemory stats`). An explicit `--since` carrying an
offset (e.g. `+05:00`) is normalised to UTC before it is echoed back or
compared against the store, since every column it compares against is UTC
`Z` text.

**Runnable tests:** `tests/cli__recall_status.rs`

**HTTP:** `GET /api/v1/learning/recall-status` — covered by `tests/serve__routes__learning.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | unset | Restrict every count to this repo. Unset reports across every repo |
| `--since` | start of the current UTC day | Lower time bound — an RFC3339 timestamp or a bare `YYYY-MM-DD` date, the same grammar `search --since` accepts |

## Scenarios

### recall-status-01 A tracked recall is pending until judged

- **Flags:** `--repo` `--json`
- **Setup:** `save` one memory in a repo, then a real tracked `find` against
  it (`query_id` in the `--json` envelope)
- **Command:** `comemory recall-status --repo <repo> --json`
- **Expect:** `queries=1`, `feedback_events=0`, `saves=1`, `pending` names
  the tracked query id. After `comemory feedback <query_id> --used <id>`,
  `feedback_events=1` and `pending` is empty.
- **Covered by:** `tests/cli__recall_status.rs::a_tracked_recall_is_pending_until_judged`

### recall-status-02 A future `--since` reports all zeros

- **Flags:** `--since` `--repo`
- **Setup:** the same save + tracked `find`
- **Command:** `comemory recall-status --repo <repo> --since 2999-01-01T00:00:00Z --json`
- **Expect:** `queries`, `feedback_events`, `saves` are all `0` and `pending`
  is empty — the window excludes everything already written.
- **Covered by:** `tests/cli__recall_status.rs::a_since_in_the_future_reports_all_zeros_and_no_pending`

### recall-status-03 An unparsable `--since` is a usage error

- **Flags:** `--since`
- **Command:** `comemory recall-status --since nonsense`
- **Expect:** exit 64 (`EX_USAGE`); stderr names `since`.
- **Covered by:** `tests/cli__recall_status.rs::an_unparsable_since_is_a_usage_error`

### recall-status-04 TTY view

- **Flags:** `--repo` (no `--json`)
- **Setup:** the same save + tracked `find`
- **Command:** `comemory recall-status --repo <repo>`
- **Expect:** a header line reporting `queries=`/`feedback_events=`/`saves=`/`pending=`
  and one line per pending query naming its `query_id`.
- **Covered by:** `tests/cli__recall_status.rs::tty_output_prints_the_header_and_the_pending_line`

### recall-status-05 Empty dir creates no database

- **Flags:** `--json`
- **Command:** `comemory recall-status --json` on a fresh data dir
- **Expect:** zeros, `pending` empty; `comemory.db` is not created.
- **Covered by:** `src/domains/learning/tests/recall_status.rs::a_missing_database_reports_zeros_without_creating_one`

### recall-status-06 A `--since` offset normalises to UTC

- **Flags:** `--since` `--repo`
- **Setup:** save one memory now
- **Command:** `comemory recall-status --repo <repo> --since <the same instant expressed in +05:00> --json`
- **Expect:** `saves=1` (the same instant still counts the save); `since` in
  the response ends in `Z`, not `+05:00`.
- **Covered by:** `src/domains/learning/tests/recall_status.rs::an_offset_since_is_normalised_to_utc_before_comparing`
