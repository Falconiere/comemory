//! Tests for the CSV id-list helpers extracted from `src/cli.rs` by #166.
//!
//! They had no colocated test while they lived on the root `cli` module; the
//! dedupe order and the two rejection messages are exactly what the extraction
//! had to preserve, so they are pinned here.

use comemory::utilities::id_list::{csv_unique, parse_id_csv, parse_symbol_id_csv};

#[test]
fn csv_unique_trims_drops_empties_and_keeps_first_mention_order() {
    assert_eq!(
        csv_unique("b,,a , b"),
        vec!["b".to_string(), "a".to_string()]
    );
}

#[test]
fn csv_unique_returns_nothing_for_an_empty_value() {
    assert!(csv_unique("").is_empty());
    assert!(csv_unique(" , ,").is_empty());
}

#[test]
fn parse_id_csv_accepts_eight_lowercase_hex_ids() {
    assert_eq!(
        parse_id_csv("0123abcd,0123abcd,89abcdef", "--supersedes").expect("valid ids"),
        vec!["0123abcd".to_string(), "89abcdef".to_string()]
    );
}

#[test]
fn parse_id_csv_names_the_flag_and_the_offending_id() {
    let err = parse_id_csv("0123abcd,NOPE", "--supersedes").expect_err("uppercase is not an id");
    assert_eq!(
        err.to_string(),
        "config: --supersedes: invalid memory id `NOPE` (expected 8 lowercase hex chars)"
    );
}

#[test]
fn parse_symbol_id_csv_dedupes_on_the_parsed_value() {
    assert_eq!(
        parse_symbol_id_csv("07,7,9", "--used-code").expect("valid symbol ids"),
        vec![7_i64, 9]
    );
}

#[test]
fn parse_symbol_id_csv_rejects_zero_negative_and_non_integers() {
    for bad in ["0", "-1", "x"] {
        let err = parse_symbol_id_csv(bad, "--used-code")
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            format!("config: --used-code: invalid symbol id `{bad}` (expected a positive integer)")
        );
    }
}
