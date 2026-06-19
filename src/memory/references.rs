//! Versioned code reference: a pointer to a file or symbol plus an optional
//! captured anchor (git blob OID + commit + branch) recording the code state
//! at save time.
//!
//! A [`Ref`] serializes as a bare YAML/JSON string when it carries no anchor
//! (so legacy hand-written `references` and `search --json` output stay
//! byte-stable) and as a `{id, blob?, commit?, branch?}` map once an anchor is
//! pinned.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

/// A single code reference attached to a memory.
///
/// `id` is the qualified target `<repo>:<path>` (file) or
/// `<repo>:<path>:<symbol>` (symbol). The anchor fields are `Some` only when
/// the reference was pinned at save time against a tracked, committed file;
/// they are `None` for legacy/hand-written refs and unpinned (untracked or
/// cross-repo) targets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ref {
    /// Qualified target id: `<repo>:<path>[:<symbol>]`.
    pub id: String,
    /// Git blob OID of the file at save time (HEAD tree), if pinned.
    pub blob: Option<String>,
    /// HEAD commit SHA at save time, if pinned.
    pub commit: Option<String>,
    /// Branch shorthand at save time (advisory), if known.
    pub branch: Option<String>,
}

impl Ref {
    /// Construct an unanchored reference from a qualified id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            blob: None,
            commit: None,
            branch: None,
        }
    }

    /// True when no anchor was captured — the ref serializes as a bare string.
    pub(crate) fn is_bare(&self) -> bool {
        self.blob.is_none() && self.commit.is_none() && self.branch.is_none()
    }
}

impl Serialize for Ref {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        if self.is_bare() {
            return s.serialize_str(&self.id);
        }
        let len = 1 + [&self.blob, &self.commit, &self.branch]
            .iter()
            .filter(|o| o.is_some())
            .count();
        let mut m = s.serialize_map(Some(len))?;
        m.serialize_entry("id", &self.id)?;
        if let Some(b) = &self.blob {
            m.serialize_entry("blob", b)?;
        }
        if let Some(c) = &self.commit {
            m.serialize_entry("commit", c)?;
        }
        if let Some(br) = &self.branch {
            m.serialize_entry("branch", br)?;
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for Ref {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        d.deserialize_any(RefVisitor)
    }
}

/// Accepts either a bare string (legacy → id only) or a structured map.
struct RefVisitor;

impl<'de> Visitor<'de> for RefVisitor {
    type Value = Ref;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a ref string or a {id, blob?, commit?, branch?} map")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Ref, E> {
        Ok(Ref::new(v))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Ref, A::Error> {
        let mut r = Ref::default();
        let mut seen_id = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "id" => {
                    r.id = map.next_value()?;
                    seen_id = true;
                }
                "blob" => r.blob = map.next_value()?,
                "commit" => r.commit = map.next_value()?,
                "branch" => r.branch = map.next_value()?,
                _ => {
                    let _: de::IgnoredAny = map.next_value()?;
                }
            }
        }
        if !seen_id {
            return Err(de::Error::missing_field("id"));
        }
        Ok(r)
    }
}
