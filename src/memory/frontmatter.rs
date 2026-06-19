//! YAML frontmatter struct plus split/render helpers for `memories/{id}-{slug}.md`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::memory::references::Ref;
use crate::prelude::*;

/// Memory taxonomy. Stored lowercase in YAML.
///
/// `clap::ValueEnum` is derived so the CLI can drive `--kind` through the
/// validated value-parser path; unknown values are rejected at parse time
/// with a usage hint listing every accepted variant.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
#[clap(rename_all = "lowercase")]
pub enum Kind {
    Decision,
    Bug,
    Convention,
    Discovery,
    Pattern,
    Note,
}

impl Kind {
    /// Canonical lowercase string used in YAML, SQL, and graph layers.
    /// Mirrors `#[serde(rename_all = "lowercase")]` so callers that need a
    /// plain `&str` (without going through serde) get the same wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Bug => "bug",
            Self::Convention => "convention",
            Self::Discovery => "discovery",
            Self::Pattern => "pattern",
            Self::Note => "note",
        }
    }

    /// Parse a lowercase kind string, falling back to `Note` for anything
    /// unknown. Centralises the match arms previously duplicated across the
    /// index and graph layers.
    pub fn parse_or_note(s: &str) -> Self {
        match s {
            "decision" => Self::Decision,
            "bug" => Self::Bug,
            "convention" => Self::Convention,
            "discovery" => Self::Discovery,
            "pattern" => Self::Pattern,
            _ => Self::Note,
        }
    }
}

/// External symbol / file references attached to a memory.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct References {
    #[serde(default)]
    pub symbols: Vec<Ref>,
    #[serde(default)]
    pub files: Vec<Ref>,
}

/// Cross-memory relationships used by the property graph.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relations {
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub conflicts_with: Vec<String>,
    #[serde(default)]
    pub derived_from: Vec<String>,
}

/// YAML frontmatter block at the top of every memory file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub id: String,
    pub kind: Kind,
    pub repo: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub author: String,
    #[serde(with = "iso8601_serde")]
    pub created: OffsetDateTime,
    pub quality: u8,
    pub schema: u32,
    pub content_hash: String,
    #[serde(default)]
    pub references: References,
    #[serde(default)]
    pub relations: Relations,
}

impl Frontmatter {
    /// Serialize to YAML (without the surrounding `---` fences).
    pub fn to_yaml(&self) -> Result<String> {
        Ok(serde_yaml::to_string(self)?)
    }

    /// Deserialize from YAML (without the surrounding `---` fences).
    pub fn from_yaml(s: &str) -> Result<Self> {
        Ok(serde_yaml::from_str(s)?)
    }

    /// Split a markdown file starting with `---\n…\n---\n` into frontmatter + body.
    pub fn split(raw: &str) -> Result<(Self, String)> {
        let stripped = raw
            .strip_prefix("---\n")
            .ok_or_else(|| Error::Other("missing leading '---'".into()))?;
        let end = stripped
            .find("\n---\n")
            .ok_or_else(|| Error::Other("missing closing '---'".into()))?;
        let yaml = &stripped[..end];
        let body = &stripped[end + 5..];
        let fm = Self::from_yaml(yaml)?;
        Ok((fm, body.to_string()))
    }

    /// Render frontmatter + body as a complete markdown file.
    pub fn render(&self, body: &str) -> Result<String> {
        let yaml = self.to_yaml()?;
        Ok(format!("---\n{}---\n{}", yaml, body))
    }
}

mod iso8601_serde {
    use super::*;
    use serde::Deserializer;
    use serde::Serializer;

    pub fn serialize<S: Serializer>(
        t: &OffsetDateTime,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let formatted = t
            .format(&Iso8601::DEFAULT)
            .map_err(serde::ser::Error::custom)?;
        s.serialize_str(&formatted)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<OffsetDateTime, D::Error> {
        let s: String = serde::Deserialize::deserialize(d)?;
        OffsetDateTime::parse(&s, &Iso8601::DEFAULT).map_err(serde::de::Error::custom)
    }
}
