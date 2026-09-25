//! Which protocol a session key speaks — a pure function of what is stored
//! for the key and what the upstream's replica manifest said.
//!
//! Selection only ever moves from `legacy` to `replica-v1`. An auth or
//! protocol failure never changes it: an upstream that refuses a credential or
//! answers garbage is not an old server, and treating it as one would send the
//! next writes over a protocol nobody chose.

/// A protocol a key can speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// The memory/code wire every platform serves today.
    Legacy,
    /// The journal protocol.
    Replica,
}

impl Protocol {
    /// The stored literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Replica => "replica-v1",
        }
    }

    /// Read a stored literal; `None` for a key never negotiated.
    #[must_use]
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw {
            Some("legacy") => Some(Self::Legacy),
            Some("replica-v1") => Some(Self::Replica),
            _ => None,
        }
    }
}

/// What the replica manifest request produced, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manifest {
    /// A valid `replica-v1` manifest advertising the protocol.
    Ready,
    /// A valid manifest with empty capabilities: the upstream is still seeding.
    Seeding,
    /// `404`: the upstream does not serve the replica routes.
    Missing,
    /// `401`/`403`, with the status the upstream answered.
    Unauthorized(u16),
    /// A `2xx` that is not a replica manifest, or `409 sync_upgrade_required`.
    Invalid(String),
    /// Timeout, `5xx`, `429`.
    Unavailable(String),
}

/// What the session does with the key this pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run under `protocol`; `coverage_reason` is set on a partial legacy
    /// selection; `upgraded` is true the pass `replica-v1` is first selected.
    Select {
        /// The protocol for this pass.
        protocol: Protocol,
        /// Why coverage is partial, for a legacy selection.
        coverage_reason: Option<&'static str>,
        /// Whether this pass moved the key from legacy (or nothing) to replica.
        upgraded: bool,
    },
    /// Suspend network work until the credential changes (the status the
    /// upstream refused it with).
    Suspend(u16),
    /// No network work this pass; nothing is downgraded.
    ProtocolError(String),
    /// Back off; nothing is downgraded.
    Unavailable(String),
}

/// Decide the protocol for one pass.
#[must_use]
pub fn decide(stored: Option<Protocol>, manifest: &Manifest) -> Decision {
    let on_replica = stored == Some(Protocol::Replica);
    match manifest {
        Manifest::Unauthorized(status) => Decision::Suspend(*status),
        Manifest::Invalid(why) => Decision::ProtocolError(why.clone()),
        Manifest::Unavailable(why) => Decision::Unavailable(why.clone()),
        Manifest::Ready => Decision::Select {
            protocol: Protocol::Replica,
            coverage_reason: None,
            upgraded: !on_replica,
        },
        Manifest::Seeding if on_replica => {
            Decision::Unavailable("the upstream is still seeding its journal".to_string())
        }
        Manifest::Missing if on_replica => Decision::ProtocolError(
            "the upstream no longer serves replica-v1; the key keeps its selection".to_string(),
        ),
        Manifest::Seeding => legacy("upstream_not_ready"),
        Manifest::Missing => legacy("replica_unsupported"),
    }
}

/// A partial legacy selection.
const fn legacy(reason: &'static str) -> Decision {
    Decision::Select {
        protocol: Protocol::Legacy,
        coverage_reason: Some(reason),
        upgraded: false,
    }
}

#[cfg(test)]
#[path = "tests/negotiate.rs"]
mod tests;
