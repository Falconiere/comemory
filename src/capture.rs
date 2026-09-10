//! Client-side session capture and distillation (platform Slices 3–4).
//!
//! Transcripts stay on the developer's machine; this module reads them,
//! redacts free text, and posts capture receipts / candidate batches to the
//! platform. Slice 4 ships first: explicit-save extraction + candidate POST
//! (`comemory distill`). Capture receipts and SessionEnd belong with #121.

pub mod candidates;
pub mod claude_code;
pub mod distill;
pub mod explicit_save;
pub mod redact;

pub use candidates::{CandidateBatch, CandidateProposal, ProposeResponse};
pub use distill::{DistillReport, DistillRequest, run};
pub use explicit_save::{ExtractedCandidate, extract_explicit_saves};
pub use redact::{REDACTION_RULE_SET_VERSION, RedactionAttestation, redact_text};
