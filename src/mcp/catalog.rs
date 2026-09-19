//! The curated MCP tool table.
//!
//! Eleven entries, not the 94-route HTTP table: an agent host budgets every
//! tool description into every turn. Each row names the clap subcommand whose
//! core the tool runs, which is what `tests/mcp__parity.rs` walks to prove no
//! tool invents a command or a parameter the CLI does not have.
//!
//! `description` is agent-facing copy, reused verbatim as the rmcp
//! `#[tool(description = …)]` attribute, so it says what the tool returns and
//! when to reach for it.

/// One row of [`TOOLS`].
pub struct ToolEntry {
    /// The MCP tool name, as the host lists and calls it.
    pub name: &'static str,
    /// The `comemory` subcommand whose core this tool runs.
    pub command: &'static str,
    /// Whether the tool writes. A `--read-only` session refuses every
    /// mutating row with a tool-level `read_only` error.
    pub mutating: bool,
    /// Agent-facing description: what comes back, and when to call it.
    pub description: &'static str,
}

/// The catalog, in the order a host lists it: reads first, writes last.
pub const TOOLS: &[ToolEntry] = &[
    ToolEntry {
        name: "find",
        command: "find",
        mutating: false,
        description: "Recall memories and code for a natural-language question. \
Returns ranked hits across both corpora plus a query_id — call this first, \
before exploring a repository, and report the verdict back through `feedback`.",
    },
    ToolEntry {
        name: "search",
        command: "search",
        mutating: false,
        description: "Search saved memories only. Returns ranked memory hits with \
their ids, titles and scores plus a query_id; use it when you already know the \
answer is a recorded decision, convention or bug rather than code.",
    },
    ToolEntry {
        name: "search_code",
        command: "search-code",
        mutating: false,
        description: "Search indexed code symbols only. Returns ranked functions, \
types and methods with their file, line span and repo plus a query_id; use it to \
locate an implementation before reading files.",
    },
    ToolEntry {
        name: "context",
        command: "context",
        mutating: false,
        description: "Assemble a token-budgeted briefing for a task. Returns the \
memories and code symbols that fit the budget, already ordered — call it when \
starting work and you want one payload instead of several searches.",
    },
    ToolEntry {
        name: "show",
        command: "show",
        mutating: false,
        description: "Fetch one memory by its 8-hex id. Returns the full body, \
frontmatter and relations — call it before citing or acting on a hit, since a \
search result carries only an excerpt.",
    },
    ToolEntry {
        name: "list",
        command: "list",
        mutating: false,
        description: "Page through live memories with optional repo, kind, tag and \
quality filters. Returns a page of rows with ids and titles — use it to browse or \
audit what is stored, not to answer a question.",
    },
    ToolEntry {
        name: "edges",
        command: "edges",
        mutating: false,
        description: "List the graph edges touching a node. Returns each neighbour \
with its relation and direction — use it to follow `supersedes`, `references` and \
co-activation links out from a memory or a file.",
    },
    ToolEntry {
        name: "repos",
        command: "repos",
        mutating: false,
        description: "List the indexed repositories. Returns each repo label with \
its symbol and memory counts — use it to learn the exact label to pass as `repo` \
elsewhere.",
    },
    ToolEntry {
        name: "recall_status",
        command: "recall-status",
        mutating: false,
        description: "Report the recall loop's state for a repo since a timestamp. \
Returns tracked query counts, verdicts, saves and the pending queries with no \
verdict yet — call it to find which query_ids still owe a `feedback` call.",
    },
    ToolEntry {
        name: "save",
        command: "save",
        mutating: true,
        description: "Store a memory. Returns the new 8-hex id and file path — call \
it for a verified correction, decision or fix with evidence, passing `supersedes` \
with the ids of any memory it replaces.",
    },
    ToolEntry {
        name: "feedback",
        command: "feedback",
        mutating: true,
        description: "Record which recalled ids you actually used. Returns the \
counts stored against that query_id — call it after every recall you acted on, \
and set confirmed_by_user only when the user stated the verdict.",
    },
];

/// The row named `name`, or `None` when the catalog has no such tool.
pub fn entry(name: &str) -> Option<&'static ToolEntry> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Whether `name` is a catalogued tool that writes. An unknown name is not
/// mutating — it is not dispatchable at all, and the router refuses it before
/// the read-only gate is ever consulted.
pub fn is_mutating(name: &str) -> bool {
    entry(name).is_some_and(|t| t.mutating)
}

#[cfg(test)]
#[path = "tests/catalog.rs"]
mod tests;
