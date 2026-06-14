//! Mirror tests for `src/cli/pagination.rs`. `PaginationArgs` is flatten-only,
//! so we parse it through the real `Cli` clap parser via the `ast` subcommand
//! (its first consumer) to pin the defaults (`--limit 50`, `--offset 0`) and
//! that the flags flow into the flattened struct.

use clap::Parser;
use comemory::cli::{Cli, Cmd};

fn parse_ast(extra: &[&str]) -> comemory::cli::ast::Args {
    let mut argv = vec!["comemory", "ast", "pat", "--lang", "rs", "--file", "x.rs"];
    argv.extend_from_slice(extra);
    match Cli::try_parse_from(argv).expect("parse ast args").cmd {
        Cmd::Ast(a) => a,
        other => panic!("expected Cmd::Ast, got {other:?}"),
    }
}

#[test]
fn defaults_are_limit_50_offset_0() {
    let a = parse_ast(&[]);
    assert_eq!(a.page.limit, 50);
    assert_eq!(a.page.offset, 0);
}

#[test]
fn limit_and_offset_flags_flow_into_struct() {
    let a = parse_ast(&["--limit", "7", "--offset", "3"]);
    assert_eq!(a.page.limit, 7);
    assert_eq!(a.page.offset, 3);
}

#[test]
fn limit_zero_is_accepted_as_all_sentinel() {
    let a = parse_ast(&["--limit", "0"]);
    assert_eq!(a.page.limit, 0);
}

#[test]
fn negative_limit_is_rejected_by_clap() {
    // usize-typed flag: a negative value is a parse error, not a panic.
    let res = Cli::try_parse_from([
        "comemory", "ast", "pat", "--lang", "rs", "--file", "x.rs", "--limit", "-1",
    ]);
    assert!(res.is_err(), "negative --limit must be a clap parse error");
}
