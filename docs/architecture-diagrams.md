# Architecture diagrams

Mermaid views of how comemory is put together, checked against the source at
`v0.59.0` (`0c35b097`). [Architecture](architecture.md) is the prose
explanation; these diagrams are the map. Where the two disagree, the code
wins. The [known drift](#known-drift-in-architecturemd) section at the end
lists every place this was the case when the diagrams were drawn.

1. [Components and storage](#1-components-and-storage)
2. [Save flow](#2-save-flow)
3. [Memory search pipeline](#3-memory-search-pipeline)
4. [`find`: the unified search](#4-find-the-unified-search)
5. [Code indexing](#5-code-indexing)

## 1. Components and storage

Three delivery adapters run the same command cores. `mcp` imports neither of
the others; `serve` borrows the clap `Cli` type from `cli` for its
completion and meta routes; `cli` launches both as subcommands. Only `store`
names `rusqlite`.

```mermaid
flowchart TB
    subgraph Clients
        DEV["Developer terminal"]
        AGENT["Agent hosts<br/>claude · codex · cursor-agent · muse · hermes"]
        CONSOLE["console / local HTTP clients"]
        GITHOOK["git hooks: post-commit · post-merge<br/>post-checkout · post-rewrite<br/>run: comemory sync --action auto &"]
        DAEMON["sync daemon<br/>(sync daemon ensure)"]
    end

    subgraph Adapters
        CLI["cli<br/>clap → dispatch → TTY or --json · sysexits<br/>also launches serve and mcp"]
        MCP["mcp: stdio, 19 tools<br/>find · search · search_code · context · show · list<br/>edges · repos · recall_status · save · feedback<br/>project_list/show/propose/evidence<br/>architecture_scaffold/show/check/save"]
        SERVE["serve: axum on 127.0.0.1, /api/v1<br/>token: header · Bearer · ?token · cookie<br/>Host guard · no CORS layer<br/>jobs + SSE: job events, activity feed"]
    end

    DEV --> CLI
    GITHOOK --> CLI
    AGENT --> MCP
    CONSOLE --> SERVE
    CLI -->|"comemory serve"| SERVE
    CLI -->|"comemory mcp"| MCP
    SERVE -.->|"imports cli::Cli + completion_script"| CLI

    subgraph Shared["utilities (shared)"]
        U1["context::Ctx (lazy DB conn) · blocking::run_blocking<br/>error_code::classify · exit_code · pagination::Page"]
        U2["simhash · file_lock · path_containment · repo_root<br/>activity · telemetry · embed · secret_scan · ref_args · query_id"]
    end
    CLI & MCP & SERVE --> Shared

    CFG["config<br/>defaults → config.toml → env<br/>(only --data-dir / COMEMORY_DATA_DIR from flags)"]

    subgraph Domains["src/domains"]
        MEM["memories<br/>save · show · trash · frontmatter · mirror · recover"]
        RET["retrieval<br/>search · find (unified) · context · search-code · suggest"]
        CODE["code<br/>ast-grep indexing · git hooks · repo admin · lazy policy"]
        DOCS["documents<br/>sources · document indexing · share"]
        GRAPH["graph<br/>edges · cochange · imports · pagerank<br/>memory_rank · derived refresh"]
        LEARN["learning<br/>feedback · eval · mine · tune · bandit · judge"]
        MAINT["maintenance<br/>doctor · gc · prune · rebuild · backup · upgrade"]
        ARCH["architecture<br/>scaffold · check · mermaid · save · learn"]
        SYNC["sync<br/>platform push/pull (device login) · daemon<br/>replica · memory-store git"]
        CAP["capture (CLI only)<br/>Claude transcripts · redact · distill"]
        PROJ["projects<br/>charter · plans · evidence · export/import"]
        INTEG["integrations<br/>install hosts · setup detect → plan → apply"]
    end
    Shared --> Domains
    CFG -.-> Domains
    DAEMON --> SYNC

    subgraph Store["store: the only module that uses rusqlite"]
        CONN["connection::open<br/>.open.lock → WAL, busy_timeout, FKs<br/>→ preflight → migrate"]
        ORM["orm bridge (toolu-orm) · identifier tokenizer"]
    end
    Domains --> Store

    subgraph Disk["~/.comemory/"]
        MD[("memories/{id}-{slug}.md<br/>SOURCE OF TRUTH")]
        TRASH[("memories/.trash/")]
        TOML[("config.toml")]
        subgraph DB["comemory.db"]
            T1["memory: memories · memory_tags · memory_fts · memory_substring<br/>memory_vec 1024d · memory_write_intent · memory_needs_embedding"]
            T2["code: code_symbols · code_fts · code_vec 768d · code_ref<br/>indexed_files · repo_marker · index_runs · index_failures · code_generation"]
            T3["documents: documents · document_chunks · document_fts<br/>document_share · source_roots · source_files"]
            T4["graph: edges · edge_fts"]
            T5["learning: retrieval_log · feedback · feedback_events · code_feedback<br/>query_expansions · bandit_arms · eval_runs · candidate_*"]
            T6["sync/replica: sync_* · replica_* · remote_code_* · remote_document_*"]
            T7["projects + 14 project_* · project_changes"]
            T8["schema_meta · activity_log · replica_device · gc_runs"]
        end
    end
    MEM --> MD
    MEM --> TRASH
    CFG -.-> TOML
    Store --> DB
    MD -.->|"comemory rebuild"| T1
    SYNC <-->|"platform push/pull"| PLATFORM["comemory platform"]
    SYNC <-->|"memory-store pull/commit/push"| GITREMOTE["git remote of memories/"]
```

## 2. Save flow

The order the code runs in (`src/domains/memories/save.rs`,
`save_persist.rs`). The lock is taken before the prior lookup, and the
database is opened only after it, so an id collision never opens or migrates
the database. Save never runs git; the CLI alone pushes to the platform.

```mermaid
sequenceDiagram
    autonumber
    participant C as Caller (CLI / HTTP / MCP)
    participant M as memories::save
    participant L as memory-save.lock
    participant FS as memories/*.md
    participant S as SQLite
    participant G as graph::derived
    participant P as sync::push_on_save

    C->>M: body, kind, tags, refs, optional vector
    M->>M: validate quality, fold title, id = sha256(body)[:8]
    M->>M: validate supersedes
    M->>M: collect refs (git blob + commit anchors, warnings)
    M->>M: parse vector, ensure dirs
    M->>L: acquire (waits up to pause_wait, else Busy)
    M->>FS: prior lookup: live file, then .trash
    alt same id, different hash
        M-->>C: IdCollision (409 / exit 65), DB never opened
    end
    M->>S: open DB lazily (preflight + migrate)
    opt vector given
        M->>S: dim guard vs schema_meta (rejects NaN/inf)
    end
    M->>S: SimHash near-dup scan (advisory duplicate_of)
    M->>S: record memory_write_intent
    M->>FS: .{id}.tmp → {id}-{slug}.md, purge trash copy
    M->>S: BEGIN IMMEDIATE: memories + tags + fts, edges,<br/>code_ref, memory_vec if vector, replica journal,<br/>clear intent, COMMIT
    alt mirror fails
        M-->>C: markdown kept, hint: comemory rebuild
    end
    M->>G: refresh_derived_best_effort (memory rank + edge_fts)
    M->>S: activity_log record (success or failure)
    opt CLI only, sync.push_on_save on
        M->>P: push outbox to platform (bounded, never fails the save)
    end
    M-->>C: {id, path, created, duplicate_of?, warnings?}
```

## 3. Memory search pipeline

`search`, `context`, and the memory leg of `find` (`src/domains/retrieval/`).
The lexical ladder runs in both the lexical and hybrid arms. When the graph
leg fires, it re-fuses every leg, so base scores change too.

```mermaid
flowchart TD
    Q["query + vector? + --repo / --kind / --since / --until / --as-of"] --> ROUTE{"router"}
    ROUTE -->|"vector + non-blank query"| HYB["hybrid arm"]
    ROUTE -->|"vector + blank query"| VEC["pure ANN arm"]
    ROUTE -->|"no vector"| LEX["lexical arm"]

    HYB --> ANN["ANN on memory_vec<br/>drop cosine below 0.55"]
    HYB --> LAD
    LEX --> LAD
    VEC --> ANN2["ANN only, source = vector"]

    subgraph LAD["FTS5 BM25 ladder (stops at first tier with hits)"]
        T1["tier 1: strict"] -->|"0 hits"| T2["tier 2: word-OR (2+ terms)"]
        T2 -->|"0 hits"| T3["tier 3: subtoken-OR"]
        T3 -->|"0 hits"| T4["tier 4: mined expansions (support 2+)"]
    end

    ANN & LAD --> RRF["RRF, k = rrf_k (default 60)"]
    RRF --> PROV["provisional ranking"]
    ANN2 --> PROV

    PROV -->|"top 8 seeds"| GW["graph walk: recursive CTE, 2 hops, undirected<br/>allowlist: supersedes · conflicts_with · derived_from<br/>relates_to · references_file · references_symbol · co_activated<br/>caps 4096 walk / 256 expanded"]
    GW -->|"new ids, source graph, tier 0"| REF["rrf_multi over base legs + graph<br/>(replaces base scores)"]
    PROV --> REF

    REF --> RR["rerank<br/>norm(rrf) × activation × feedback × quality × rank<br/>clamped to prior_clamp, then × supersede 0.2<br/>drops deleted rows"]
    RR --> DIV["diversify: SimHash collapse, then MMR<br/>(token Jaccard, no embeddings)"]
    DIV --> PAGE["window: pool = clamp(offset + 2·limit, 50, 200)<br/>slice page, total = in-window count"]
    PAGE --> TRK{"track on?<br/>(off for eval/tune/read-only HTTP)"}
    TRK -->|"yes"| LOG["retrieval_log every page<br/>access bump only at offset 0<br/>returns query_id"]
    TRK -->|"no"| OUT
    LOG --> OUT["Page of hits + score_parts"]
```

## 4. `find`: the unified search

`find` is the default MCP recall tool. It is not the pipeline above end to
end: it runs up to three legs at one pool size and fuses them
(`src/domains/retrieval/unified/`). Time and kind filters reach only the
memory leg.

```mermaid
flowchart LR
    F["find query<br/>--domain all|memory|code|document"] --> ML["memory leg<br/>route → rerank → diversify<br/>(section 3 without telemetry)"]
    F --> CL["code leg: search_code_hits<br/>filters: repo, lang"]
    F --> DL["document leg: BM25 on document_fts<br/>one hit per document, no priors<br/>filters: repo, path globs"]
    ML --> FU["rrf_multi_weighted<br/>memory 1.0 · code 1.0<br/>document = document_leg_weight<br/>ids memory: / code: / document:"]
    CL --> FU
    DL --> FU
    FU --> PG["one paginate over fused list"]
    PG --> LG["retrieval_log source=find (memory ids only)<br/>head window bumps memory + code access"]
```

## 5. Code indexing

Every trigger ends in `index_code::run` (`src/domains/code/index_code.rs`).
The file walk is one transaction. Graph mining is a separate best-effort
transaction after it. `last_head` is stamped inside the walk;
`last_mined_commit`, which the lazy check reads, advances at the end of
mining.

```mermaid
flowchart TD
    subgraph Triggers
        MAN["comemory index-code"]
        HOOK["git hook → sync --action auto<br/>→ index_checkout + refresh_stale"]
        DMN["daemon sync cycle → refresh_stale"]
        JOB["POST /api/v1/code/index (job)"]
        LAZY["lazy (default): search-code / context<br/>HEAD ≠ last_mined_commit → detached child<br/>debounce 200 ms + same-HEAD guard"]
    end
    MAN & HOOK & DMN & JOB & LAZY --> OPEN["open git repo, then DB<br/>worktree label → main · refuse archived"]
    OPEN --> TX["BEGIN IMMEDIATE (whole walk)<br/>--mode full clears indexed_files"]
    TX --> WALK["walk: .gitignore + hidden filtered<br/>language by extension: rust · ts/tsx · js · py · go"]
    WALK --> SKIP{"git-index blob OID<br/>= indexed_files?"}
    SKIP -->|"same / untracked / no lang"| NEXT["skip"]
    SKIP -->|"changed"| EXT["purge old rows → ast-grep extract<br/>code_symbols + code_fts + chunk children (parent_id)<br/>collect imports · upsert indexed_files"]
    EXT --> STAMP["stamp repo_marker: root_path · last_head · format<br/>COMMIT"]
    STAMP --> MAT["materialize (separate best-effort tx, skipped if no symbols)<br/>a. co_changed from git (cursor, 1000 first run, skip commits over 20 files)<br/>b. imports for re-indexed files<br/>c. weighted PageRank → rank_score<br/>d. co-activation reward (if reinforce.enabled)<br/>e. advance last_mined_commit"]
    MAT --> DER["refresh_derived_best_effort"]
    DER --> RUN["index_runs row · CLI: sync after_index push"]

    BYO["ingest-code NDJSON<br/>repo, path, blob_oid, symbol, kind, lang, lines,<br/>snippet, simhash, embedding (+ parent_symbol, chunk_index)"] -->|"one tx, dim guard"| CV[("code_symbols + code_fts + code_vec + indexed_files")]

    subgraph SC["search-code"]
        CR["BM25: symbol 2.0 · snippet 1.0 · path 1.5<br/>+ ANN ≥ 0.50 if vector · RRF when both legs hit<br/>no ladder"]
        CRR["rerank: norm relevance × PageRank × ACT-R activation<br/>× working-set affinity × Beta feedback<br/>then merge chunks into parent<br/>no supersede / quality / MMR"]
        CR --> CRR
    end
```

## Known drift in architecture.md

When these diagrams were drawn, `architecture.md` disagreed with the code in
these places:

- §2 lists config layering as ending in "CLI flags"; `cli::load_config` is
  defaults → file → env, and only `--data-dir` comes from a flag.
- §6 step 5 says save runs `git add + commit + push` under
  `COMEMORY_GIT_AUTO_SYNC`; save never runs git (`[git] auto_sync` is read
  only by `comemory sync`'s memory-store flow), and the CLI pushes to the
  platform through `sync::push_on_save`.
- §7 says one transaction per file; the walk is one transaction, and graph
  mining runs in a second one.
- §7 gives `ingest-code` rows as `{qualified, snippet, embedding}`; the real
  row has eleven required fields and rejects unknown ones.
- §8 says the git hooks run `index-code`; there are four hooks
  (`post-rewrite` included) and they run `comemory sync --action auto`.
