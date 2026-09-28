# Test managed replica negotiation

The managed platform requires `X-Comemory-Sync-Protocol: repository-policy-v1`
and its current `X-Comemory-Policy-Revision`, including on replica routes.
Successful replies echo both headers. Replica manifests and request bodies
continue to use `replica-v1`. A stale revision returns `sync_policy_changed`;
an unsupported policy protocol returns `sync_upgrade_required`.

`tests/replica_managed.rs` exercises the real managed API and the current CLI,
including both refusals and `sync --action verify` reaching a replica report.
It is opt-in because it needs an existing organization-scoped credential:

```bash
COMEMORY_MANAGED_AUTH_FILE=/absolute/path/to/auth.json \
  cargo test --all-features --test replica_managed -- --ignored
```

Use a test workspace on a real platform deployment. The test copies only the
auth file into an isolated temporary store and disables the coordinator via
`COMEMORY_SYNC_DAEMON=0`. Verification can download workspace data and repair
local differences; the test creates no memories or local outbox to upload.
The CLI child has a 120-second deadline; direct HTTP calls have 30-second
timeouts. The temporary store is removed when the test ends. Credentials and workspace
data are never committed. Missing credentials or an unavailable API fail the
explicit test run instead of being replaced with a simulated platform.

The ordinary unit/integration suites continue to exercise real unmanaged
engines. Their lack of a repository-policy gate is why this separate managed
regression is necessary (issue #356).
