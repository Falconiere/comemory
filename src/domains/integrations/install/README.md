# install/

**What belongs here:** the embedded half of `comemory install`.

| File | Primary item | Purpose |
| --- | --- | --- |
| `bundle.rs` | `extract` | Embeds the repository-root `integrations/agent/**` assets with `include_str!` and writes a versioned local marketplace atomically — staging directory, then rename. Re-extracting over an identical tree is a no-op; a file the operator has edited is refused with the edit left in place, and a symlinked destination or catalog is refused before anything is written |

`src/domains/integrations/install.rs` beside this folder holds the host
validation, the configuration-directory precedence, the per-version and
per-host `.installed-<host>` markers, the hook-dependency probes and the native
plugin-manager calls.
