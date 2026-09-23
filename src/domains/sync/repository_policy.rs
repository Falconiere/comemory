//! Authoritative repository policy resolved against this machine's checkouts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::domains::code::git_utils;
use crate::domains::sync::AuthFile;
use crate::domains::sync::client_policy::{self, SyncPolicyStatus};
use crate::domains::sync::client_protocol::SYNC_PROTOCOL;
use crate::domains::sync::repository_identity::{
    canonical_github_name, canonical_github_repository,
};
use crate::domains::sync::skip_repos::normalize_repo_label;
use crate::prelude::*;
use crate::store::{Connection, repo_marker, repository_approval, sync_manifest, sync_state};

const IMPORT_GATE: &str = "repository_allowlist";

/// One negotiated policy plus the GitHub identities resolved on this machine.
#[derive(Debug, Clone)]
pub struct RepositoryPolicy {
    workspace_id: String,
    revision: i64,
    allowed: BTreeSet<String>,
    mappings: BTreeMap<String, String>,
    local_identities: BTreeMap<String, String>,
}

impl RepositoryPolicy {
    /// Fetch, validate, resolve, fingerprint, and reconcile repository policy.
    pub fn load(conn: &mut Connection, auth: &AuthFile) -> Result<Self> {
        Self::load_with_timeout(conn, auth, crate::domains::sync::client::HTTP_TIMEOUT)
    }

    /// Load policy under an explicit request timeout for inline pushes.
    pub fn load_with_timeout(
        conn: &mut Connection,
        auth: &AuthFile,
        timeout: Duration,
    ) -> Result<Self> {
        let secret = auth.effective_secret();
        let status = client_policy::fetch_with_timeout(&auth.api_url, &secret, timeout)?;
        let policy = Self::resolve(conn, status, &auth.workspace_id)?;
        policy.persist_resolved_labels(conn)?;
        sync_state::ensure(conn, &auth.workspace_id, &auth.api_url)?;
        let fingerprint = policy.fingerprint(&auth.api_url);
        let _ = sync_state::reconcile_policy(conn, &auth.workspace_id, &fingerprint)?;
        Ok(policy)
    }

    /// Policy revision carried by every managed data request.
    #[must_use]
    pub const fn revision(&self) -> i64 {
        self.revision
    }

    /// Every label this policy resolves, paired with its canonical name.
    ///
    /// An allowed canonical name resolves to itself, since
    /// [`Self::memory_repository`] accepts one directly. Mappings and local
    /// checkout identities are included only when the allowlist still covers
    /// them, so the result is exactly the set of labels that would resolve.
    #[must_use]
    pub fn resolved_labels(&self) -> Vec<(String, String)> {
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        for canonical in &self.allowed {
            out.insert(normalize_repo_label(canonical), canonical.clone());
        }
        for source in [&self.mappings, &self.local_identities] {
            for (label, canonical) in source {
                if self.allowed.contains(canonical) {
                    out.insert(normalize_repo_label(label), canonical.clone());
                }
            }
        }
        out.into_iter().collect()
    }

    /// Write [`Self::resolved_labels`] where an offline run can read it.
    ///
    /// One transaction, so a machine never observes a half-replaced map: a
    /// capture reading it mid-write would withhold a document whose repository
    /// is in fact approved, or worse, share one whose approval was revoked.
    ///
    /// # Errors
    /// Propagates SQLite failures.
    pub fn persist_resolved_labels(&self, conn: &mut Connection) -> Result<()> {
        let at = OffsetDateTime::now_utc()
            .format(&Iso8601::DEFAULT)
            .map_err(|e| Error::Other(format!("timestamp: {e}")))?;
        let tx = conn.transaction()?;
        repository_approval::replace_all(&tx, &self.resolved_labels(), &at)?;
        tx.commit()?;
        Ok(())
    }

    /// Canonical allowed repository for one local memory label.
    #[must_use]
    pub fn memory_repository(&self, label: &str) -> Option<&str> {
        let label = normalize_repo_label(label);
        if label.is_empty() {
            return None;
        }
        if let Some(canonical) = canonical_github_name(&label)
            && self.allowed.contains(&canonical)
        {
            return self.allowed.get(&canonical).map(String::as_str);
        }
        self.mappings
            .get(&label)
            .or_else(|| self.local_identities.get(&label))
            .filter(|canonical| self.allowed.contains(*canonical))
            .map(String::as_str)
    }

