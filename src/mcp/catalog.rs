//! The curated MCP tool table.
//!
//! Eleven entries keep tool discovery compact alongside the 94-route HTTP
//! table. Each row names the clap subcommand whose
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
        description: "Search memories, code and documents. Returns ranked hits and query_id. Start with k=3; read selected hits, then feedback.",
    },
    ToolEntry {
        name: "search",
        command: "search",
        mutating: false,
        description: "Search memories only; returns ids, titles, scores and query_id. Use k=3 for focused recall.",
    },
    ToolEntry {
        name: "search_code",
        command: "search-code",
        mutating: false,
        description: "Search indexed code; returns symbols, locations and query_id. Use k=3, then read the relevant files.",
    },
    ToolEntry {
        name: "context",
        command: "context",
        mutating: false,
        description: "Return full memory bodies and linked code/relations. k limits memory count, not tokens; prefer find then selective show for concise recall.",
    },
    ToolEntry {
        name: "show",
        command: "show",
        mutating: false,
        description: "Read one memory by its 8-hex id: full body, metadata and relations. Read selected hits before relying on them.",
    },
    ToolEntry {
        name: "list",
        command: "list",
        mutating: false,
        description: "Browse a page of live memories; filter by repo, kind, tag or quality.",
    },
    ToolEntry {
        name: "edges",
        command: "edges",
        mutating: false,
        description: "Search relation triplets by words in titles, paths or relation names. Returns ranked edges; this is not node-id adjacency lookup.",
    },
    ToolEntry {
        name: "repos",
        command: "repos",
        mutating: false,
        description: "List indexed repo labels, counts and freshness. Unscoped by default; use these labels in repo parameters.",
    },
    ToolEntry {
        name: "recall_status",
        command: "recall-status",
        mutating: false,
        description: "Report shared repo activity since a timestamp: queries, verdicts, saves and unjudged queries. Not session-specific; empty recalls need no verdict.",
    },
    ToolEntry {
        name: "save",
        command: "save",
        mutating: true,
        description: "Save a verified lesson with evidence. Returns id and path; use supersedes for replaced memories.",
    },
    ToolEntry {
        name: "feedback",
        command: "feedback",
        mutating: true,
        description: "Judge returned ids for query_id. Use confirmed_by_user only for explicit user verdicts; skip empty recalls.",
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
