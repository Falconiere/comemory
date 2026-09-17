# setup/

**What belongs here:** the three phases of `comemory setup`, kept apart so
the wizard's decisions are testable without a terminal.

| File | Primary item | Purpose |
| --- | --- | --- |
| `detect.rs` | `Detected` | Read-only probe of machine + repo. Delegates to `api::doctor`, `api::hooks`, `api::repos`, `api::sources`, and `sync::auth_file`; never creates the database and never touches the network |
| `plan.rs` | `run` | Pure `(Detected, Request) -> Vec<Step>`. Every decision lives here |
| `apply.rs` | `run` | The only phase that writes. Dispatches each pending step to the command that owns it; a per-step failure is recorded, never propagated |

`src/api/setup.rs` beside this folder holds the request/response types, the
stable `STEP_IDS`, and the `run` that sequences the three.
