//! Truth table for [`comemory::retrieval::code_ref_status::classify`].
//!
//! Covers every branch of the resolution order for both file and symbol refs:
//! unpinned, repo-off-disk -> unknown, and per-kind fresh/stale/ghost/unknown.

use comemory::retrieval::code_ref_status::{CurrentRef, RefStatus, classify};

/// Build a [`CurrentRef`] from its three observable inputs.
fn cur<'a>(
    head_blob: Option<&'a str>,
    repo_on_disk: bool,
    symbol_present: Option<bool>,
) -> CurrentRef<'a> {
    CurrentRef {
        head_blob,
        repo_on_disk,
        symbol_present,
    }
}

#[test]
fn as_str_round_trips_every_variant() {
    assert_eq!(RefStatus::Fresh.as_str(), "fresh");
    assert_eq!(RefStatus::Stale.as_str(), "stale");
    assert_eq!(RefStatus::Ghost.as_str(), "ghost");
    assert_eq!(RefStatus::Unpinned.as_str(), "unpinned");
    assert_eq!(RefStatus::Unknown.as_str(), "unknown");
}

#[test]
fn unpinned_when_no_anchor_regardless_of_kind_or_current_state() {
    // pinned_blob None short-circuits before any current-state inspection.
    let c = cur(Some("abc"), true, Some(true));
    assert_eq!(classify(None, &c, false), RefStatus::Unpinned);
    assert_eq!(classify(None, &c, true), RefStatus::Unpinned);
    // Even with a hostile current state, no anchor -> Unpinned.
    let gone = cur(None, false, Some(false));
    assert_eq!(classify(None, &gone, true), RefStatus::Unpinned);
}

#[test]
fn unknown_when_repo_not_on_disk_for_pinned_ref() {
    let c = cur(Some("abc"), false, Some(true));
    assert_eq!(classify(Some("abc"), &c, false), RefStatus::Unknown);
    assert_eq!(classify(Some("abc"), &c, true), RefStatus::Unknown);
}

#[test]
fn file_ref_fresh_when_head_blob_matches_pin() {
    let c = cur(Some("abc"), true, None);
    assert_eq!(classify(Some("abc"), &c, false), RefStatus::Fresh);
}

#[test]
fn file_ref_stale_when_head_blob_differs() {
    let c = cur(Some("def"), true, None);
    assert_eq!(classify(Some("abc"), &c, false), RefStatus::Stale);
}

#[test]
fn file_ref_ghost_when_head_blob_absent() {
    let c = cur(None, true, None);
    assert_eq!(classify(Some("abc"), &c, false), RefStatus::Ghost);
}

#[test]
fn file_ref_is_index_independent_symbol_present_ignored() {
    // symbol_present must not change a file-ref verdict.
    for sp in [None, Some(true), Some(false)] {
        let c = cur(Some("abc"), true, sp);
        assert_eq!(classify(Some("abc"), &c, false), RefStatus::Fresh);
    }
}

#[test]
fn symbol_ref_ghost_when_file_gone_from_head() {
    // head_blob None means the file vanished — ghost before consulting the index.
    for sp in [None, Some(true), Some(false)] {
        let c = cur(None, true, sp);
        assert_eq!(classify(Some("abc"), &c, true), RefStatus::Ghost);
    }
}

#[test]
fn symbol_ref_ghost_when_symbol_absent_from_current_index() {
    let c = cur(Some("abc"), true, Some(false));
    assert_eq!(classify(Some("abc"), &c, true), RefStatus::Ghost);
}

#[test]
fn symbol_ref_unknown_when_no_current_index() {
    // File present but no index to decide ghost-ness -> Unknown (not a false ghost).
    let c = cur(Some("abc"), true, None);
    assert_eq!(classify(Some("abc"), &c, true), RefStatus::Unknown);
}

#[test]
fn symbol_ref_fresh_when_present_and_blob_matches() {
    let c = cur(Some("abc"), true, Some(true));
    assert_eq!(classify(Some("abc"), &c, true), RefStatus::Fresh);
}

#[test]
fn symbol_ref_stale_when_present_but_blob_differs() {
    let c = cur(Some("def"), true, Some(true));
    assert_eq!(classify(Some("abc"), &c, true), RefStatus::Stale);
}
