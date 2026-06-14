//! Mirror tests for `src/output/page.rs`. Exercise the canonical paginator
//! [`Page::from_slice`] (window math, `--limit 0` = all, `has_more`/`total`
//! at boundaries, empty input) plus the [`Page::new`] passthrough constructor
//! and the JSON contract.

use comemory::output::page::Page;

fn nums(n: usize) -> Vec<usize> {
    (0..n).collect()
}

#[test]
fn from_slice_windows_middle_of_list() {
    let p = Page::from_slice(nums(10), 3, 4);
    assert_eq!(p.items, vec![4, 5, 6]);
    assert_eq!(p.limit, 3);
    assert_eq!(p.offset, 4);
    assert_eq!(p.total, Some(10));
    assert!(p.has_more, "offset 4 + 3 shown < 10 total");
}

#[test]
fn from_slice_last_window_has_no_more() {
    let p = Page::from_slice(nums(10), 4, 8);
    assert_eq!(p.items, vec![8, 9]);
    assert_eq!(p.total, Some(10));
    assert!(!p.has_more, "8 + 2 shown == 10 total, nothing beyond");
}

#[test]
fn from_slice_exact_boundary_has_no_more() {
    // limit lands exactly on the end: shown fills to total, no more pages.
    let p = Page::from_slice(nums(6), 3, 3);
    assert_eq!(p.items, vec![3, 4, 5]);
    assert!(!p.has_more);
}

#[test]
fn from_slice_first_page_of_many_has_more() {
    let p = Page::from_slice(nums(6), 3, 0);
    assert_eq!(p.items, vec![0, 1, 2]);
    assert!(p.has_more);
}

#[test]
fn from_slice_limit_zero_means_all() {
    let p = Page::from_slice(nums(5), 0, 0);
    assert_eq!(p.items, vec![0, 1, 2, 3, 4]);
    assert_eq!(p.limit, 0);
    assert_eq!(p.total, Some(5));
    assert!(!p.has_more, "limit 0 = all -> never has_more");
}

#[test]
fn from_slice_limit_zero_with_offset_returns_tail_all() {
    let p = Page::from_slice(nums(5), 0, 2);
    assert_eq!(p.items, vec![2, 3, 4]);
    assert!(
        !p.has_more,
        "limit 0 = all -> never has_more even with offset"
    );
}

#[test]
fn from_slice_offset_past_end_is_empty() {
    let p = Page::from_slice(nums(3), 5, 10);
    assert!(p.items.is_empty());
    assert_eq!(p.total, Some(3));
    assert!(!p.has_more);
}

#[test]
fn from_slice_offset_at_end_is_empty() {
    let p = Page::from_slice(nums(3), 2, 3);
    assert!(p.items.is_empty());
    assert!(!p.has_more);
}

#[test]
fn from_slice_empty_input() {
    let p = Page::from_slice(Vec::<usize>::new(), 10, 0);
    assert!(p.items.is_empty());
    assert_eq!(p.total, Some(0));
    assert!(!p.has_more);
}

#[test]
fn from_slice_empty_input_limit_zero() {
    let p = Page::from_slice(Vec::<usize>::new(), 0, 0);
    assert!(p.items.is_empty());
    assert_eq!(p.total, Some(0));
    assert!(!p.has_more);
}

#[test]
fn new_passes_through_metadata_unchanged() {
    let p = Page::new(vec![10, 20], 5, 7, None, true);
    assert_eq!(p.items, vec![10, 20]);
    assert_eq!(p.limit, 5);
    assert_eq!(p.offset, 7);
    assert_eq!(p.total, None);
    assert!(p.has_more);
}

#[test]
fn serializes_to_expected_json_shape() {
    let p = Page::from_slice(nums(5), 2, 1);
    let v = serde_json::to_value(&p).expect("serialize Page");
    assert_eq!(
        v,
        serde_json::json!({
            "items": [1, 2],
            "limit": 2,
            "offset": 1,
            "total": 5,
            "has_more": true,
        })
    );
}

#[test]
fn serializes_total_none_as_json_null() {
    let p: Page<usize> = Page::new(vec![], 0, 0, None, false);
    let v = serde_json::to_value(&p).expect("serialize Page");
    assert_eq!(v["total"], serde_json::Value::Null);
}
