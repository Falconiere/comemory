//! Reformulation mining: find query pairs where a failed query was
//! reworded into one that earned used-feedback, and distill
//! (failed_term → fix_term) expansion mappings.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::Connection;
use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::prelude::*;
use crate::store::tokenizer::split::query_tokens;

/// Reformulation window: a follow-up query counts as a rewording of a
/// failed one only when it ran within this many minutes after it.
pub const REFORMULATION_WINDOW_MIN: i64 = 10;

/// One mined (term → expansion) mapping with its observation count.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MinedMapping {
    /// Failed query term (tokenized form).
    pub term: String,
    /// Term from the successful rewording.
    pub expansion: String,
    /// Number of (failed, successful) pairs that produced this mapping.
    pub support: u64,
}

/// Scan `retrieval_log` for (failed q1 → used-feedback q2) pairs inside
/// the reformulation window and aggregate term-diff mappings by support.
/// A pair qualifies when q1 earned no used feedback, q2 did, they share
/// at least one token (same-intent guard), and q2 ran within
/// [`REFORMULATION_WINDOW_MIN`] minutes of q1. Each qualifying pair
/// contributes every (q1∖q2) × (q2∖q1) token combination. Rows with an
/// unparsable timestamp are skipped. Deterministic: rows are processed
/// in `(at, query_id)` order and the output is sorted by
/// `(term, expansion)`.
///
/// Rows with `source = 'search-code'` are excluded entirely: code-search
/// queries can only receive code-target feedback, so without memory
/// verdicts they would read as permanently failed and mint spurious
/// expansions. `source = 'context'` rows still participate — context
/// queries are first-class mining citizens since M2.
pub fn mine(conn: &Connection) -> Result<Vec<MinedMapping>> {
    let mut stmt = conn.prepare(
        "SELECT query_id, query, at FROM retrieval_log
         WHERE source != 'search-code' ORDER BY at, query_id",
    )?;
    let log: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    // Only memory-target verdicts mark a query successful: a code verdict
    // (target_kind = 'code', written by `stats::code_feedback`) says
    // nothing about memory retrieval quality, so a follow-up whose only
    // used feedback is code-target must not read as a successful rewording.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT query_id FROM feedback_events
          WHERE verdict = 'used' AND target_kind = 'memory'",
    )?;
    let used: HashSet<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;

    let parsed: Vec<(bool, BTreeSet<String>, OffsetDateTime)> = log
        .iter()
        .filter_map(|(qid, query, at)| {
            let t = OffsetDateTime::parse(at, &Rfc3339).ok()?;
            Some((used.contains(qid), query_tokens(query), t))
        })
        .collect();

    let mut counts: BTreeMap<(String, String), u64> = BTreeMap::new();
    for (i, (i_used, i_toks, i_at)) in parsed.iter().enumerate() {
        if *i_used {
            continue; // q1 must have failed
        }
        for (j_used, j_toks, j_at) in parsed.iter().skip(i + 1) {
            if (*j_at - *i_at).whole_minutes() > REFORMULATION_WINDOW_MIN {
                break; // rows are at-ordered, so every later row is too far
            }
            if !*j_used || i_toks.intersection(j_toks).next().is_none() {
                continue; // q2 must have succeeded and share intent
            }
            // The two difference sets are disjoint by construction (a
            // term in both would be in the intersection), so no
            // self-mapping (term == expansion) can ever be counted.
            for failed in i_toks.difference(j_toks) {
                for fix in j_toks.difference(i_toks) {
                    *counts.entry((failed.clone(), fix.clone())).or_insert(0) += 1;
                }
            }
        }
    }
    Ok(counts
        .into_iter()
        .map(|((term, expansion), support)| MinedMapping {
            term,
            expansion,
            support,
        })
        .collect())
}

/// Replace the whole `query_expansions` table with `mappings` in one
/// transaction. Rebuild-not-increment: support is always derived from
/// the *current* log, so gc-evicted rows stop contributing and stale
/// mappings decay on re-mine.
pub fn apply(conn: &mut Connection, mappings: &[MinedMapping], now_iso: &str) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM query_expansions", [])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO query_expansions(term, expansion, support, last_mined)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for m in mappings {
            stmt.execute(rusqlite::params![
                m.term,
                m.expansion,
                m.support as i64,
                now_iso
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}
