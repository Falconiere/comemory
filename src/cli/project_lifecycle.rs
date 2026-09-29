//! `comemory project archive|restore|pause|resume <ID>` (#328): the flags
//! the four lifecycle verbs share, a thin shell over
//! `domains::projects::lifecycle` that `project.rs` runs under the local
//! operator's envelope and renders as the project view. Clap argument ids
//! equal the core body's serde names.

use clap::Args as ClapArgs;

use crate::domains::projects::lifecycle::{Body, Kind, Request};
use crate::prelude::*;
use crate::utilities::uuid;

/// Args for `project archive|restore|pause|resume`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// The project's UUID.
    pub id: String,
    /// The `version` you last read (`project show`); a stale one is refused
    /// with `version_conflict`.
    #[arg(long = "expected-version", id = "expectedVersion")]
    pub expected_version: i64,
    /// Why, up to 4000 characters. Required and non-blank for `pause`.
    #[arg(long)]
    pub reason: Option<String>,
    /// Retry key (1–200 UTF-16 units): rerunning with the same key and flags
    /// prints the first answer and writes nothing. A fresh one is minted when
    /// omitted, so only a run that names its key is retry-safe.
    #[arg(long = "idempotency-key", id = "idempotencyKey")]
    pub idempotency_key: Option<String>,
}

impl Args {
    /// The core request these flags name for `kind`, its key minted when
    /// absent.
    pub fn request(self, kind: Kind) -> Result<Request> {
        let idempotency_key = match self.idempotency_key {
            Some(key) => key,
            None => uuid::new_v4()?,
        };
        Ok(Request {
            kind,
            id: self.id,
            body: Body {
                workspace_id: None,
                idempotency_key,
                expected_version: self.expected_version,
                reason: self.reason,
            },
        })
    }
}
