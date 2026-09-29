//! Who a project command acts as: the platform's two principal kinds plus an
//! id, stored as the `*_principal_type` / `*_principal_id` column pairs.
//! The capability envelope (`authority`) wraps it; the transfer import (#342)
//! remaps the local operator's id to the platform user on migrate.

use serde::Serialize;

/// The platform's `principal_type` vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalType {
    /// A human: the local operator, or a workspace member on a hosted engine.
    User,
    /// An agent acting under a project grant.
    ProjectAgent,
}

impl PrincipalType {
    /// The stored column value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ProjectAgent => "project_agent",
        }
    }
}

/// One actor: a principal kind and its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Human or agent.
    pub principal_type: PrincipalType,
    /// The caller-supplied identifier; the engine has no user model.
    pub id: String,
}

/// The id the CLI's operator carries in a data directory with no platform
/// identity behind it.
pub const LOCAL_OPERATOR_ID: &str = "local-operator";

/// The id a local agent session (MCP, local-mode HTTP) carries: one stable
/// id, as a platform agent's key id is stable across its connections.
pub const LOCAL_AGENT_ID: &str = "local-agent";

impl Principal {
    /// A `principal_type` principal with `id`.
    #[must_use]
    pub fn new(principal_type: PrincipalType, id: &str) -> Self {
        Self {
            principal_type,
            id: id.to_string(),
        }
    }
}
