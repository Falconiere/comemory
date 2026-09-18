//! `cli::setup::render` — the summary is a pure function of a `Response`, so
//! every footer and marker branch is exercised here by writing into a buffer.
//! The end-to-end shape against the real binary is snapshotted in
//! `tests/cli__setup.rs`.
use super::summary;
use crate::cli::setup::Mode;
use crate::domains::integrations::setup::{RepoContext, Response, Step, StepState};

/// Build a step with the given id and state.
fn step(id: &'static str, state: StepState) -> Step {
    Step {
        id,
        title: format!("Title for {id}"),
        detail: "detail".to_string(),
        state,
    }
}

/// Build a response around `steps`, counting the states the way
/// `domains::integrations::setup` does.
fn response(steps: Vec<Step>, repo: Option<RepoContext>) -> Response {
    let count =
        |matcher: fn(&StepState) -> bool| steps.iter().filter(|s| matcher(&s.state)).count();
    Response {
        data_dir: "/tmp/data".to_string(),
        repo,
        dry_run: false,
        satisfied: count(|s| matches!(s, StepState::Satisfied)),
        applied: count(|s| matches!(s, StepState::Applied { .. })),
        failed: count(|s| matches!(s, StepState::Failed { .. })),
        skipped: count(|s| matches!(s, StepState::Skipped)),
        steps,
    }
}

/// Render to a string with the ANSI styling stripped, so assertions can pin
/// whole lines rather than fragments that survive a spacing bug.
fn render(resp: &Response, mode: Mode) -> String {
    let mut out = Vec::new();
    summary(&mut out, resp, mode).unwrap();
    strip_ansi(&String::from_utf8(out).unwrap())
}

/// Remove ANSI SGR sequences; every escape this renderer emits ends in `m`.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        for next in chars.by_ref() {
            if next == 'm' {
                break;
            }
        }
    }
    out
}

/// The rendered lines, for whole-line assertions.
fn lines(resp: &Response, mode: Mode) -> Vec<String> {
    render(resp, mode).lines().map(str::to_string).collect()
}

#[test]
fn every_state_gets_its_own_marker_and_trailing_clause() {
    let resp = response(
        vec![
            step("a", StepState::Satisfied),
            step("b", StepState::Pending),
            step("c", StepState::Skipped),
            step(
                "d",
                StepState::Unavailable {
                    reason: "no git here".to_string(),
                },
            ),
            step(
                "e",
                StepState::Applied {
                    detail: "wrote three hooks".to_string(),
                },
            ),
            step(
                "f",
                StepState::Failed {
                    error: "permission denied".to_string(),
                },
            ),
        ],
        None,
    );

    let rendered = lines(&resp, Mode::Apply);

    // Whole lines: marker, title, and the trailing clause in one assertion,
    // so a spacing or ordering bug cannot slip past a substring match. A
    // state with its own note replaces the generic detail; the rest fall
    // back to it.
    for expected in [
        "  ✔ Title for a — detail",
        "  ◻ Title for b — detail",
        "  – Title for c — detail",
        "  ! Title for d — no git here",
        "  ✔ Title for e — wrote three hooks",
        "  ✘ Title for f — permission denied",
    ] {
        assert!(
            rendered.iter().any(|line| line == expected),
            "missing exact line {expected:?} in {rendered:#?}"
        );
    }
}

#[test]
fn a_step_with_no_note_and_no_detail_renders_without_a_dash() {
    let mut bare = step("a", StepState::Pending);
    bare.detail = String::new();
    let resp = response(vec![bare], None);

    let rendered = lines(&resp, Mode::Apply);

    assert!(
        rendered.iter().any(|line| line == "  ◻ Title for a"),
        "with no note and no detail the line ends after the title, with no \
         dangling dash and no trailing space: {rendered:#?}"
    );
}

#[test]
fn the_failure_footer_names_every_failed_id_and_outranks_the_done_line() {
    let resp = response(
        vec![
            step(
                "data-dir",
                StepState::Failed {
                    error: "read-only".to_string(),
                },
            ),
            step(
                "git-hooks",
                StepState::Applied {
                    detail: "ok".to_string(),
                },
            ),
        ],
        None,
    );

    let text = render(&resp, Mode::Apply);

    let rendered: Vec<String> = text.lines().map(str::to_string).collect();
    assert!(
        rendered
            .iter()
            .any(|line| line == "1 step(s) failed: data-dir"),
        "the footer names exactly the failed id, and not the applied one: \
         {rendered:#?}"
    );
    assert!(
        !text.contains("Done —"),
        "a failure outranks the success footer: {text}"
    );
}

#[test]
fn the_apply_footer_reports_what_ran_or_says_there_was_nothing_to_do() {
    let applied = response(
        vec![step(
            "git-hooks",
            StepState::Applied {
                detail: "ok".to_string(),
            },
        )],
        None,
    );
    assert!(render(&applied, Mode::Apply).contains("Done — 1 step(s) applied"));

    let settled = response(vec![step("git-hooks", StepState::Satisfied)], None);
    let text = render(&settled, Mode::Apply);
    assert!(text.contains("Nothing to do"), "{text}");
}

#[test]
fn the_plan_footer_counts_pending_steps_and_names_the_apply_command() {
    let pending = response(
        vec![
            step("a", StepState::Pending),
            step("b", StepState::Pending),
            step("c", StepState::Satisfied),
        ],
        None,
    );

    let text = render(&pending, Mode::Plan);

    assert!(text.contains("2 step(s) would run"), "{text}");
    assert!(text.contains("comemory setup --yes"), "{text}");
    assert!(text.contains("plan"), "the Plan header says so: {text}");
}

#[test]
fn a_plan_with_nothing_pending_says_so_instead_of_offering_yes() {
    let resp = response(
        vec![
            step("a", StepState::Satisfied),
            step(
                "b",
                StepState::Unavailable {
                    reason: "nope".to_string(),
                },
            ),
        ],
        None,
    );

    let text = render(&resp, Mode::Plan);

    assert!(text.contains("Nothing to do"), "{text}");
    assert!(
        !text.contains("--yes"),
        "there is nothing for --yes to apply: {text}"
    );
}

#[test]
fn the_location_line_names_the_repo_when_there_is_one() {
    let with_repo = response(
        vec![step("a", StepState::Satisfied)],
        Some(RepoContext {
            label: "comemory".to_string(),
            root: "/src/comemory".to_string(),
            is_git: true,
        }),
    );
    let text = render(&with_repo, Mode::Plan);
    assert!(text.contains("/tmp/data"), "names the data dir: {text}");
    assert!(text.contains("repo comemory"), "names the repo: {text}");

    let without = response(vec![step("a", StepState::Satisfied)], None);
    assert!(
        render(&without, Mode::Plan).contains("not in a git repository"),
        "says so when there is no repo"
    );
}
