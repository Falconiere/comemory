//! An error's structured `details` object, serialized in insertion order.
//!
//! `serde_json`'s map sorts its keys (the crate does not enable
//! `preserve_order`), but the platform's project refusals emit theirs in
//! the order they were written — `proposal_stale` answers
//! `code, basePlanVersion, currentPlanVersion`. Keeping the pairs in a `Vec`
//! and serializing them as a map reproduces those bytes without changing the
//! key order of every other JSON document the binary writes.

use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::Value;

/// Key/value pairs that serialize as one JSON object, keys in push order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OrderedDetails(Vec<(&'static str, Value)>);

impl OrderedDetails {
    /// The platform's project shape: `code` first, then `pairs` in order.
    pub fn coded(code: &'static str, pairs: Vec<(&'static str, Value)>) -> Self {
        let mut all = Vec::with_capacity(pairs.len() + 1);
        all.push(("code", Value::from(code)));
        all.extend(pairs);
        Self(all)
    }

    /// Exactly `pairs`, in order, with no `code` member added.
    pub fn from_pairs(pairs: Vec<(&'static str, Value)>) -> Self {
        Self(pairs)
    }

    /// Append one more member after the existing ones.
    pub fn push(&mut self, key: &'static str, value: Value) {
        self.0.push((key, value));
    }
}

impl Serialize for OrderedDetails {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[cfg(test)]
#[path = "tests/ordered_details.rs"]
mod tests;
