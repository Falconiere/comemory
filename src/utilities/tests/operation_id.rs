#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The operation-id shape: day-sortable, 128 bits of digest, distinct per
//! mutation even for the same entity in the same instant.

use std::collections::BTreeSet;

use crate::utilities::operation_id;

#[test]
fn an_operation_id_is_op_date_and_thirty_two_hex() {
    let id = operation_id::mint("memory", "a1b2c3d4", "upsert");
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 3, "{id}");
    assert_eq!(parts[0], "op");
    assert_eq!(parts[1].len(), 8);
    assert!(parts[1].bytes().all(|b| b.is_ascii_digit()));
    assert_eq!(
        parts[2].len(),
        32,
        "128 bits, not the 32 a day-shared space collides at"
    );
    assert!(
        parts[2]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
}

#[test]
fn ten_thousand_mints_of_one_entity_never_collide() {
    let ids: BTreeSet<String> = (0..10_000)
        .map(|_| operation_id::mint("memory", "a1b2c3d4", "upsert"))
        .collect();
    assert_eq!(ids.len(), 10_000);
}
