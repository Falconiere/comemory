//! Wire-level types exchanged with the frontend.
//!
//! Every node id is namespaced (`m:`, `r:`, `a:`, `t:`, `f:`, `s:`) so the
//! frontend can route styling and the backend can resolve to the correct
//! kuzu table. Edge ids are opaque, deterministic 16-hex strings produced
//! by [`edge_id`].

use std::hash::Hasher;

use serde::{Deserialize, Serialize};
use siphasher::sip::SipHasher13;

/// One graph node in either direction (request payload or response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDto {
    /// Namespaced id, e.g. `m:a1b2c3d4`.
    pub id: String,
    /// Short human label (often the bare id without prefix).
    pub label: String,
    /// Discriminator matching kuzu node table: `Memory`, `Repo`, `Author`,
    /// `Tag`, `File`, `Symbol`.
    pub kind: String,
    /// Free-form per-kind properties.
    #[serde(default)]
    pub props: serde_json::Value,
}

/// One graph edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeDto {
    /// Opaque deterministic id from [`edge_id`].
    pub id: String,
    pub source: String,
    pub target: String,
    /// kuzu relation table name, e.g. `InRepo`, `Supersedes`.
    pub kind: String,
    #[serde(default)]
    pub props: serde_json::Value,
}

/// Response payload shared by `/api/seed` and `/api/expand`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPayload {
    pub nodes: Vec<NodeDto>,
    pub edges: Vec<EdgeDto>,
}

/// Compute a deterministic, opaque edge id from the triple.
///
/// Uses [`SipHasher13`] with a fixed all-zero key so the output is stable
/// across processes for the same inputs. The format is `e:<16-hex>`.
pub fn edge_id(source: &str, kind: &str, target: &str) -> String {
    let mut h = SipHasher13::new_with_keys(0, 0);
    h.write(source.as_bytes());
    h.write_u8(0);
    h.write(kind.as_bytes());
    h.write_u8(0);
    h.write(target.as_bytes());
    format!("e:{:016x}", h.finish())
}
