/* architecture.js — the interactive decks on architecture.html.
   Each `[data-deck]` figure is rendered from one spec below: a list of
   cards, optional directed edges between them, and the facts the inspector
   shows for the selected card. The capabilities edges are the
   `owner_dependencies` table in scripts/architecture-policy.json, restated
   here because GitHub Pages serves docs/ alone; the layers edges are the
   `use crate::…` imports ast-grep finds under src/. Keep both in step with
   the tree when the policy or the imports change. */
(function () {
  'use strict';

  var GITHUB = 'https://github.com/Falconiere/comemory/blob/main/';

  function src(path) { return '<a href="' + GITHUB + path + '" rel="noopener">' + path + '</a>'; }
  function doc(path, text) { return '<a href="documentation.html#' + path + '">' + (text || path) + '</a>'; }
  function list(items) { return '<ul>' + items.map(function (item) { return '<li>' + item + '</li>'; }).join('') + '</ul>'; }

  var LAYERS = {
    kind: 'grid',
    initial: 'domains',
    edges: [
      ['cli', 'domains'], ['cli', 'store'], ['cli', 'config'], ['cli', 'utilities'],
      ['serve', 'domains'], ['serve', 'store'], ['serve', 'utilities'], ['serve', 'cli'],
      ['domains', 'store'], ['domains', 'config'], ['domains', 'utilities'],
      ['utilities', 'store'], ['utilities', 'config'],
      ['config', 'store']
    ],
    cards: [
      { id: 'cli', name: 'cli/', sub: 'Delivery · clap subcommands, one file each, and the TTY and JSON writers.', facts: [
        ['owns', 'One file per <code>comemory &lt;subcommand&gt;</code> with its own <code>Args</code>, a thin <code>run</code>, and the output hookup; the dispatcher in <code>src/cli.rs</code>; the shared flag layers <code>pagination</code> and <code>search_only</code>; the writers in <code>cli/output/</code>.'],
        ['never', 'Business logic. A <code>cli/*.rs</code> file parses flags, loads <code>Config</code>, calls a capability core, and renders the result.'],
        ['tests', 'Crate-root integration tests: <code>tests/cli__*.rs</code> per command, <code>tests/cli_scenario_*.rs</code> for journeys. Never under <code>src/cli/</code>.'],
        ['read', list([src('src/cli.rs'), src('src/cli/README.md'), doc('cli-reference.md', 'CLI reference')])]
      ] },
      { id: 'serve', name: 'serve/', sub: 'Delivery · the loopback-only /api/v1 server, jobs, and the request gate.', facts: [
        ['owns', 'Router assembly and gating middleware, the versioned routes, the background-job model with progress and log streaming, the envelope, and the per-session bearer token and Host-header guard.'],
        ['never', 'Command logic. Every route calls the same <code>domains::&lt;capability&gt;::&lt;cmd&gt;::run</code> core the CLI calls. Since #178 nothing here imports <code>cli::output</code>.'],
        ['exception', '<code>routes/meta.rs</code> imports <code>cli::{Cli, completion_script}</code>: <code>GET /completions</code> and <code>GET /commands</code> are clap-introspection surfaces.'],
        ['read', list([src('src/serve.rs'), src('src/serve/README.md'), doc('guides/http-api.md', 'The HTTP API')])]
      ] },
      { id: 'domains', name: 'domains/', sub: 'Ten business capabilities, each owning one area of behavior end to end.', facts: [
        ['owns', 'Models, algorithms, and the command cores both delivery adapters call. A sibling <code>&lt;name&gt;.rs</code> declares each folder from <code>src/domains.rs</code>.'],
        ['never', 'Importing <code>cli</code> or <code>serve</code>. Another capability only through the directed table in <code>scripts/architecture-policy.json</code>.'],
        ['may use', '<code>store</code>, <code>config</code>, <code>errors</code>, <code>prelude</code>, and the named primitives in <code>utilities/</code>.'],
        ['read', list([src('src/domains/README.md'), doc('designs/2026-09-17-domain-first-migration.md', 'Domain-first migration contract')])]
      ] },
      { id: 'store', name: 'store/', sub: 'The SQLite chokepoint: connection, migrations, declared schema, row CRUD.', facts: [
        ['owns', 'Connection setup and PRAGMAs, the marker-keyed migration runner, the declared schema in <code>schema_*.rs</code> (toolu-orm 0.7), FTS5 and <code>vec0</code> helpers with dimension guards, the identifier tokenizer, and the row writers.'],
        ['never', 'Ranking or business logic, and since #177 any call into a domain. A passive domain model may be named only as a type, under <code>passive_store_models</code>.'],
        ['gate', 'The only module that may import <code>rusqlite</code>; no function returns a driver cursor. <code>store-leak-baseline.txt</code> ratchets both.'],
        ['read', list([src('src/store.rs'), src('src/store/README.md'), doc('guides/schema-migrations.md', 'Schema migrations'), doc('guides/runtime-orm.md', 'Runtime queries')])]
      ] },
      { id: 'utilities', name: 'utilities/', sub: 'Transport-neutral primitives, one concern per file, one implementation each.', facts: [
        ['owns', 'The execution <code>Ctx</code>, <code>pagination</code>, <code>id_list</code>, <code>when</code>, <code>ref_args</code>, <code>embedding_input</code>, <code>simhash</code>, <code>digest</code>, <code>file_lock</code>, <code>path_containment</code>, <code>progress</code>, <code>repo_root</code>, <code>telemetry</code>.'],
        ['never', 'Importing <code>cli</code>, <code>serve</code>, or <code>output</code>. A command\'s result model stays with the capability that produces it.'],
        ['declared', 'Two files reach into a capability for a type it owns (<code>memories::References</code>, <code>code::git_utils</code>), listed with a written reason under <code>shared_domain_dependencies</code>.'],
        ['read', list([src('src/utilities.rs'), src('src/utilities/README.md')])]
      ] },
      { id: 'config', name: 'config/', sub: 'Layered configuration: defaults, config.toml, COMEMORY_* environment.', facts: [
        ['owns', 'The three layers in order, <code>Paths</code> as the single resolution of the on-disk layout, and the validation pass run after every layer.'],
        ['never', 'An environment read anywhere else in the crate; the <code>no-direct-env-var</code> guardrail names this folder as its one exemption.'],
        ['declared', 'One reach into a capability: <code>sync::skip_repos::SkipMatcher</code>, because <code>validate</code> must reject an invalid glob at load and the matcher owns the rule.'],
        ['read', list([src('src/config.rs'), src('src/config/README.md'), doc('configuration.md', 'Configuration')])]
      ] }
    ]
  };

  var CAPABILITIES = {
    kind: 'grid',
    initial: 'retrieval',
    edges: [
      ['capture', 'sync'],
      ['code', 'graph'],
      ['documents', 'code'], ['documents', 'graph'],
      ['graph', 'code'], ['graph', 'learning'], ['graph', 'memories'],
      ['integrations', 'code'], ['integrations', 'documents'], ['integrations', 'maintenance'], ['integrations', 'sync'],
      ['learning', 'retrieval'],
      ['maintenance', 'code'], ['maintenance', 'documents'], ['maintenance', 'graph'], ['maintenance', 'memories'], ['maintenance', 'retrieval'], ['maintenance', 'sync'],
      ['memories', 'code'], ['memories', 'graph'], ['memories', 'retrieval'],
      ['retrieval', 'code'], ['retrieval', 'graph'], ['retrieval', 'memories'],
      ['sync', 'code'], ['sync', 'graph'], ['sync', 'memories']
    ],
    cards: [
      { id: 'memories', name: 'memories', sub: 'The memory lifecycle: the markdown record, its atomic store, and the save, delete, list, show, update, restore and trash cores.', landed: '#169' },
      { id: 'retrieval', name: 'retrieval', sub: 'Hybrid search across memories, code and documents: legs, fusion, priors, diversification, graph expansion, scope, and the search, context and find cores.', landed: '#171' },
      { id: 'code', name: 'code', sub: 'AST extraction, code indexing, the repository inventory, git hooks, and reindex freshness.', landed: '#167' },
      { id: 'graph', name: 'graph', sub: 'The edges relation graph: reference and link derivation, co-change and import mining, PageRank, neighbor walks, and the graph and edges cores.', landed: '#170' },
      { id: 'documents', name: 'documents', sub: 'Document extraction, the durable source registry, discovery, and document indexing.', landed: '#168' },
      { id: 'learning', name: 'learning', sub: 'The learning loop: feedback counters, golden sets and metrics, reformulation mining, and the feedback, eval, mine, tune and bandit cores.', landed: '#173' },
      { id: 'sync', name: 'sync', sub: 'Organization authentication, platform push and pull, the workspace channel, secret redaction, the opt-in daemon, and the git memory store.', landed: '#172' },
      { id: 'capture', name: 'capture', sub: 'Client-side coding-session capture: transcript reading, redaction receipts, explicit-save distillation.', landed: '#174' },
      { id: 'maintenance', name: 'maintenance', sub: 'Corpus health and repair: doctor, gc, prune, consolidate, the atomic rebuild, re-embedding, stats, and the binary upgrade.', landed: '#176' },
      { id: 'integrations', name: 'integrations', sub: 'The embedded agent bundle and its marketplace, host validation, and the detect, plan, apply onboarding behind comemory setup.', landed: '#175' }
    ]
  };
  CAPABILITIES.cards.forEach(function (card) {
    card.facts = [
      ['owns', card.sub],
      ['landed by', '<a href="https://github.com/Falconiere/comemory/issues/' + card.landed.slice(1) + '" rel="noopener">' + card.landed + '</a>'],
      ['read', list([src('src/domains/' + card.id + '.rs'), src('src/domains/' + card.id + '/README.md')])]
    ];
  });

  var PIPELINE = {
    kind: 'pipe',
    initial: 'route',
    cards: [
      { id: 'route', name: 'route', step: 'stage 01', sub: 'Candidates from the lexical and vector legs.', facts: [
        ['branches', list(['vector and a query: hybrid, ANN + FTS5 BM25 fused by RRF', 'vector and an empty query: pure vector', 'no vector: pure lexical'])],
        ['filters', '<code>--repo</code>, <code>--kind</code>, <code>--since</code>, <code>--until</code>, <code>--as-of</code> constrain every branch through one <code>Filters</code> bundle.'],
        ['ladder', 'When the strict lexical leg returns nothing: word-OR at 2 or more terms, then subtoken-OR, then a learned-expansion tier over mined <code>query_expansions</code>. Hits carry a tier 1 to 4; the ladder never fires on the pure-vector path.'],
        ['tokenizer', 'The <code>identifier</code> FTS5 tokenizer splits camelCase and snake_case and is baked into the DDL, so every lexical query benefits.'],
        ['file', src('src/domains/retrieval/router.rs')]
      ] },
      { id: 'expand', name: 'graph expand', step: 'stage 02', sub: 'A third leg seeded from the provisional top hits.', facts: [
        ['seeds', 'The top <code>graph_seeds</code> hits of the provisional ranking, 8 by default.'],
        ['walk', 'One recursive CTE over <code>edges</code>, depth at most <code>graph_hops</code> (default 2), both orientations, over an allowlist of kinds. The hub kinds <code>in_repo</code>, <code>authored_by</code> and <code>tagged</code> are excluded.'],
        ['fusion', 'MIN(depth) per memory, live rows only, ranked by hops then id, fused in as one more RRF list. Ids only the walk found are labelled source <code>graph</code>, tier 0.'],
        ['why', 'A memory can be lexically dark for a query while <code>edges</code> already links it to a top hit. Setting hops to 0 returns the pre-leg pipeline by construction.'],
        ['file', src('src/domains/retrieval/graph_route.rs')]
      ] },
      { id: 'rerank', name: 'rerank', step: 'stage 03', sub: 'Five multiplicative priors over the fused relevance.', facts: [
        ['formula', '<code>final = rrf × activation × feedback × quality × supersede × rank</code>'],
        ['priors', list(['activation: ACT-R, recency × access count', 'feedback: Beta-smoothed used and irrelevant counts', 'quality: frontmatter 1 to 5', 'supersede: a fixed 0.2× when a live memory supersedes this one', 'rank: <code>1 + 0.2·ln(1 + raw/median)</code> over <code>memories.rank_score</code>'])],
        ['clamp', 'Activation, feedback, quality and rank are clamped to <code>[prior_clamp.lo, prior_clamp.hi]</code>; the supersede penalty bypasses the clamp on purpose.'],
        ['neutral', 'A never-ranked corpus has median 0 and boost exactly 1.0; a ranked corpus with no edges gets a uniform 1.1386, which cannot reorder anything.'],
        ['file', src('src/domains/retrieval/rerank.rs')]
      ] },
      { id: 'diversify', name: 'diversify', step: 'stage 04', sub: 'Collapse near-duplicates, then trade relevance for spread.', facts: [
        ['simhash', 'Hits within the Hamming threshold collapse to the highest-scoring one.'],
        ['mmr', 'Maximal marginal relevance re-ranks the survivors; <code>mmr_lambda</code> blends relevance against diversity.'],
        ['window', 'The pool is <code>clamp(offset + 2·limit, 50, 200)</code>, run through fuse, rerank and diversify whole, then sliced. <code>has_more</code> goes false at the ceiling: deeper results need a refined query, not more paging.'],
        ['file', src('src/domains/retrieval/diversify.rs')]
      ] },
      { id: 'emit', name: 'emit', step: 'stage 05', sub: 'A cited bundle on the terminal or as JSON, and a head window that learns.', facts: [
        ['tty', 'One line per hit with a coloured score and its source label.'],
        ['json', '<code>{"hits":[{"memory_id","score","source","tier","superseded_by"?,"score_parts":{…}}],"query_id"}</code> inside the <code>Page</code> envelope <code>{items, limit, offset, total, has_more}</code>.'],
        ['reinforce', 'A tracked run logs <code>retrieval_log</code> at every offset but bumps <code>access_count</code> only at offset 0, so the same <code>--offset</code> returns the same page across identical runs.'],
        ['feedback', '<code>comemory feedback --query-id … --used …</code> closes the loop; <code>eval</code>, <code>mine</code> and <code>tune</code> read what it wrote.'],
        ['file', src('src/domains/retrieval/pipeline.rs')]
      ] }
    ]
  };

  var STORAGE = {
    kind: 'grid4',
    initial: 'memories',
    cards: [
      { id: 'memories', name: 'memories', step: 'memory layer', sub: 'Frontmatter and body mirror keyed by id, plus rank_score.', facts: [['purpose', 'The row mirror of every markdown file, with the SimHash and the materialized memory-graph PageRank in <code>rank_score</code>. Partial creation-order indexes serve the unfiltered, repo, kind and combined listing pages.'], ['rebuilt from', 'markdown, by <code>comemory rebuild</code>']] },
      { id: 'memory_fts', name: 'memory_fts', step: 'memory layer · fts5', sub: 'Lexical index over body and title.', facts: [['purpose', 'The BM25 leg for <code>search</code>, tokenized by the identifier tokenizer so camelCase and snake_case split.'], ['rebuilt from', 'markdown']] },
      { id: 'memory_substring', name: 'memory_substring', step: 'memory layer · fts5', sub: 'Trigram candidates for listing by literal substring.', facts: [['purpose', 'External-content FTS5 over <code>memories.body</code>, kept by insert, update and delete triggers; a match is verified with the original escaped <code>LIKE</code>. Queries under three characters keep the scan path.'], ['since', 'migration 19']] },
      { id: 'memory_vec', name: 'memory_vec', step: 'memory layer · vec0', sub: 'Dense vectors keyed by id, 1024 dimensions.', facts: [['purpose', 'The ANN leg. The dimension is locked at first save and a mismatched embedder fails fast with <code>VecDimMismatch</code>.'], ['written by', '<code>save --vector</code> / <code>--vector-stdin</code>, <code>reembed</code>']] },
      { id: 'code_symbols', name: 'code_symbols', step: 'code layer', sub: 'Symbols extracted from indexed repos, with rank_score and parent_id.', facts: [['purpose', 'File, kind, snippet and SimHash per symbol; <code>parent_id</code> points a chunk at the symbol it was split from; <code>rank_score</code> carries the materialized PageRank.'], ['rebuilt from', 'the previous database, copied across by <code>rebuild</code>; or a fresh <code>index-code</code>']] },
      { id: 'code_fts', name: 'code_fts', step: 'code layer · fts5', sub: 'Identifiers, snippets and path tokens.', facts: [['purpose', 'The weighted BM25 leg for <code>search-code</code>, over symbol, snippet and path tokens.']] },
      { id: 'code_vec', name: 'code_vec', step: 'code layer · vec0', sub: 'Dense vectors for symbols, 768 dimensions.', facts: [['purpose', 'The optional thresholded ANN leg for <code>search-code</code>.'], ['written by', '<code>ingest-code</code> from JSONL rows of <code>{qualified, snippet, embedding}</code>']] },
      { id: 'code_ref', name: 'code_ref', step: 'code layer', sub: 'Version-pinned references from a memory to code.', facts: [['purpose', 'The anchor behind <code>--ref-file</code> and <code>--ref-symbol</code>: <code>pinned_blob</code>, <code>pinned_commit</code>, <code>branch</code>. Full-replace on re-save, so a dropped ref is removed.'], ['read by', '<code>context</code>, which classifies each ref fresh, stale, ghost, unpinned or unknown']] },
      { id: 'edges', name: 'edges', step: 'graph', sub: 'Typed, weighted src to dst rows; the whole graph.', facts: [['purpose', '<code>(src_kind, src_id, edge_kind, dst_kind, dst_id, weight)</code>. Reference edges come from frontmatter; <code>co_changed</code>, <code>imports</code> and <code>co_activated</code> are mined. Walks are recursive CTEs.'], ['rebuilt from', 'markdown for reference edges; the previous database for mined ones']] },
      { id: 'edge_fts', name: 'edge_fts', step: 'graph · fts5', sub: 'Each edge rendered as searchable triplet text.', facts: [['purpose', 'Backs <code>comemory edges</code>. Refresh-materialized in one ordered statement, never written through; created empty by migration 12 and healed on first use.']] },
      { id: 'retrieval_log', name: 'retrieval_log', step: 'learning', sub: 'Every tracked query and its returned page.', facts: [['purpose', 'The query log that <code>feedback</code> resolves a <code>query_id</code> against and that <code>eval --history</code> and the golden-set harvest read.']] },
      { id: 'feedback', name: 'feedback', step: 'learning', sub: 'Aggregated used and irrelevant counters per memory.', facts: [['purpose', 'The Beta-smoothed prior in rerank. <code>feedback_events</code> beside it keeps every event with its provenance: manual, implicit, auto_search_edit or auto_coactivation.']] },
      { id: 'code_feedback', name: 'code_feedback', step: 'learning', sub: 'The same counters for code symbols.', facts: [['purpose', 'The feedback prior in <code>code_rerank</code>, written by <code>feedback --used-code</code>.']] },
      { id: 'query_expansions', name: 'query_expansions', step: 'learning', sub: 'Mined query reformulations.', facts: [['purpose', 'The fourth tier of the lexical ladder. <code>comemory mine --apply</code> distills them from the log; the harvest reads manual provenance only.']] },
      { id: 'bandit_arms', name: 'bandit_arms', step: 'learning', sub: 'Arms for the eval-gated online bandit.', facts: [['purpose', 'Thompson-sampled ranking blends behind <code>comemory bandit</code>.'], ['since', 'migration 10']] },
      { id: 'schema_meta', name: 'schema_meta', step: 'meta', sub: 'Schema version, locked dimensions, migration markers.', facts: [['purpose', 'Key/value rows. The migration runner is keyed on the markers here; a key no migration in this binary writes means a newer comemory wrote the database and the open is refused with exit 70.'], ['also', 'the <code>lazy_reindex_head:&lt;repo&gt;</code> debounce record']] },
      { id: 'repo_marker', name: 'repo_marker', step: 'meta', sub: 'Per-repo indexing cursor and working-tree root.', facts: [['purpose', '<code>last_head</code>, <code>last_mined_commit</code>, and <code>root_path</code> so <code>serve</code> can resolve a <code>file:&lt;repo&gt;:&lt;path&gt;</code> id back to disk. The lazy-reindex staleness probe is two reads here plus one HEAD resolve.']] },
      { id: 'documents', name: 'documents', step: 'document layer', sub: 'Indexed documents from registered sources.', facts: [['purpose', 'The document index and its FTS5 twin, fed by <code>comemory index</code> from the durable source registry; a third leg in <code>find</code>.']] }
    ]
  };

  var SPECS = { layers: LAYERS, capabilities: CAPABILITIES, pipeline: PIPELINE, storage: STORAGE };

  /* ---- rendering -------------------------------------------------------- */

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function renderCard(card, kind) {
    var button = el('button', 'card');
    button.type = 'button';
    button.setAttribute('aria-current', 'false');
    button.setAttribute('data-id', card.id);
    if (card.step) button.appendChild(el('span', 'card-step', card.step));
    var head = el('span', 'card-head');
    head.appendChild(el('span', 'card-name', card.name));
    var role = el('span', 'card-role', '');
    role.setAttribute('data-glyph', '');
    head.appendChild(role);
    var dot = el('span', 'dot');
    dot.setAttribute('aria-hidden', 'true');
    head.appendChild(dot);
    button.appendChild(head);
    button.appendChild(el('span', 'card-sub', card.sub));
    if (kind === 'pipe') button.querySelector('.card-name').style.display = 'block';
    return button;
  }

  function renderFacts(card, spec) {
    var rail = el('dl', 'fact-rail');
    var facts = card.facts.slice();
    if (spec.edges) {
      var uses = spec.edges.filter(function (edge) { return edge[0] === card.id; }).map(function (edge) { return '<code>' + edge[1] + '</code>'; });
      var usedBy = spec.edges.filter(function (edge) { return edge[1] === card.id; }).map(function (edge) { return '<code>' + edge[0] + '</code>'; });
      facts.push(['→ uses', uses.length ? uses.join(' · ') : '<span class="muted">nothing in this deck</span>']);
      facts.push(['← used by', usedBy.length ? usedBy.join(' · ') : '<span class="muted">nothing in this deck</span>']);
    }
    facts.forEach(function (fact) {
      var row = el('div');
      row.appendChild(el('dt', null, fact[0]));
      var dd = el('dd');
      dd.innerHTML = fact[1];
      row.appendChild(dd);
      rail.appendChild(row);
    });
    var title = el('p', 'label', card.name);
    title.style.marginTop = '1rem';
    title.style.marginBottom = '0.5rem';
    var wrap = el('div');
    wrap.appendChild(title);
    wrap.appendChild(rail);
    return wrap;
  }

  var ROLE_TEXT = { uses: ['→', 'uses'], 'used-by': ['←', 'used by'], both: ['↔', 'uses · used by'], current: ['', 'current'], none: ['', ''] };

  function applyRoles(deck, spec, activeId) {
    var uses = new Set();
    var usedBy = new Set();
    if (spec.edges) {
      spec.edges.forEach(function (edge) {
        if (edge[0] === activeId) uses.add(edge[1]);
        if (edge[1] === activeId) usedBy.add(edge[0]);
      });
      deck.setAttribute('data-active', activeId);
    }
    Array.prototype.forEach.call(deck.querySelectorAll('.card'), function (button) {
      var id = button.getAttribute('data-id');
      var role = 'none';
      if (id === activeId) role = 'current';
      else if (uses.has(id) && usedBy.has(id)) role = 'both';
      else if (uses.has(id)) role = 'uses';
      else if (usedBy.has(id)) role = 'used-by';
      button.setAttribute('aria-current', id === activeId ? 'true' : 'false');
      button.setAttribute('data-role', role);
      var label = button.querySelector('.card-role');
      var text = spec.edges || role === 'current' ? ROLE_TEXT[role] : ROLE_TEXT.none;
      label.setAttribute('data-glyph', text[0]);
      label.textContent = text[1];
    });
  }

  function renderDeck(figure) {
    var spec = SPECS[figure.getAttribute('data-deck')];
    if (!spec) return;
    var body = figure.querySelector('.viz-body');
    var listNode = el(spec.kind === 'pipe' ? 'ol' : 'ul', spec.kind === 'pipe' ? 'pipe' : spec.kind === 'grid4' ? 'deck deck-4' : 'deck');
    listNode.setAttribute('aria-label', figure.querySelector('.viz-head .label').textContent);
    spec.cards.forEach(function (card) {
      var item = el('li');
      item.appendChild(renderCard(card, spec.kind));
      listNode.appendChild(item);
    });
    var inspector = el('div', 'inspector');
    inspector.setAttribute('aria-live', 'polite');
    body.appendChild(listNode);
    body.appendChild(inspector);

    function select(id) {
      var card = spec.cards.filter(function (candidate) { return candidate.id === id; })[0];
      if (!card) return;
      applyRoles(listNode, spec, id);
      inspector.innerHTML = '';
      inspector.appendChild(renderFacts(card, spec));
    }

    listNode.addEventListener('click', function (event) {
      var button = event.target.closest('.card');
      if (button) select(button.getAttribute('data-id'));
    });
    listNode.addEventListener('keydown', function (event) {
      var keys = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };
      if (!(event.key in keys)) return;
      var buttons = Array.prototype.slice.call(listNode.querySelectorAll('.card'));
      var index = buttons.indexOf(document.activeElement);
      if (index === -1) return;
      event.preventDefault();
      var next = buttons[(index + keys[event.key] + buttons.length) % buttons.length];
      next.focus();
      select(next.getAttribute('data-id'));
    });
    select(spec.initial || spec.cards[0].id);
  }

  Array.prototype.forEach.call(document.querySelectorAll('[data-deck]'), renderDeck);
})();
