#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use comemory::api::{Ctx, sync};
use comemory::config::{Config, Paths};
use comemory::store::connection;

#[test]
fn empty_changes_returns_zero_head() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("ensure_dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let resp = sync::changes::run(&mut ctx, 0, 50).expect("changes");

    assert!(resp.entries.is_empty());
    assert!(resp.next_seq.is_none());
    assert_eq!(resp.head_seq, 0);
}