    /// Canonical allowed repository for one indexed local checkout.
    #[must_use]
    pub fn code_repository(&self, label: &str) -> Option<&str> {
        self.local_identities
            .get(&normalize_repo_label(label))
            .filter(|canonical| self.allowed.contains(*canonical))
            .map(String::as_str)
    }

    /// Content hashes of the local live memories authorized by this policy.
    pub fn authorized_content_hashes(&self, conn: &Connection) -> Result<Vec<String>> {
        Ok(sync_manifest::live_repository_hashes(conn)?
            .into_iter()
            .filter_map(|(hash, label)| self.memory_repository(&label).map(|_| hash))
            .collect())
    }

    fn resolve(conn: &Connection, status: SyncPolicyStatus, workspace_id: &str) -> Result<Self> {
        validate_status(&status, workspace_id)?;
        let allowed = status
            .allowlist
            .into_iter()
            .map(|repo| repo.full_name)
            .collect::<BTreeSet<_>>();
        let mut mappings = BTreeMap::new();
        for mapping in status.repo_mappings {
            let label = normalize_repo_label(&mapping.label);
            if label.is_empty() || !allowed.contains(&mapping.full_name) {
                return Err(Error::Other(
                    "sync policy contains an invalid repository mapping".into(),
                ));
            }
            if mappings
                .insert(label, mapping.full_name.clone())
                .is_some_and(|old| old != mapping.full_name)
            {
                return Err(Error::Other(
                    "sync policy contains conflicting repository mappings".into(),
                ));
            }
        }
        Ok(Self {
            workspace_id: status.workspace_id,
            revision: status.policy_revision,
            allowed,
            mappings,
            local_identities: resolve_local_identities(conn)?,
        })
    }

    fn fingerprint(&self, api_url: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(api_url.trim_end_matches('/').as_bytes());
        hasher.update(b"\0");
        hasher.update(self.workspace_id.as_bytes());
        hasher.update(self.revision.to_be_bytes());
        for value in &self.allowed {
            hasher.update(b"\0a:");
            hasher.update(value.as_bytes());
        }
        for (label, value) in &self.mappings {
            hasher.update(b"\0m:");
            hasher.update(label.as_bytes());
            hasher.update(b"=");
            hasher.update(value.as_bytes());
        }
        for (label, value) in &self.local_identities {
            hasher.update(b"\0l:");
            hasher.update(label.as_bytes());
            hasher.update(b"=");
            hasher.update(value.as_bytes());
        }
        hex_digest(hasher.finalize().as_slice())
    }
}

fn validate_status(status: &SyncPolicyStatus, workspace_id: &str) -> Result<()> {
    if status.workspace_id != workspace_id
        || status.policy_revision < 0
        || status.sync_protocol != SYNC_PROTOCOL
        || status.import_gate != IMPORT_GATE
    {
        return Err(Error::Other(
            "platform returned an unsupported sync policy".into(),
        ));
    }
    for repo in &status.allowlist {
        if canonical_github_name(&repo.full_name).as_deref() != Some(repo.full_name.as_str()) {
            return Err(Error::Other(
                "sync policy contains a non-canonical repository".into(),
            ));
        }
    }
    Ok(())
}

fn resolve_local_identities(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for label in repo_marker::all_repos(conn)? {
        let Some(root) = repo_marker::root_path(conn, &label)? else {
            continue;
        };
        let Some(remote) = git_utils::remote_url(Path::new(&root), "origin") else {
            continue;
        };
        let Some(identity) = canonical_github_repository(&remote) else {
            continue;
        };
        candidates
            .entry(normalize_repo_label(&label))
            .or_default()
            .insert(identity);
    }
    Ok(candidates
        .into_iter()
        .filter_map(|(label, identities)| {
            let mut values = identities.into_iter();
            let first = values.next()?;
            values.next().is_none().then_some((label, first))
        })
        .collect())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

#[cfg(test)]
#[path = "tests/repository_policy.rs"]
mod tests;
