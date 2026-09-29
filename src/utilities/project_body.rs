//! Turn a raw project request body into its core request, answering every
//! malformed input with the platform's schema-edge `invalid_request` (`400`)
//! and the field it concerns, as the platform's `invalidRequestFrom` does:
//! `"<field> is required"` for a missing field, `"<field> is invalid"` for
//! anything else, `"body must be an object"` when the body is not a JSON
//! object at all. Shared by the HTTP routes and the CLI, which parses
//! `project proposal submit --operations` through it.

use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_path_to_error::Segment;

use crate::prelude::*;
use crate::utilities::project_error::{ProjectError, RequestEdge};

/// `raw` as `T`, or the `400` naming the first field that failed.
pub fn body<T: DeserializeOwned>(raw: &[u8]) -> Result<T> {
    let value: Value = serde_json::from_slice(raw).map_err(|_| not_an_object())?;
    from_value(value)
}

/// A parsed body as `T` — for a caller that adds a field first, such as a
/// route's path id — refused as [`body`] refuses it.
pub fn from_value<T: DeserializeOwned>(value: Value) -> Result<T> {
    if !value.is_object() {
        return Err(not_an_object());
    }
    serde_path_to_error::deserialize(value).map_err(|e| {
        let path: Vec<String> = e.path().iter().filter_map(segment).collect();
        let message = e.inner().to_string();
        // A missing field is reported at its parent's path; every other
        // failure (wrong type, unknown field) at the field's own.
        let (field, reason) = match quoted(&message, "missing field `") {
            Some(name) => (joined(&path, name), "required"),
            None => (path.join("."), "invalid"),
        };
        ProjectError::invalid_field(&field, reason).into()
    })
}

/// The platform's refusal for a body that is not a JSON object.
fn not_an_object() -> Error {
    ProjectError::invalid_field("body", "invalid")
        .at(RequestEdge::Schema, Some("body must be an object"))
        .into()
}

/// One path segment as the platform names it: a key, or an array index.
fn segment(segment: &Segment) -> Option<String> {
    match segment {
        Segment::Seq { index } => Some(index.to_string()),
        Segment::Map { key } => Some(key.clone()),
        Segment::Enum { variant } => Some(variant.clone()),
        Segment::Unknown => None,
    }
}

/// The text between `prefix` and the next backtick in `message`.
fn quoted<'a>(message: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = message.strip_prefix(prefix)?;
    rest.split('`').next()
}

/// `name` under `path`, dot-joined.
fn joined(path: &[String], name: &str) -> String {
    let mut all = path.to_vec();
    all.push(name.to_string());
    all.join(".")
}

#[cfg(test)]
#[path = "tests/project_body.rs"]
mod tests;
