# integrations/

**What belongs here:** everything that makes comemory work inside an agent
host, and everything that takes a machine or a repo from "binary installed" to
"memory + code search working in my agent". The embedded bundle and its host
registration, and the detect → plan → apply onboarding composition on top.

**What does NOT belong here:** delivery, and other capabilities' work. Clap
argument shapes and host aliases, terminal detection, the `Intent`/`Prompting`
→ `Mode` decision, the wizard, the rendered summary and the exit-code mapping
stay in [`cli/`](../../cli/README.md). The authored agent assets stay in the
repository-root `integrations/agent/` tree — `install/bundle.rs` embeds that
tree, it does not own it. Git reindex hooks are `domains::code` operations and
`SessionEnd` capture behavior is `domains::capture`; `setup` calls them.

Both cores are **CLI-only** (`serve::routes::meta::CLI_ONLY`, and an exception
in `tests/api__parity.rs`): a server must never write into an operator's agent
configuration or their repository.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `install.rs` | `Request` | Shared middle of `comemory install` — validate the host, resolve its configuration directory, extract the embedded bundle, write `<bundle>/plugins/comemory/.mcp.json` naming this binary, and register the plugin with the host's native plugin manager. Connection-free: [`run`](install::run) never calls `Ctx::conn`, so installing an integration never creates a database. Host names are validated strings, not clap enums. `install/` holds the embedded `bundle` |
| `setup.rs` | `Request` | Shared middle of `comemory setup` — the stable `STEP_IDS`, the `StepState` machine, and the `run` that sequences detect → plan → apply. Conn-free until a step applies. `setup/` holds the three phases |

The per-host `.installed-<host>` marker lives under
`<data_dir>/integrations/<version>/`. It is deliberately **not** the extracted
bundle: that tree is shared by every host, so its existence says nothing about
which hosts actually have the plugin registered, and it is version-scoped so an
upgrade correctly reports a host as needing a reinstall.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/integrations.rs`.
