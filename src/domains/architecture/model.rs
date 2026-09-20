//! The architecture model: the versioned `{ groups, components, edges }`
//! value an agent enriches, `architecture save` validates, and the console
//! renders. It is the only shape this domain persists — one JSON document per
//! repo, carried in the body of a memory tagged [`TAG`].
//!
//! Every field is deliberately plain data: no path is resolved, no rank is
//! recomputed, nothing here reads the store. [`validate`](super::validate)
//! owns every rule, so a `Model` that exists is not yet a `Model` that may be
//! saved.

use serde::{Deserialize, Serialize};

/// Schema version this build reads and writes. A model carrying any other
/// value is refused rather than guessed at.
pub const SCHEMA_VERSION: u32 = 1;

/// Tag every architecture memory carries. The console selects on it
/// (`GET /api/v1/memories?tag=architecture`), and so does `show` / `check`.
pub const TAG: &str = "architecture";

/// Hard ceiling on components in one model — past this a diagram stops being
/// an architecture picture and becomes the file graph again.
pub const MAX_COMPONENTS: usize = 200;

/// Hard ceiling on edges in one model.
pub const MAX_EDGES: usize = 2000;

/// Hard ceiling on the serialized model, in bytes. The model shares the
/// memory corpus, so it may not grow into a blob that dominates it.
pub const MAX_BYTES: usize = 32 * 1024;

/// Hard ceiling on one component's `summary`, in characters.
pub const MAX_SUMMARY: usize = 280;

/// Generate an `as_str` returning each variant's wire string. One definition
/// for the three enums that need one: three hand-written match arms over the
/// same shape are the near-duplicates `scripts/dup-check.sh` exists to catch,
/// and a JSON round trip plus a quote trim is the fragile alternative.
macro_rules! wire_strings {
    ($ty:ty { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $ty {
            /// The wire string this variant serializes as.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    };
}

/// How a model was produced. Recorded verbatim so a reader can tell a
/// deterministic scaffold from an agent's enrichment.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Emitted by `architecture scaffold` with no enrichment.
    Scaffold,
    /// Enriched by an agent (through the bundled skill or `architecture learn`).
    Agent,
    /// Hand-written.
    Manual,
}

/// Preferred layout direction, passed through to the renderer (Mermaid's
/// `flowchart <direction>`; the console may ignore it).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Left to right — the default.
    #[default]
    #[serde(rename = "LR")]
    Lr,
    /// Top to bottom.
    #[serde(rename = "TB")]
    Tb,
    /// Right to left.
    #[serde(rename = "RL")]
    Rl,
    /// Bottom to top.
    #[serde(rename = "BT")]
    Bt,
}

/// What a component *is*. Closed set: a renderer may map each kind onto a
/// shape without a fallback branch.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    /// A module or package inside this repository.
    Module,
    /// A deployable service.
    Service,
    /// An architectural layer spanning modules.
    Layer,
    /// A datastore, queue, or cache.
    Store,
    /// A dependency outside this repository — the one kind that owns no
    /// indexed files.
    External,
}

/// What a relation *means*. `imports` and `co_changed` mirror the mined code
/// graph; the rest are claims only a human or an agent can make.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Source imports destination (mined).
    Imports,
    /// Source and destination change together (mined).
    CoChanged,
    /// Source calls destination at runtime.
    Calls,
    /// Source depends on destination without calling it directly.
    Depends,
    /// Source reads state destination owns.
    Reads,
    /// Source writes state destination owns.
    Writes,
    /// Source publishes events destination consumes.
    Publishes,
}

wire_strings!(Source { Scaffold => "scaffold", Agent => "agent", Manual => "manual" });

wire_strings!(Direction { Lr => "LR", Tb => "TB", Rl => "RL", Bt => "BT" });

wire_strings!(EdgeKind {
    Imports => "imports",
    CoChanged => "co_changed",
    Calls => "calls",
    Depends => "depends",
    Reads => "reads",
    Writes => "writes",
    Publishes => "publishes",
});

/// A visual grouping of components (a Mermaid `subgraph`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Group {
    /// Stable id; referenced by [`Component::group`].
    pub id: String,
    /// Human label.
    pub name: String,
}

/// One node of the architecture: a named part of the system plus the
/// repo-relative paths it is made of.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// Stable id, renderer-safe (`^[A-Za-z][A-Za-z0-9_]{0,63}$`).
    pub id: String,
    /// Human label shown in the diagram.
    pub name: String,
    /// Owning [`Group::id`], when the component sits inside one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// What this component is.
    pub kind: ComponentKind,
    /// Repo-relative path prefixes this component is made of. Empty exactly
    /// when [`ComponentKind::External`].
    #[serde(default)]
    pub members: Vec<String>,
    /// One-line description, at most [`MAX_SUMMARY`] characters.
    #[serde(default)]
    pub summary: String,
    /// Summed PageRank of the member files, as scaffolded.
    #[serde(default)]
    pub rank: f64,
    /// Number of indexed files behind this component, as scaffolded.
    #[serde(default)]
    pub files: u32,
}

/// One directed relation between two components.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// Source [`Component::id`].
    pub from: String,
    /// Destination [`Component::id`].
    pub to: String,
    /// What the relation means.
    pub kind: EdgeKind,
    /// Relative strength — the number of contributing file-level edges for a
    /// mined kind, or the agent's own weighting.
    #[serde(default = "default_weight")]
    pub weight: i64,
}

fn default_weight() -> i64 {
    1
}

/// The whole model for one repo.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Model {
    /// Always [`SCHEMA_VERSION`] for a model this build accepts.
    pub schema: u32,
    /// Repo label this model describes (`index-code --repo`).
    pub repo: String,
    /// RFC 3339 timestamp of the scaffold or enrichment that produced it.
    pub generated_at: String,
    /// How it was produced.
    pub source: Source,
    /// Preferred layout direction.
    #[serde(default)]
    pub direction: Direction,
    /// Visual groupings.
    #[serde(default)]
    pub groups: Vec<Group>,
    /// The components themselves.
    pub components: Vec<Component>,
    /// The relations between them.
    #[serde(default)]
    pub edges: Vec<Edge>,
}
