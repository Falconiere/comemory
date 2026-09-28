//! The sysexits code `main.rs` exits with for a returned [`Error`] (design
//! §6.4): 64 `EX_USAGE` (`NotFound`, `Usage`, `Unsupported`); 65 `EX_DATAERR`
//! (`Yaml`, `Json`, `Toml`, `Frontmatter`, `VecDimMismatch`, `IdCollision`,
//! `Document`); 69 `EX_UNAVAILABLE` (`Unavailable`, `Embedder`); 70
//! `EX_SOFTWARE` (`Sqlite`, `Migration`, `SchemaTooNew`, `Ast`, `Git`,
//! `Forbidden`, `BadRequest`, `ConfirmationRequired`, `Cancelled`, `Other`);
//! 74 `EX_IOERR` (`Io`); 75 `EX_TEMPFAIL` (`IndexRunning`, `EpochMismatch`,
//! `Conflict`, `Busy`, `RestoreUnverified`); 78 `EX_CONFIG` (`Config`).
//!
//! A project refusal ([`Error::Project`]) takes its code from its
//! [`Class`] through [`exit_for_class`], so each project code reads the
//! same way as a CLI exit, an HTTP status and an MCP error. Lives in the
//! library, not `main.rs`, so a test can assert it.

use crate::prelude::*;
use crate::utilities::error_code::{Class, classify};

/// Map an [`Error`] to its sysexits-style exit code (see the module header).
pub fn exit_code(err: &Error) -> i32 {
    match err {
        Error::Io(_) => 74,
        Error::Config(_) => 78,
        Error::Unavailable(_) | Error::Embedder(_) => 69,
        // Retryable like a contended index run: the peer must re-read the
        // stream it was replaced with, not treat the refusal as fatal. A
        // contended `memory-save.lock` (`Error::Busy`) joins the same
        // bucket: the other holder will release it. So does a restored
        // engine refusing sync (`Error::RestoreUnverified`) until the
        // operator merges its erasure manifest.
        Error::IndexRunning { .. }
        | Error::EpochMismatch(_)
        | Error::Conflict(_)
        | Error::Busy(_)
        | Error::RestoreUnverified(_) => 75,
        Error::NotFound(_) | Error::Usage(_) | Error::Unsupported(_) => 64,
        Error::Yaml(_)
        | Error::Json(_)
        | Error::Toml(_)
        | Error::VecDimMismatch { .. }
        | Error::IdCollision { .. }
        | Error::Frontmatter(_)
        | Error::Document(_) => 65,
        Error::Sqlite(_)
        | Error::Ast(_)
        | Error::Git(_)
        | Error::Migration(_)
        | Error::SchemaTooNew(_)
        | Error::Forbidden(_)
        | Error::BadRequest(_)
        | Error::ConfirmationRequired(_)
        | Error::Cancelled
        | Error::Other(_) => 70,
        Error::Project(_) => exit_for_class(classify(err).1),
    }
}

/// The exit code for a [`Class`]. Each entry reuses the code an existing
/// variant of that reading already exits with — `NotFound`/`Usage` 64,
/// `VecDimMismatch` 65, `Unavailable` 69, `Forbidden` 70, `Conflict`/`Busy`
/// 75, `Unsupported` 64, `Other` 70 — so a project `forbidden` exits like the
/// crate's own `forbidden`. `Unauthorized` joins `Forbidden`.
pub fn exit_for_class(class: Class) -> i32 {
    match class {
        Class::NotFound | Class::BadRequest | Class::NotImplemented => 64,
        Class::Unprocessable => 65,
        Class::Unavailable => 69,
        Class::Unauthorized | Class::Forbidden | Class::Internal => 70,
        Class::Conflict | Class::Locked => 75,
    }
}

#[cfg(test)]
#[path = "tests/exit_code.rs"]
mod tests;
