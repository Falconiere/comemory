//! Document lexical and semantic ranking.

use crate::domains::retrieval::doc_route::{self, DocHit, DocOrigin};
use crate::domains::retrieval::scope::{Domain, Filters};
use crate::prelude::*;
use crate::store::Connection;
use crate::store::document_fts::{DocumentFtsHit, HitSource};

/// Combine lexical and compatible document-vector matches.
pub fn route_documents_semantic(
    conn: &Connection,
    query: &str,
    filters: Filters<'_>,
    path_globs: &[String],
    k: usize,
    vector: Option<&[f32]>,
    model: Option<&str>,
) -> Result<Vec<DocHit>> {
    let mut lexical = doc_route::route_documents(conn, query, filters, path_globs, k)?;
    let (Some(vector), Some(model)) = (vector, model) else {
        return Ok(lexical);
    };
    if k == 0 || !filters.domains.contains(Domain::Document) || query.trim().is_empty() {
        return Ok(lexical);
    }
    let matcher = doc_route::build_path_matcher(path_globs)?;
    let candidates = crate::store::document_vectors::search(conn, model, vector, usize::MAX)?;
    let mut ranked = std::collections::HashMap::new();
    for (rank, hit) in lexical.iter().enumerate() {
        ranked.insert(hit.document_id.clone(), 1.0_f64 / (61 + rank) as f64);
    }
    let mut seen = std::collections::HashSet::new();
    for (id, ordinal, score) in candidates {
        if seen.contains(&id) {
            continue;
        }
        let hit = DocumentFtsHit {
            source: HitSource::Local(id.clone()),
            ordinal,
            score,
        };
        if let Some(mut doc) = doc_route::build_hit(conn, filters.repo, matcher.as_ref(), hit, 0)? {
            seen.insert(id.clone());
            let rank = seen.len();
            *ranked.entry(id.clone()).or_insert(0.0) += 1.0 / (60 + rank) as f64;
            if let Some(existing) = lexical
                .iter_mut()
                .find(|h| h.document_id == id && matches!(h.origin, DocOrigin::Local))
            {
                existing.ann = Some(score);
            } else {
                doc.ann = Some(score);
                lexical.push(doc);
            }
            if seen.len() == k {
                break;
            }
        }
    }
    lexical.sort_by(|a, b| {
        ranked
            .get(&b.document_id)
            .copied()
            .unwrap_or_default()
            .total_cmp(&ranked.get(&a.document_id).copied().unwrap_or_default())
            .then_with(|| a.document_id.cmp(&b.document_id))
    });
    lexical.truncate(k);
    Ok(lexical)
}
