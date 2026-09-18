//! The per-domain filters one benchmark task applies, and the rule that
//! refuses one whose only leg the task does not run.
//!
//! Every filter narrows specific legs: `kind` and the time bounds the memory
//! leg, `lang` the code leg, `path` the document leg, `repo` the memory and
//! code legs. A task that sets a filter for a leg it excludes would narrow
//! nothing and quietly contradict its own recorded filter set, so it fails to
//! load rather than running.

use serde::{Deserialize, Serialize};

use crate::domains::learning::evaluation::candidate_identity::CandidateDomain;
use crate::domains::retrieval::scope::{Domain, Domains};
use crate::prelude::*;

/// The per-domain filters one task applies.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskFilters {
    /// Narrows the memory and code legs.
    #[serde(default)]
    pub repo: Option<String>,
    /// Canonical lowercase memory kind. Narrows the memory leg only.
    #[serde(default)]
    pub kind: Option<String>,
    /// Source language. Narrows the code leg only.
    #[serde(default)]
    pub lang: Option<String>,
    /// Git-style globs. Narrow the document leg only.
    #[serde(default)]
    pub path: Vec<String>,
    /// Created-date lower bound. Narrows the memory leg only.
    #[serde(default)]
    pub since: Option<String>,
    /// Created-date upper bound. Narrows the memory leg only.
    #[serde(default)]
    pub until: Option<String>,
    /// As-of cutoff; also scopes the supersede penalty. Memory leg only.
    #[serde(default)]
    pub as_of: Option<String>,
}

/// Map a candidate domain onto the retrieval domain mask's vocabulary.
pub fn domain_of(domain: CandidateDomain) -> Domain {
    match domain {
        CandidateDomain::Memory => Domain::Memory,
        CandidateDomain::Code => Domain::Code,
        CandidateDomain::Document => Domain::Document,
    }
}

/// Refuse a filter whose only leg this task does not run: it would narrow
/// nothing and quietly change what the recorded filters claim.
pub fn validate_task_filters(
    id: &str,
    filters: &TaskFilters,
    scope: &str,
    mask: Domains,
) -> Result<()> {
    let f = filters;
    let memory = mask.contains(Domain::Memory);
    let inert: [(&str, bool, Domain); 6] = [
        ("kind", f.kind.is_some(), Domain::Memory),
        ("since", f.since.is_some(), Domain::Memory),
        ("until", f.until.is_some(), Domain::Memory),
        ("as_of", f.as_of.is_some(), Domain::Memory),
        ("lang", f.lang.is_some(), Domain::Code),
        ("path", !f.path.is_empty(), Domain::Document),
    ];
    for (key, present, needs) in inert {
        let satisfied = match needs {
            Domain::Memory => memory,
            other => mask.contains(other),
        };
        if present && !satisfied {
            return Err(Error::Config(format!(
                "task `{id}`: `{key}` narrows the {} leg only, which `domain: {scope}` excludes",
                leg_label(needs)
            )));
        }
    }
    Ok(())
}

/// The leg name a filter narrows, for the inert-filter message.
fn leg_label(domain: Domain) -> &'static str {
    match domain {
        Domain::Memory => "memory",
        Domain::Code => "code",
        Domain::Document => "document",
    }
}
