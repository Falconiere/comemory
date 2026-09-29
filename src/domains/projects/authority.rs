//! Who may run which project verb (#315), ported with its messages from the
//! platform's `project-principal.ts`, `project-workspace-role.ts` and
//! `rpc/projects/README.md`. Every core is a sealed [`Command`] whose
//! `execute` needs an [`Actor`], and only [`run`] mints one — after the
//! [`Envelope`] admits the verb, so a refusal comes before any store access.
//! The actor is never `Origin.actor`, a self-declared telemetry label.

use crate::domains::projects::principal::{
    LOCAL_AGENT_ID, LOCAL_OPERATOR_ID, Principal, PrincipalType,
};
use crate::prelude::*;
use crate::utilities::context::Ctx;
use crate::utilities::project_error::ProjectError;

/// One agent-reachable verb family a project-agent grant may carry: the
/// platform's `PROJECT_AGENT_CAPABILITIES`. No human-only verb has a name
/// here, so no envelope can spell one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Read the charter, plan, work items, proposals, evidence and activity.
    ProjectRead,
    /// Submit an immutable plan or scope proposal — never review one.
    ProposalCreate,
    /// Request a version-pinned work packet with cited engine context.
    WorkPacketCreate,
    /// Start, heartbeat, block, resume and request review on an execution.
    ExecutionUpdate,
    /// Attach typed evidence and retry its verification.
    EvidenceCreate,
    /// Report project health with a rationale and evidence ids.
    HealthUpdate,
}

impl Capability {
    /// Every capability, in the platform's command order.
    pub const ALL: [Self; 6] = [
        Self::ProjectRead,
        Self::ProposalCreate,
        Self::WorkPacketCreate,
        Self::ExecutionUpdate,
        Self::EvidenceCreate,
        Self::HealthUpdate,
    ];

    /// The platform's wire name (`project.read`, …).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectRead => "project.read",
            Self::ProposalCreate => "proposal.create",
            Self::WorkPacketCreate => "work_packet.create",
            Self::ExecutionUpdate => "execution.update",
            Self::EvidenceCreate => "evidence.create",
            Self::HealthUpdate => "health.update",
        }
    }

    /// The capability `value` names exactly, if any.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == value)
    }

    /// This capability's bit in [`Capabilities`].
    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// The capabilities an agent envelope holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities(u8);

impl Capabilities {
    /// All six: the local agent's default.
    #[must_use]
    pub fn all() -> Self {
        Self(
            Capability::ALL
                .into_iter()
                .fold(0, |bits, c| bits | c.bit()),
        )
    }

    /// Nothing granted.
    #[must_use]
    pub fn none() -> Self {
        Self(0)
    }

    /// Whether `capability` is held.
    #[must_use]
    pub fn contains(self, capability: Capability) -> bool {
        self.0 & capability.bit() != 0
    }

    /// Parse a capability list as a whole, as the platform parses a grant's
    /// stored column: one unrecognised value degrades the entire list to
    /// [`Capabilities::none`] — the safe direction for an authority value.
    /// The warning names `source`, the value's position and the list length,
    /// never the value itself.
    pub fn parse<S: AsRef<str>>(source: &str, values: &[S]) -> Self {
        let mut set = Self::none();
        for (position, value) in values.iter().enumerate() {
            let Some(capability) = Capability::parse(value.as_ref()) else {
                tracing::warn!(
                    source,
                    position,
                    count = values.len(),
                    "project envelope: unrecognised capability; the list grants nothing"
                );
                return Self::none();
            };
            set.0 |= capability.bit();
        }
        set
    }
}

/// A human's authority over a project, as the platform's middleware resolves
/// it. Ordered: each tier holds every verb of the tiers below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// A workspace member: create, read, propose, work, evidence and
    /// work-item completion.
    Member,
    /// The project lead or a workspace admin: adds the lifecycle verbs,
    /// proposal review, health, and project completion and cancellation.
    Lead,
    /// A workspace owner or admin: adds hard deletion.
    Owner,
}

/// What an envelope's principal may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// A `user` principal at a tier.
    Human(Tier),
    /// A `project_agent` principal holding capabilities.
    Agent(Capabilities),
}

/// Who a command runs as and what it may reach. The principal kind follows
/// from the constructor, so a human never carries capabilities and an agent
/// never carries a tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    principal: Principal,
    reach: Reach,
}

impl Envelope {
    /// A human principal `id` at `tier`.
    #[must_use]
    pub fn user(id: &str, tier: Tier) -> Self {
        Self {
            principal: Principal::new(PrincipalType::User, id),
            reach: Reach::Human(tier),
        }
    }

