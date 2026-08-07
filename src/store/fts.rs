//! FTS5 wrappers for memory and code lexical search: the shared MATCH
//! builders, the code leg, and the parse-error plumbing. The memory leg's
//! four ladder tiers live in [`crate::store::fts_memory`] and are
//! re-exported here, so `fts::search_memory*` stays the call-site path.

use rusqlite::{Connection, params};

use crate::prelude::*;

/// Memory leg, re-exported from [`crate::store::fts_memory`] so callers
/// (and this module's doc links) keep one `fts::` path per FTS helper.
pub use crate::store::fts_memory::{
    MemoryFtsHit, search_memory, search_memory_expanded, search_memory_relaxed,
    search_memory_subtokens,
};

/// FTS5 hit for the code table; lower `score` (BM25) = better match.
pub struct CodeFtsHit {
    /// Identifier of the matched `code_symbols` row.
    pub symbol_id: i64,
    /// BM25 relevance score; lower is better.
    pub score: f32,
}

/// Insert a row into the `memory_fts` virtual table indexing the memory body and tags.
pub fn index_memory(conn: &Connection, memory_id: &str, body: &str, tags_csv: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_fts(memory_id, body, tags) VALUES(?1, ?2, ?3)",
        params![memory_id, body, tags_csv],
    )?;
    Ok(())
}

/// Maximum number of sanitized terms used when building MATCH expressions.
/// Multi-KB queries are clamped to this many terms so the FTS5 expression
/// stays bounded; terms past the cap are silently dropped.
const MAX_QUERY_TERMS: usize = 32;

/// Split `query` on whitespace into FTS5-safe terms: embedded double quotes
/// are stripped so user text is always treated as data, never as MATCH
/// syntax, and the list is clamped to [`MAX_QUERY_TERMS`]. Shared by
/// [`build_match_query`], [`build_or_query`], and [`term_count`].
fn sanitize_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .take(MAX_QUERY_TERMS)
        .collect()
}

/// Number of sanitized terms in `query` — exactly the terms the MATCH
/// builders quote (quote-stripped, empty terms dropped, clamped to the
/// same [`MAX_QUERY_TERMS`] cap). Used by the router's relaxed-fallback
/// guard so the guard and the builders cannot disagree on what counts as
/// a term.
pub fn term_count(query: &str) -> usize {
    sanitize_terms(query).len()
}

