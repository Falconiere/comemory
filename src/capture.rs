//! Client-side session capture and distillation (platform Slices 3–4).
//!
//! Transcripts stay on the developer's machine; this module reads them,
//! redacts free text, and posts capture receipts / candidate batches to the
//! platform. Slice 3: session receipts + SessionEnd (`comemory capture`).
//! Slice 4: explicit-save extraction + candidate POST (`comemory distill`).
//!
//! CLI-only (`serve::routes::meta::CLI_ONLY`).

pub mod candidates;
pub mod claude_code;
pub mod client;
pub mod distill;
pub mod explicit_save;
pub mod hook;
pub mod receipt;
pub mod redact;
pub mod run;

pub use candidates::{CandidateBatch, CandidateProposal, ProposeResponse};
pub use client::{CaptureSourceRow, PostSessionResponse, SessionReceipt};
pub use distill::{DistillReport, DistillRequest, run};
pub use explicit_save::{ExtractedCandidate, extract_explicit_saves};
pub use receipt::build_receipt;
pub use redact::{REDACTION_RULE_SET_VERSION, RedactionAttestation, redact_text};
pub use run::{CaptureReport, CaptureRequest, run_capture, run_sources};
