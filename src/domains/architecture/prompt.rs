//! The prompt `architecture learn` hands to the caller's agent: what to do
//! with the scaffold, what the rules are, and what to print. It is plain text
//! on purpose — every agent reads it, and a human can read the file too.

use crate::domains::architecture::model::{MAX_COMPONENTS, MAX_SUMMARY, Model};
use crate::prelude::*;

/// Build the prompt for `repo` around `scaffolded`.
pub fn build(repo: &str, scaffolded: &Model) -> Result<String> {
    let json = serde_json::to_string_pretty(scaffolded)?;
    Ok(format!(
        "# Describe the architecture of `{repo}`\n\
         \n\
         Below is a deterministic scaffold of `{repo}`, clustered from its indexed\n\
         files and their mined import / co-change edges. Enrich it and print the\n\
         result.\n\
         \n\
         Rules:\n\
         \n\
         1. Print ONLY the enriched JSON model — no commentary around it.\n\
         2. Keep every `id` and `members` entry you are given unless a component is\n\
            genuinely wrong; every member path must exist in the repository, because\n\
            `comemory architecture save` validates them against the code index and\n\
            refuses the model otherwise.\n\
         3. Replace each `name` with what the component actually is, and write a\n\
            `summary` of at most {MAX_SUMMARY} characters saying what it is\n\
            responsible for. Read the code and the folder READMEs; do not guess.\n\
         4. Set each `kind` to one of module, service, layer, store, external. An\n\
            `external` component describes a dependency outside this repository and\n\
            must carry an empty `members` list.\n\
         5. Add `groups` when components belong to layers, and set each component's\n\
            `group` to that group's id.\n\
         6. Keep the mined `imports` / `co_changed` edges. Add `calls`, `depends`,\n\
            `reads`, `writes` or `publishes` edges you can justify from the code.\n\
         7. Set `\"source\": \"agent\"`. Keep `\"schema\": 1` and `\"repo\": \"{repo}\"`.\n\
         8. At most {MAX_COMPONENTS} components, and the whole document must stay\n\
            under 32 KiB.\n\
         \n\
         ## Scaffold\n\
         \n\
         ```json\n{json}\n```\n"
    ))
}
