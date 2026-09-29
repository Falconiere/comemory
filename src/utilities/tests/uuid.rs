#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Test mirror for `src/utilities/uuid.rs`.

use comemory::utilities::uuid::{canonical, new_v4};

#[test]
fn minted_ids_are_canonical_version_4_and_distinct() {
    let ids: Vec<String> = (0..64).map(|_| new_v4().unwrap()).collect();
    for id in &ids {
        assert_eq!(canonical(id).as_deref(), Some(id.as_str()), "{id}");
        assert_eq!(&id[14..15], "4", "{id}");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
    }
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn canonical_lowercases_and_refuses_other_shapes() {
    assert_eq!(
        canonical("0F8C2D7E-3B1A-4C5D-9E6F-7A8B9C0D1E2F").as_deref(),
        Some("0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f")
    );
    for bad in [
        "",
        "not-a-uuid",
        "0f8c2d7e3b1a4c5d9e6f7a8b9c0d1e2f",
        "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2",
        "0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2fa",
        "0f8c2d7g-3b1a-4c5d-9e6f-7a8b9c0d1e2f",
        "{0f8c2d7e-3b1a-4c5d-9e6f-7a8b9c0d1e2f}",
    ] {
        assert_eq!(canonical(bad), None, "{bad}");
    }
}