/// Build a strict FTS5 MATCH query: every whitespace term double-quoted
/// (quotes stripped from input — terms are data, never syntax), last term
/// prefix-matched so an in-progress final word still hits. Clamped to the
/// first [`MAX_QUERY_TERMS`] (32) sanitized terms; when clamped, the
/// prefix `*` lands on the 32nd kept term rather than the query's true
/// final word.
pub fn build_match_query(query: &str) -> String {
    let terms = sanitize_terms(query);
    let n = terms.len();
    terms
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i + 1 == n {
                format!("\"{t}\"*")
            } else {
                format!("\"{t}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quote every token and join with ` OR `. This is the FTS5-injection
/// guard shared by every OR-tier builder: terms are data, never syntax —
/// the surrounding double quotes make FTS5 treat each token as a string
/// literal (callers must have stripped embedded quotes already, which
/// [`sanitize_terms`] and the identifier tokenizer both guarantee).
fn quote_or_join(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Build a relaxed OR query over the same sanitized terms, clamped to the
/// first [`MAX_QUERY_TERMS`] (32). The last-term prefix `*` used by
/// [`build_match_query`] is intentionally dropped here: the relaxed tier
/// already broadens recall via OR, and prefixing on top of that would
/// trade precision for matches the strict tier never implied. Used as the
/// fallback tier when the strict AND query finds nothing.
pub fn build_or_query(query: &str) -> String {
    quote_or_join(&sanitize_terms(query))
}

/// Build an OR query over the identifier sub-tokens of every sanitized
/// term, for the final recall tier when both the strict AND and the
/// word-level OR tiers come back empty.
///
/// Each sanitized term is split via the identifier tokenizer
/// ([`crate::store::tokenizer::split::split_text`]): `VecDimMismatch`
/// yields the parts `vec`/`dim`/`mismatch` plus the colocated whole
/// `vecdimmismatch`. The colocated whole is deliberately *included* in the
/// OR expression — it can only add recall (a verbatim-identifier mention
/// still matches via its colocated index token) and never subtracts, so
/// the expression for `VecDimMismatch` is
/// `"vec" OR "vecdimmismatch" OR "dim" OR "mismatch"`.
///
/// Returns an empty string when no individual term actually split (a term
/// "splits" when it yields more than one non-colocated part): plain-word
/// queries gain nothing from this tier, and the empty expression signals
/// "no subtoken expansion possible" so [`search_memory_subtokens`]
/// short-circuits to an empty result. The split check is per-term — an
/// aggregate distinct-part count would wrongly suppress expansion when
/// parts collide across terms (`VecDim vec` splits but its parts
/// `vec`/`dim` number no more than its terms).
pub fn build_subtoken_or_query(query: &str) -> String {
    let terms = sanitize_terms(query);
    let mut tokens: Vec<String> = Vec::new();
    let mut any_term_split = false;
    for term in &terms {
        let mut non_colocated_parts = 0usize;
        for tok in crate::store::tokenizer::split::split_text(term) {
            if !tok.colocated {
                non_colocated_parts += 1;
            }
            if !tokens.contains(&tok.text) {
                tokens.push(tok.text);
            }
        }
        if non_colocated_parts > 1 {
            any_term_split = true;
        }
    }
    if !any_term_split {
        return String::new();
    }
    quote_or_join(&tokens)
}

/// Minimum mined support for an expansion to be applied at query time.
/// Below this, a mapping is visible in `comemory mine` reports but
/// never changes retrieval (one observation is an anecdote, not a rule).
pub const EXPANSION_MIN_SUPPORT: i64 = 2;

/// Maximum expansions OR-ed in per query term (highest support first).
pub const MAX_EXPANSIONS_PER_TERM: usize = 2;

/// Build the tier-4 MATCH expression: every query token OR-ed with its
/// mined expansions (support >= [`EXPANSION_MIN_SUPPORT`], at most
/// [`MAX_EXPANSIONS_PER_TERM`] each, highest support first, ties broken
/// alphabetically). Returns an empty string when no token has an
/// applicable expansion — the tier has nothing the earlier tiers did
/// not already try, so the caller short-circuits to empty.
///
/// Both the lookup keys and the base OR-tokens come from
/// [`crate::store::tokenizer::split::query_token_list`] — the exact
/// tokenization mining uses when it writes `query_expansions` keys
/// (identifier-split, lowercased, diacritic-folded). A mapping mined
/// from the failed query `Café sizing` is stored under `cafe`, and the
/// same fold here makes the lookup hit; the folded base tokens likewise
/// match the FTS index tokens. The token list is clamped to
/// [`MAX_QUERY_TERMS`] like every other builder.
pub fn build_expanded_or_query(conn: &Connection, query: &str) -> Result<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut any_expansion = false;
    let mut stmt = conn.prepare_cached(
        "SELECT expansion FROM query_expansions
          WHERE term = ?1 AND support >= ?2
          ORDER BY support DESC, expansion
          LIMIT ?3",
    )?;
    for key in crate::store::tokenizer::split::query_token_list(query)
        .into_iter()
        .take(MAX_QUERY_TERMS)
    {
        if !tokens.contains(&key) {
            tokens.push(key.clone());
        }
        let expansions: Vec<String> = stmt
            .query_map(
                params![key, EXPANSION_MIN_SUPPORT, MAX_EXPANSIONS_PER_TERM as i64],
                |r| r.get(0),
            )?
            .collect::<std::result::Result<_, _>>()?;
        for e in expansions {
            if !tokens.contains(&e) {
                tokens.push(e);
                any_expansion = true;
            }
        }
    }
    if !any_expansion {
        return Ok(String::new());
    }
    Ok(quote_or_join(&tokens))
}

/// Prepare and drain an FTS5 query, downgrading MATCH parse errors at both
/// the prepare and the row-iteration stage to an empty result. Shared by
/// the memory ([`crate::store::fts_memory`]) and code paths so they cannot
/// drift on parse-error handling.
pub(crate) fn run_fts_query<R, P, F>(
    conn: &Connection,
    sql: &str,
    params: P,
    row_fn: F,
) -> Result<Vec<R>>
where
    P: rusqlite::Params,
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<R>,
{
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) if is_fts5_parse_error(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    collect_with_fts5_guard(stmt.query_map(params, row_fn), &mut out)?;
    Ok(out)
}

/// Drain a `query_map` result iterator, downgrading any FTS5 MATCH parse
/// error to an empty result and propagating every other SQLite error.
/// Shared by [`search_memory`] and [`search_code`] so the memory and code
/// FTS paths cannot drift on parse-error handling.
fn collect_with_fts5_guard<R, F>(
    iter: rusqlite::Result<rusqlite::MappedRows<'_, F>>,
    out: &mut Vec<R>,
) -> Result<()>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<R>,
{
    let mapped = match iter {
        Ok(it) => it,
        Err(e) if is_fts5_parse_error(&e) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for row in mapped {
        match row {
            Ok(hit) => out.push(hit),
            Err(e) if is_fts5_parse_error(&e) => {
                out.clear();
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Best-effort classification for FTS5 MATCH-expression parse errors.
/// FTS5 surfaces these as `SQLITE_ERROR` with a `fts5:` prefix or, on older
/// builds, a `syntax error near "<token>"` message. Used by [`search_memory`]
/// / [`search_code`] to downgrade a user-typed malformed query to an empty
/// result rather than aborting the wider retrieval pipeline.
///
/// The matcher is deliberately narrow:
/// - `fts5: ...` prefix is the canonical modern shape.
/// - `syntax error near` is the legacy shape; we require the trailing
///   `near` token so an unrelated SQLite error whose message happens to
///   contain `syntax error` (e.g. a `SQLITE_CORRUPT` text on the FTS5
///   shadow table) doesn't silently truncate results to an empty success.
pub fn is_fts5_parse_error(e: &rusqlite::Error) -> bool {
    let s = e.to_string().to_lowercase();
    s.starts_with("fts5:") || s.contains("syntax error near")
}

/// Insert a row into the `code_fts` virtual table for a code symbol.
///
/// `path_tokens` should be the *raw* relative path: the `identifier`
/// tokenizer splits on `/`, `.`, `-` and camelCase/digit boundaries
/// itself, and pre-lowercasing would destroy the camelCase boundaries it
/// needs (`MyComponent.tsx` would index as the single token
/// `mycomponent` instead of `my` + `component`). This matches what the
/// 0004 migration backfill feeds it (`SELECT … path FROM code_symbols`).
pub fn index_code(
    conn: &Connection,
    symbol_id: i64,
    symbol: &str,
    snippet: &str,
    path_tokens: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO code_fts(symbol_id, symbol, snippet, path_tokens) \
         VALUES(?1, ?2, ?3, ?4)",
        params![symbol_id, symbol, snippet, path_tokens],
    )?;
    Ok(())
}

/// Run a BM25 search over `code_fts` and return the top-`k` symbol hits.
///
/// The query is rewritten via [`build_match_query`] and ranked with a
/// weighted BM25 whose `(symbol, snippet, path_tokens)` column weights
/// come from `weights` (`cfg.retrieval.code_bm25_weights`; with the
/// default `(2.0, 1.0, 1.5)` symbol-name hits outrank path hits, which
/// outrank snippet hits). Like the memory matcher in
/// [`crate::store::fts_memory`], the weights are
/// interpolated into the SQL text — `bm25()` arguments cannot be bound
/// parameters, and the values come from validated config, never raw user
/// input. Optional `repo` and `lang` filters are applied via a JOIN on
/// `code_symbols` so the lexical and ANN branches share the same scope
/// when the code route runs a filtered hybrid query. FTS5 MATCH parse
/// errors are downgraded to an empty result for the same reason as
/// [`search_memory`] — a typo in the user query should not abort the
/// wider retrieval pipeline.
pub fn search_code(
    conn: &Connection,
    query: &str,
    k: usize,
    repo: Option<&str>,
    lang: Option<&str>,
    weights: (f32, f32, f32),
) -> Result<Vec<CodeFtsHit>> {
    let match_expr = build_match_query(query);
    if match_expr.is_empty() || k == 0 {
        return Ok(Vec::new());
    }
    let (w_sym, w_snip, w_path) = weights;
    let sql = format!(
        "SELECT code_fts.symbol_id, bm25(code_fts, 0.0, {w_sym}, {w_snip}, {w_path}) AS score \
           FROM code_fts \
           JOIN code_symbols c ON c.id = code_fts.symbol_id \
          WHERE code_fts MATCH ?1 \
            AND (?3 IS NULL OR c.repo = ?3) \
            AND (?4 IS NULL OR c.lang = ?4) \
          ORDER BY score \
          LIMIT ?2"
    );
    run_fts_query(
        conn,
        &sql,
        params![match_expr, k as i64, repo, lang],
        |row| {
            Ok(CodeFtsHit {
                symbol_id: row.get(0)?,
                score: row.get(1)?,
            })
        },
    )
}

#[cfg(test)]
#[path = "tests/fts.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fts_2.rs"]
mod tests_2;
