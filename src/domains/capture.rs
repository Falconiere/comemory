//! `domains::capture` — what a coding session leaves behind, and how much of
//! it may leave this machine.
//!
//! `claude_code` reads the tool's own transcript and `redact` decides what may
//! be sent, attesting the rule set it used. Only a session's shape and the
//! developer's explicit claims are sent, by the `run` and `distill` use cases.
//! [`hook`][h] owns the `SessionEnd` contract on both ends: the installed
//! command and the payload it is handed back. Credentials come from
//! `domains::sync`; the transcript body never leaves. Clap, stdin, rendering
//! and the exit code stay in `cli`; CLI-only, no `/api/v1` route.
//!
//! [h]: crate::domains::capture::hook

/// Candidate-batch wire types and the batch POST.
pub mod candidates;
/// Claude Code JSONL transcript adapter: session metadata and Bash lines.
pub mod claude_code;
/// Platform HTTP for capture consent rows and session receipts.
pub mod client;
/// The `comemory distill` use case: extract, redact, propose.
pub mod distill;
/// Recover explicit `comemory save` claims from a transcript's Bash lines.
pub mod explicit_save;
/// The Claude Code `SessionEnd` contract: installer and payload decoder.
pub mod hook;
/// Assemble the wire receipt: redact, digest, bound.
pub mod receipt;
/// The client redaction rule set and the attestation it produces.
pub mod redact;
/// The `comemory capture` use cases: consent rows and one session receipt.
pub mod run;

pub use candidates::{CandidateBatch, CandidateProposal, ProposeResponse};
pub use client::{CaptureSourceRow, PostSessionResponse, SessionReceipt};
pub use distill::{DistillReport, DistillRequest, run};
pub use explicit_save::{ExtractedCandidate, extract_explicit_saves};
pub use receipt::build_receipt;
pub use redact::{REDACTION_RULE_SET_VERSION, RedactionAttestation, redact_text};
pub use run::{CaptureReport, CaptureRequest, run_capture, run_sources};
