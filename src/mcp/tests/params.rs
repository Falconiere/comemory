#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/mcp/params.rs`: the provenance inversion that keeps an
//! agent's inferred verdict out of the golden harvest, and the strictness the
//! `deny_unknown_fields` derive buys, and `project_show`'s `view` choice.
//!
//! The two `source` words are asserted against the real
//! `feedback_tracking::Source` parser and its stored provenance, so a rename
//! there fails here instead of silently filing every agent verdict as manual.

use comemory::domains::learning::feedback;
use comemory::domains::learning::feedback_tracking::Source;
use comemory::mcp::params::FeedbackParams;
use serde_json::json;

fn parse(value: serde_json::Value) -> serde_json::Result<FeedbackParams> {
    serde_json::from_value(value)
}

#[test]
fn an_unconfirmed_verdict_is_implicit() {
    let params = parse(json!({"query_id": "q-20260918-ab12cd34", "used": ["ab12cd34"]}))
        .expect("minimal params parse");
    assert!(!params.confirmed_by_user, "the default is unconfirmed");

    let request = feedback::Request::from(params);
    assert_eq!(request.source.as_deref(), Some("implicit"));
    assert_eq!(request.used, vec!["ab12cd34".to_string()]);
    assert!(request.irrelevant.is_empty());
    assert!(request.used_code.is_empty());
    assert!(request.irrelevant_code.is_empty());
}

#[test]
fn a_user_confirmed_verdict_is_explicit() {
    let params = parse(json!({
        "query_id": "q-20260918-ab12cd34",
        "irrelevant": ["cd34ab12"],
        "used_code": ["7"],
        "irrelevant_code": ["9"],
        "confirmed_by_user": true
    }))
    .expect("full params parse");

    let request = feedback::Request::from(params);
    assert_eq!(request.source.as_deref(), Some("explicit"));
    assert_eq!(request.irrelevant, vec!["cd34ab12".to_string()]);
    assert_eq!(request.used_code, vec!["7".to_string()]);
    assert_eq!(request.irrelevant_code, vec!["9".to_string()]);
}

#[test]
fn both_source_words_are_the_ones_the_core_parses() {
    // The mapping is only correct if the core accepts these exact words and
    // stores them as the two distinct provenances the harvest keys on.
    let implicit = Source::parse("implicit").expect("`implicit` is a source word");
    let explicit = Source::parse("explicit").expect("`explicit` is a source word");
    assert_eq!(implicit.provenance(), "implicit");
    assert_eq!(explicit.provenance(), "manual");
    assert_ne!(implicit.provenance(), explicit.provenance());
}

#[test]
fn an_unknown_field_is_rejected() {
    let err = parse(json!({
        "query_id": "q-20260918-ab12cd34",
        "provenance": "manual"
    }))
    .expect_err("deny_unknown_fields must reject an invented field");
    assert!(err.to_string().contains("unknown field"), "error: {err}");
}

#[test]
fn a_missing_query_id_is_rejected() {
    let err = parse(json!({"used": ["ab12cd34"]})).expect_err("query_id is required");
    assert!(err.to_string().contains("query_id"), "error: {err}");
}

#[test]
fn project_show_defaults_to_the_charter_and_carries_the_activity_page() {
    use comemory::mcp::params::{ProjectShowParams, ProjectShowView};
    let id = "11111111-1111-4111-8111-111111111111";
    let parse = |value| serde_json::from_value::<ProjectShowParams>(value);
    let bare = parse(json!({"id": id})).unwrap();
    bare.page_fields_fit_the_view().unwrap();
    assert_eq!(
        (
            bare.id.as_str(),
            bare.view,
            bare.limit,
            bare.cursor,
            bare.order
        ),
        (id, ProjectShowView::Charter, None, None, None)
    );
    let plan = parse(json!({"id": id, "view": "plan"})).unwrap();
    assert_eq!(plan.view, ProjectShowView::Plan);
    let page = parse(json!({
        "id": id, "view": "activity", "limit": 7, "cursor": "c", "order": "asc"
    }))
    .unwrap();
    page.page_fields_fit_the_view().unwrap();
    assert_eq!(
        (
            page.view,
            page.limit,
            page.cursor.as_deref(),
            page.order.as_deref()
        ),
        (ProjectShowView::Activity, Some(7), Some("c"), Some("asc"))
    );
    let e = parse(json!({"id": id, "view": "roadmap"})).unwrap_err();
    assert!(e.to_string().contains("unknown variant `roadmap`"), "{e}");
    let e = parse(json!({"id": id, "after": 0})).unwrap_err();
    assert!(e.to_string().contains("unknown field `after`"), "{e}");
}

#[test]
fn a_page_field_on_a_view_that_does_not_page_is_refused_naming_it() {
    use comemory::mcp::params::ProjectShowParams;
    use comemory::utilities::error_code::{Class, classify};
    let id = "11111111-1111-4111-8111-111111111111";
    for (view, field, value) in [
        ("charter", "limit", json!(5)),
        ("charter", "cursor", json!("c")),
        ("plan", "order", json!("asc")),
    ] {
        let params: ProjectShowParams =
            serde_json::from_value(json!({"id": id, "view": view, field: value})).unwrap();
        let e = params.page_fields_fit_the_view().unwrap_err();
        assert_eq!(classify(&e), ("invalid_request", Class::BadRequest), "{e}");
        assert_eq!(e.to_string(), format!("{field} is activity_only"));
    }
    // The default view is the charter, so a bare page field is refused too.
    let bare: ProjectShowParams = serde_json::from_value(json!({"id": id, "limit": 5})).unwrap();
    assert!(bare.page_fields_fit_the_view().is_err());
}
