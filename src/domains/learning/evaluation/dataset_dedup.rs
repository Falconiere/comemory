//! The three "which record survives" rules of the reviewed dataset export
//! (#210): duplicate observations, verdicts that name something the
//! observation did not return, and contradictory verdicts.
//!
//! All three are deterministic and total. Nothing here guesses a label: a rule
//! either keeps a row or drops it and says which counter it went into.

use std::collections::{BTreeMap, BTreeSet};

use crate::domains::learning::evaluation::candidate_identity::CandidateIdentity;
use crate::domains::learning::evaluation::judgment::MatchOutcome;
use crate::domains::learning::judge::target_of;
use crate::prelude::*;

/// A tiebreak rank, compared lexicographically and won by the greatest.
type Rank = (u64, String, String);

/// One observation, as the duplicate collapse sees it.
pub struct DuplicateInput<'a> {
    /// `query`, `knobs_hash`, `corpus_digest` and `filters_json` joined: two
    /// observations sharing it asked the same question of the same corpus
    /// under the same ranking configuration.
    pub dedup_key: &'a str,
    /// The observation's capture instant.
    pub at: &'a str,
    /// The observation's id.
    pub observation_id: &'a str,
    /// How many exportable verdicts it carries.
    pub verdicts: u64,
}

/// One verdict, as the contradiction resolution sees it.
pub struct ContradictionInput<'a> {
    /// `query_group`, `candidate_ref` and the provenance word joined. Manual
    /// and implicit verdicts never contend: they are different evidence
    /// classes and are written to different files.
    pub key: &'a str,
    /// The grade this verdict carries.
    pub relevance: u8,
    /// When it was recorded.
    pub at: &'a str,
    /// The observation it was recorded against.
    pub observation_id: &'a str,
}

/// How one verdict relates to the candidates its observation actually returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictOutcome {
    /// It names candidate `usize` of the observation, at the version observed.
    Matched(usize),
    /// It names an identity the observation returned, at a different content
    /// version. Excluded and counted — never silently accepted as a match.
    Stale,
    /// It names nothing the observation returned. Excluded and counted — a
    /// positive retrieval did not return must never be inserted.
    RecallMiss,
}

/// The indices of the observations a duplicate collapse removes.
///
/// One observation survives per `dedup_key`: the one carrying the most
/// exportable verdicts, then the latest `at`, then the greatest
/// `observation_id`. Verdict count leads because the pools are identical by
/// construction and the verdicts are the expensive artifact.
pub fn collapse_duplicates(rows: &[DuplicateInput<'_>]) -> BTreeSet<usize> {
    let ranked: Vec<(String, Rank)> = rows
        .iter()
        .map(|row| {
            (
                row.dedup_key.to_string(),
                (
                    row.verdicts,
                    row.at.to_string(),
                    row.observation_id.to_string(),
                ),
            )
        })
        .collect();
    losers(&ranked)
}

/// The indices of the verdicts a contradiction resolution removes.
///
/// A key carrying one grade is not contested and loses nothing, even when two
/// observations recorded the same verdict. A key carrying more than one grade
/// keeps the latest verdict, ties broken by the greatest `observation_id`, and
/// drops the rest whole rather than downgrading them to unjudged — which would
/// turn a reviewer's verdict into a second kind of guess.
pub fn resolve_contradictions(rows: &[ContradictionInput<'_>]) -> BTreeSet<usize> {
    let mut grades: BTreeMap<&str, BTreeSet<u8>> = BTreeMap::new();
    for row in rows {
        grades.entry(row.key).or_default().insert(row.relevance);
    }
    let contested: Vec<(usize, &ContradictionInput<'_>)> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| grades.get(row.key).is_some_and(|g| g.len() > 1))
        .collect();
    let ranked: Vec<(String, Rank)> = contested
        .iter()
        .map(|(_, row)| {
            (
                row.key.to_string(),
                (0, row.at.to_string(), row.observation_id.to_string()),
            )
        })
        .collect();
    losers(&ranked)
        .into_iter()
        .filter_map(|local| contested.get(local).map(|(original, _)| *original))
        .collect()
}

/// How `verdict` relates to `observed`, through the contract's own matcher.
///
/// Routed through `judge::target_of` and `TargetKey::matches` rather than a
/// second comparison, so the export and `comemory judge` can never disagree
/// about what a verdict addresses. `task` names the caller in the error a
/// malformed target raises.
pub fn classify(
    verdict: &CandidateIdentity,
    observed: &[CandidateIdentity],
    task: &str,
) -> Result<VerdictOutcome> {
    let key = target_of(verdict).resolve(task)?;
    let mut stale = false;
    for (index, seen) in observed.iter().enumerate() {
        match key.matches(seen) {
            MatchOutcome::Yes => return Ok(VerdictOutcome::Matched(index)),
            MatchOutcome::Stale => stale = true,
            MatchOutcome::No => {}
        }
    }
    Ok(if stale {
        VerdictOutcome::Stale
    } else {
        VerdictOutcome::RecallMiss
    })
}

/// Group by key, keep the single greatest rank per key, and return the indices
/// of every row that lost. The shared shape of both collapse rules.
fn losers(ranked: &[(String, Rank)]) -> BTreeSet<usize> {
    let mut winner: BTreeMap<&str, (usize, &Rank)> = BTreeMap::new();
    for (index, (key, rank)) in ranked.iter().enumerate() {
        match winner.get(key.as_str()) {
            Some((_, best)) if *best >= rank => {}
            _ => {
                winner.insert(key.as_str(), (index, rank));
            }
        }
    }
    let kept: BTreeSet<usize> = winner.values().map(|(index, _)| *index).collect();
    (0..ranked.len()).filter(|i| !kept.contains(i)).collect()
}

#[cfg(test)]
#[path = "tests/dataset_dedup.rs"]
mod tests;