    /// An agent principal `id` holding `capabilities`.
    #[must_use]
    pub fn agent(id: &str, capabilities: Capabilities) -> Self {
        Self {
            principal: Principal::new(PrincipalType::ProjectAgent, id),
            reach: Reach::Agent(capabilities),
        }
    }

    /// The CLI's envelope: the person running this engine, a `user` at
    /// [`Tier::Owner`] — there is no one else to ask.
    #[must_use]
    pub fn local_operator() -> Self {
        Self::user(LOCAL_OPERATOR_ID, Tier::Owner)
    }

    /// The MCP and local-mode HTTP envelope: a `project_agent` holding every
    /// capability and, by principal kind, no human verb.
    #[must_use]
    pub fn local_agent() -> Self {
        Self::agent(LOCAL_AGENT_ID, Capabilities::all())
    }

    /// The principal this envelope acts as.
    #[must_use]
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// `Ok` when this envelope admits `verb`, else the refusal: `403
    /// project_agent_scope` for an agent (a human-only verb, then a missing
    /// capability) and `403 forbidden` for a human below the verb's tier.
    /// Pure: never touches the store.
    pub fn authorize(&self, verb: Verb) -> Result<()> {
        let refusal = match (self.reach, verb.rule()) {
            (Reach::Human(held), rule) if held >= rule.tier() => return Ok(()),
            (Reach::Human(_), _) => ProjectError::TierForbidden {
                reason: verb.below_tier().to_string(),
            },
            (Reach::Agent(_), Rule::HumanOnly(_)) => ProjectError::ProjectAgentScope {
                reason: "This command requires a signed-in human".to_string(),
            },
            (Reach::Agent(held), Rule::Shared(_, needed)) if held.contains(needed) => {
                return Ok(());
            }
            (Reach::Agent(_), Rule::Shared(_, needed)) => ProjectError::ProjectAgentScope {
                reason: format!(
                    "This grant does not carry the {} capability",
                    needed.as_str()
                ),
            },
        };
        Err(refusal.into())
    }
}

/// Who may run a verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// A human at the tier or above, or an agent holding the capability.
    Shared(Tier, Capability),
    /// A human at the tier or above; an agent never, by principal kind.
    HumanOnly(Tier),
}

impl Rule {
    /// The human requirement.
    fn tier(self) -> Tier {
        match self {
            Self::Shared(tier, _) | Self::HumanOnly(tier) => tier,
        }
    }
}

/// Every project verb, one per platform route command. A later task names
/// its verb here instead of re-deciding who may run it; grant management
/// has no verb, because grants stay on the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// `GET /projects/{id}`.
    ProjectShow,
    /// `GET /projects` — an agent reader here, unlike the platform's
    /// session-only route, because the MCP budget makes `project_list` one.
    ProjectList,
    /// `GET /projects/{id}/plan`.
    PlanRead,
    /// `GET /projects/{id}/proposals[/{proposalId}]`.
    ProposalRead,
    /// `GET /projects/{id}/work-items[/{workItemId}]`.
    WorkItemRead,
    /// `GET /projects/{id}/evidence`.
    EvidenceRead,
    /// `GET /projects/{id}/activity`.
    ActivityRead,
    /// `POST /projects/{id}/proposals`.
    ProposalCreate,
    /// `POST …/work-items/{workItemId}/work-packet`.
    WorkPacketCreate,
    /// `POST …/work-items/{workItemId}/{ready,start}`.
    WorkItemTransition,
    /// `POST …/executions/{executionId}/{heartbeat,block,resume,request-review}`.
    ExecutionUpdate,
    /// `POST …/evidence`, `…/executions/{executionId}/evidence`,
    /// `…/evidence/{evidenceId}/verify`.
    EvidenceCreate,
    /// `POST /projects/{id}/health`.
    HealthUpdate,
    /// `POST /projects`.
    ProjectCreate,
    /// `GET /project-approvals`, the cross-project pending inbox.
    ApprovalInbox,
    /// `GET /project-activity`, the cross-project activity log.
    WorkspaceActivity,
    /// `POST …/work-items/{workItemId}/complete`.
    WorkItemComplete,
    /// `POST /projects/{id}/archive`.
    Archive,
    /// `POST /projects/{id}/restore`.
    Restore,
    /// `POST /projects/{id}/pause`.
    Pause,
    /// `POST /projects/{id}/resume`.
    Resume,
    /// `POST …/proposals/{proposalId}/approve`.
    ProposalApprove,
    /// `POST …/proposals/{proposalId}/request-changes`.
    ProposalRequestChanges,
    /// `POST …/proposals/{proposalId}/reject`.
    ProposalReject,
    /// `POST /projects/{id}/complete`.
    ProjectComplete,
    /// `POST /projects/{id}/cancel`.
    ProjectCancel,
    /// `DELETE /projects/{id}`.
    ProjectDelete,
}

