#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::identity`].

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::identity::{
    BinaryIdentity, SOCKET_ID_LEN, UNIT_ID_LEN, canonical_data_dir, data_dir_id,
};

#[test]
fn a_symlinked_data_dir_resolves_to_the_same_canonical_dir_and_id() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let a = canonical_data_dir(&Paths::new(real)).unwrap();
    let b = canonical_data_dir(&Paths::new(link)).unwrap();
    assert_eq!(a, b);
    assert_eq!(data_dir_id(&a, UNIT_ID_LEN), data_dir_id(&b, UNIT_ID_LEN));
    assert_eq!(data_dir_id(&a, UNIT_ID_LEN).len(), UNIT_ID_LEN);
    assert_eq!(data_dir_id(&a, SOCKET_ID_LEN).len(), SOCKET_ID_LEN);
    assert!(data_dir_id(&a, SOCKET_ID_LEN).starts_with(&data_dir_id(&a, UNIT_ID_LEN)));
}

#[test]
fn two_directories_get_two_ids() {
    let root = tempfile::tempdir().unwrap();
    let (one, two) = (root.path().join("one"), root.path().join("two"));
    std::fs::create_dir(&one).unwrap();
    std::fs::create_dir(&two).unwrap();
    assert_ne!(
        data_dir_id(&one, UNIT_ID_LEN),
        data_dir_id(&two, UNIT_ID_LEN)
    );
}

#[test]
fn a_missing_data_dir_is_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let err = canonical_data_dir(&Paths::new(root.path().join("nope"))).unwrap_err();
    assert!(err.to_string().contains("cannot resolve"), "{err}");
}

#[test]
fn the_current_binary_is_this_build() {
    let me = BinaryIdentity::current().unwrap();
    assert_eq!(me.version, env!("CARGO_PKG_VERSION"));
    assert!(me.path.is_absolute() && me.path.exists());
}
