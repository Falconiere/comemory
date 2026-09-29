//! Who a project command acts as: the platform's two principal kinds plus an
//! id, stored as the `*_principal_type` / `*_principal_id` column pairs.
//!
//! Every core takes the actor as an argument. Until the capability envelope
//! (#315) wraps it, every adapter passes [`Principal::local_operator`]; the
//! transfer import (#342) remaps that id to the platform user on migrate.

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

impl Principal {
    /// A human principal with `id`.
    #[must_use]
    pub fn user(id: &str) -> Self {
        Self {
            principal_type: PrincipalType::User,
            id: id.to_string(),
        }
    }

    /// The person running this engine: a `user` with [`LOCAL_OPERATOR_ID`].
    #[must_use]
    pub fn local_operator() -> Self {
        Self::user(LOCAL_OPERATOR_ID)
    }
}
