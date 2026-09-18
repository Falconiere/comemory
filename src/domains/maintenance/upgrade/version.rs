//! Release version parsing and ordering for `comemory upgrade`: the running
//! build's `CARGO_PKG_VERSION` against a release tag (`v0.19.0`). No
//! `semver` dependency — comemory only ever publishes `MAJOR.MINOR.PATCH`
//! tags with an optional pre-release suffix, and that is all this models.

use std::cmp::Ordering;
use std::fmt;

use crate::prelude::*;

/// A parsed release version. Ordered numerically on the triple; a
/// pre-release (`0.19.0-rc.1`) sorts BELOW its final release, and two
/// pre-releases of the same triple compare segment by segment (`rc.10`
/// above `rc.9`, numeric segments below alphabetic ones, as semver orders
/// them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// `MAJOR`.
    pub major: u64,
    /// `MINOR`.
    pub minor: u64,
    /// `PATCH`.
    pub patch: u64,
    /// The text after the first `-`, when present (`rc.1`).
    pub pre: Option<String>,
}

impl Version {
    /// Parse `0.19.0`, `v0.19.0`, or `0.19.0-rc.1`. A leading `v` is
    /// dropped. Anything else is a usage error naming the input.
    pub fn parse(raw: &str) -> Result<Self> {
        let bad = |why: &str| Error::Usage(format!("invalid version `{raw}`: {why}"));
        let s = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
        let (core, pre) = match s.split_once('-') {
            Some((_, "")) => return Err(bad("empty pre-release suffix")),
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (s, None),
        };
        let mut parts = core.split('.');
        let mut next = |what: &str| -> Result<u64> {
            parts
                .next()
                .and_then(|p| p.parse().ok())
                .ok_or_else(|| bad(&format!("missing or non-numeric {what}")))
        };
        let major = next("major")?;
        let minor = next("minor")?;
        let patch = next("patch")?;
        if parts.next().is_some() {
            return Err(bad("expected MAJOR.MINOR.PATCH"));
        }
        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// The git tag this version is released under (`v0.19.0`).
    pub fn tag(&self) -> String {
        format!("v{self}")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match &self.pre {
            Some(pre) => write!(f, "-{pre}"),
            None => Ok(()),
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => pre_cmp(a, b),
            })
    }
}

/// Semver pre-release ordering: dot-separated segments, numeric ones
/// compared as numbers and ranking below alphanumeric ones, a shorter
/// prefix ranking below its extension (`rc` < `rc.1`).
fn pre_cmp(left: &str, right: &str) -> Ordering {
    let mut lhs = left.split('.');
    let mut rhs = right.split('.');
    loop {
        let ord = match (lhs.next(), rhs.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(seg_l), Some(seg_r)) => segment_cmp(seg_l, seg_r),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
}

/// One pre-release segment against another: both numeric → by value; a
/// numeric segment ranks below an alphanumeric one; else by text.
fn segment_cmp(seg_l: &str, seg_r: &str) -> Ordering {
    match (seg_l.parse::<u64>(), seg_r.parse::<u64>()) {
        (Ok(num_l), Ok(num_r)) => num_l.cmp(&num_r),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => seg_l.cmp(seg_r),
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
#[path = "tests/version.rs"]
mod tests;
