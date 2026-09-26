# `comemory watch`

Follow the organization's changes and pull whenever something changed. This is
the pull direction's answer to the question push answered by moving into
`save`: how a second machine sees a change without polling.

With the required resident coordinator running (#257, the ordinary case),
`watch` attaches to *that* coordinator instead of holding its own platform
connection: `--once` subscribes, asks for one `catch_up`, reports `pulled`,
and exits — a subscription or catch-up that never completes fails with exit
69, not a retry. Without `--once` it reports one `pulled` event per
`pass_finished` the coordinator emits (its own ticks, hook wakes, and channel
nudges all count), re-attaching with the same jittered backoff below if the
subscription drops — a coordinator restart included.

Only under the `COMEMORY_SYNC_DAEMON=0` harness switch, where no coordinator
exists to attach to, does `watch` hold the platform's workspace channel
itself, as it always did: the socket carries **nudges**, never memories.
Every frame — `hello` on connect, `change` after anyone writes — triggers the
same cursored pull a manual `comemory sync --action pull` would run, so a
missed frame costs latency and a duplicate frame costs one empty pull. A
refused or dropped socket is not an error: it reconnects with jittered
backoff (1s → 30s) until interrupted.

**Runnable tests:** `tests/cli__watch.rs` (the harness-switch fallback path),
`tests/replica_daemon_4.rs` (attached to a real coordinator),
`src/domains/sync/tests/watch.rs`

**HTTP:** none — `transport: "cli-only"`. It talks to the platform's
`POST /v1/ws/ticket` and `GET /v1/ws`, and holding a socket open until
interrupted is not a request-response shape the local engine could mirror.

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--once` | `false` | Pull once the channel greets, then exit — for scripts and smoke checks |

## Scenarios

### watch-01 A nudge triggers a pull that writes the memory

- **Flags:** `--once`
- **Setup:** logged in against the platform fixture, which holds one memory
  this machine does not have
- **Command:** `comemory watch --once --json`
- **Expect:** the run mints a ticket (`POST /v1/ws/ticket`), upgrades
  (`GET /v1/ws`), and the announced memory is on disk under `memories/`
  afterwards.
- **Covered by:** `tests/cli__watch.rs::watch_pulls_what_the_channel_announces`

### watch-02 Watching without a login is a usage error

- **Flags:** `--once`
- **Command:** `comemory watch --once`
- **Expect:** non-zero exit; stderr points at `comemory auth login`.
- **Covered by:** `tests/cli__watch.rs::watch_without_a_login_is_a_usage_error`

### watch-03 Reconnect backoff is bounded at both ends

- **Flags:** _(none)_
- **Expect:** the schedule never waits less than 1s or more than 30s, and a
  fraction outside `[0, 1]` cannot push it out of that range.
- **Covered by:** `src/domains/sync/tests/watch.rs::backoff_grows_to_a_thirty_second_ceiling_and_never_drops_below_a_second`
