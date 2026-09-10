#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Reference extractor held to the platform's known answers over a real session.

use std::path::PathBuf;

use comemory::capture::claude_code::{bash_commands_from_jsonl, read_transcript_file};
use comemory::capture::explicit_save::extract_explicit_saves;

struct Expected {
    kind: &'static str,
    title: &'static str,
    tags: &'static [&'static str],
    body_length: usize,
}

const EXPECTED: &[Expected] = &[
    Expected {
        kind: "fact",
        title: "comemory.io feature gap map (2026-09-10)",
        tags: &["roadmap", "gaps", "slices", "sync", "console", "marketing"],
        body_length: 2_097,
    },
    Expected {
        kind: "fact",
        title: "Feature-gap issues filed on GitHub (#45-#59)",
        tags: &["issues", "roadmap", "gaps", "github"],
        body_length: 593,
    },
    Expected {
        kind: "fact",
        title: "Engine sync routes landed in 0.21.0; author absent from HTTP API",
        tags: &["engine", "sync", "pin", "author", "blocked"],
        body_length: 1_032,
    },
    Expected {
        kind: "fact",
        title: "comemory.io repo needs bun installed; local console suite fails on macOS+Node 26",
        tags: &["bun", "setup", "tests", "macos", "node", "flake"],
        body_length: 1_186,
    },
    Expected {
        kind: "fact",
        title: "Console tests pass locally with --localstorage-file; docs ingest impossible from a Worker",
        tags: &[
            "console",
            "tests",
            "node",
            "docs-ingest",
            "engine",
            "worker",
            "migration",
        ],
        body_length: 1_289,
    },
    Expected {
        kind: "pattern",
        title: "Gate every merge on threads+label+checks, not just the bot label",
        tags: &["process", "review", "merge", "github", "gate"],
        body_length: 1_111,
    },
];

fn fixture_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude-code-session-saves.jsonl");
    read_transcript_file(&path).expect("fixture readable")
}

fn extracted() -> Vec<comemory::capture::ExtractedCandidate> {
    let commands = bash_commands_from_jsonl(&fixture_text());
    extract_explicit_saves(&commands)
}

#[test]
fn recovers_exactly_the_six_real_saves_in_order() {
    let got = extracted();
    assert_eq!(got.len(), EXPECTED.len());
    assert_eq!(
        got.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
        EXPECTED.iter().map(|e| e.title).collect::<Vec<_>>()
    );
}

#[test]
fn classifies_each_into_the_product_kind() {
    let got = extracted();
    for (i, candidate) in got.iter().enumerate() {
        assert_eq!(candidate.kind, EXPECTED[i].kind);
    }
}

#[test]
fn recovers_exact_tag_lists() {
    let got = extracted();
    for (i, candidate) in got.iter().enumerate() {
        assert_eq!(
            candidate.tags,
            EXPECTED[i]
                .tags
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn recovers_body_lengths_inside_the_cap() {
    let got = extracted();
    for (i, candidate) in got.iter().enumerate() {
        let body = candidate.body.as_ref().expect("body present");
        assert_eq!(body.chars().count(), EXPECTED[i].body_length);
        assert!(body.chars().count() <= 4_000);
    }
}

#[test]
fn keeps_newlines_in_five_of_six_bodies() {
    let got = extracted();
    let multiline = got
        .iter()
        .filter(|c| c.body.as_deref().is_some_and(|b| b.contains('\n')))
        .count();
    assert_eq!(multiline, 5);
}

#[test]
fn unescapes_shell_quotes_in_bodies() {
    let got = extracted();
    let with_quotes: Vec<_> = got
        .iter()
        .filter(|c| c.body.as_deref().is_some_and(|b| b.contains('"')))
        .collect();
    assert!(!with_quotes.is_empty());
    for candidate in with_quotes {
        assert!(!candidate.body.as_deref().unwrap().contains("\\\""));
    }
}

#[test]
fn records_monotonic_saved_at_timestamps() {
    let got = extracted();
    // Fixture timestamps are RFC 3339 with a fixed offset, so lexical order
    // matches chronological order.
    let timestamps: Vec<&str> = got.iter().map(|c| c.saved_at.as_str()).collect();
    assert!(timestamps.iter().all(|t| !t.is_empty()));
    let mut sorted = timestamps.clone();
    sorted.sort_unstable();
    assert_eq!(timestamps, sorted);
}