impl Verb {
    /// Every verb, readers first, then the shared writers, then the
    /// human-only verbs by tier.
    pub const ALL: [Self; 27] = [
        Self::ProjectShow,
        Self::ProjectList,
        Self::PlanRead,
        Self::ProposalRead,
        Self::WorkItemRead,
        Self::EvidenceRead,
        Self::ActivityRead,
        Self::ProposalCreate,
        Self::WorkPacketCreate,
        Self::WorkItemTransition,
        Self::ExecutionUpdate,
        Self::EvidenceCreate,
        Self::HealthUpdate,
        Self::ProjectCreate,
        Self::ApprovalInbox,
        Self::WorkspaceActivity,
        Self::WorkItemComplete,
        Self::Archive,
        Self::Restore,
        Self::Pause,
        Self::Resume,
        Self::ProposalApprove,
        Self::ProposalRequestChanges,
        Self::ProposalReject,
        Self::ProjectComplete,
        Self::ProjectCancel,
        Self::ProjectDelete,
    ];

    /// The ported per-verb table.
    fn rule(self) -> Rule {
        use Capability as C;
        match self {
            Self::ProjectShow
            | Self::ProjectList
            | Self::PlanRead
            | Self::ProposalRead
            | Self::WorkItemRead
            | Self::EvidenceRead
            | Self::ActivityRead => Rule::Shared(Tier::Member, C::ProjectRead),
            Self::ProposalCreate => Rule::Shared(Tier::Member, C::ProposalCreate),
            Self::WorkPacketCreate => Rule::Shared(Tier::Member, C::WorkPacketCreate),
            Self::WorkItemTransition | Self::ExecutionUpdate => {
                Rule::Shared(Tier::Member, C::ExecutionUpdate)
            }
            Self::EvidenceCreate => Rule::Shared(Tier::Member, C::EvidenceCreate),
            Self::HealthUpdate => Rule::Shared(Tier::Lead, C::HealthUpdate),
            Self::ProjectCreate
            | Self::ApprovalInbox
            | Self::WorkspaceActivity
            | Self::WorkItemComplete => Rule::HumanOnly(Tier::Member),
            Self::Archive
            | Self::Restore
            | Self::Pause
            | Self::Resume
            | Self::ProposalApprove
            | Self::ProposalRequestChanges
            | Self::ProposalReject
            | Self::ProjectComplete
            | Self::ProjectCancel => Rule::HumanOnly(Tier::Lead),
            Self::ProjectDelete => Rule::HumanOnly(Tier::Owner),
        }
    }

    /// The platform's sentence for a human below this verb's tier. Every
    /// human is at least a member, so only lead and owner verbs reach it.
    fn below_tier(self) -> &'static str {
        match self {
            Self::ProposalApprove | Self::ProposalRequestChanges | Self::ProposalReject => {
                "Only the project lead or a workspace admin may review a proposal"
            }
            Self::HealthUpdate => "Only the project lead or a workspace admin may report health",
            Self::ProjectDelete => "Only a workspace owner or admin can delete a project",
            _ => "Only the project lead or a workspace admin may run this command",
        }
    }
}

/// The principal an admitted command runs as, and records on every row and
/// project activity event it writes. Only [`run`] mints one, after the
/// envelope admits the command's verb.
#[derive(Debug)]
pub struct Actor {
    principal: Principal,
}

impl Actor {
    /// The admitted principal.
    #[must_use]
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
}

/// The seal on [`Command`]: only a type inside `domains::projects` can
/// implement it, so no adapter can wrap one core in another verb.
pub(in crate::domains::projects) mod sealed {
    /// Implemented by every project command's request type.
    pub trait Sealed {}
}

/// A project core: a request type that names its verb and executes as an
/// admitted [`Actor`]. Run it with [`run`]; nothing else can supply the actor.
pub trait Command: sealed::Sealed {
    /// What the core returns.
    type Response;

    /// The verb the envelope must admit.
    fn verb(&self) -> Verb;

    /// Run the core as `actor`.
    fn execute(self, ctx: &mut Ctx<'_>, actor: &Actor) -> Result<Self::Response>;
}

/// Run `command` under `envelope`. A refusal returns before `execute` — so
/// before any store access: a lazy [`Ctx`] never opens its database, and no
/// project row, project activity event or `activity_log` row is written.
pub fn run<C: Command>(ctx: &mut Ctx<'_>, envelope: &Envelope, command: C) -> Result<C::Response> {
    envelope.authorize(command.verb())?;
    let actor = Actor {
        principal: envelope.principal.clone(),
    };
    command.execute(ctx, &actor)
}

#[cfg(test)]
#[path = "tests/authority.rs"]
mod tests;
