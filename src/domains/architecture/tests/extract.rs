#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

use comemory::domains::architecture::extract::model_json;

#[test]
fn a_fenced_model_is_taken_out_of_the_surrounding_prose() {
    let out =
        "Here is the model you asked for:\n\n```json\n{\"schema\": 1}\n```\n\nHope that helps!";
    assert_eq!(model_json(out).trim(), "{\"schema\": 1}");
}

#[test]
fn an_unfenced_object_is_found_inside_prose() {
    let out = "Sure thing. {\"schema\": 1, \"components\": []} — done.";
    assert_eq!(model_json(out), "{\"schema\": 1, \"components\": []}");
}

#[test]
fn a_brace_inside_a_string_does_not_end_the_object() {
    let out = "{\"summary\": \"uses {braces} and \\\"quotes\\\"\", \"schema\": 1} trailing";
    assert_eq!(
        model_json(out),
        "{\"summary\": \"uses {braces} and \\\"quotes\\\"\", \"schema\": 1}"
    );
}

#[test]
fn output_with_no_json_falls_through_to_the_parser() {
    let out = "I could not read the repository.";
    assert_eq!(model_json(out), out);
    assert!(serde_json::from_str::<serde_json::Value>(model_json(out)).is_err());
}
