#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Route-table smoke test for the sync resource.

use comemory::serve::routes::sync;

#[test]
fn table_entries_has_three_routes() {
    assert_eq!(sync::table_entries().len(), 3);
}
