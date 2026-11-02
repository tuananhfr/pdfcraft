//! Region ids of co-editing (`erpHost` v3): the unit one person locks and sends at a time.
//!
//! - `annot:<comment id>`: one comment with its pop-up (`annot:nm:<name>`, `annot:obj:<num>.<gen>`);
//! - `field:<full name>`: one form field's value and its widgets' appearance;
//! - `pages`: the order lock (insert, delete, reorder, rotate pages);
//! - `*`: the whole document, for every other edit.

use std::fmt;

use crate::stable_ids::AnnotId;

/// The live room takes region ids of 1 to 256 bytes without control characters.
pub const MAX_REGION_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    Annot(AnnotId),
    Field(String),
    Pages,
    Document,
}

impl Region {
    /// The id back from its [`Display`](fmt::Display) form; `None` for anything else.
    pub fn parse(s: &str) -> Option<Self> {
        if !is_valid(s) {
            return None;
        }
        match s {
            "*" => Some(Self::Document),
            "pages" => Some(Self::Pages),
            _ => {
                if let Some(id) = s.strip_prefix("annot:") {
                    AnnotId::parse(id).map(Self::Annot)
                } else {
                    s.strip_prefix("field:").filter(|n| !n.is_empty()).map(|n| Self::Field(n.to_owned()))
                }
            }
        }
    }

    /// Whether the room can carry this id; a comment or field whose id can't be sent is edited
    /// under `*` instead.
    pub fn is_sendable(&self) -> bool {
        is_valid(&self.to_string())
    }
}

fn is_valid(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_REGION_BYTES && !s.chars().any(char::is_control)
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Annot(id) => write!(f, "annot:{id}"),
            Self::Field(name) => write!(f, "field:{name}"),
            Self::Pages => f.write_str("pages"),
            Self::Document => f.write_str("*"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for s in ["*", "pages", "annot:nm:0a1b2c00-0001-4abc-8def-0123456789ab", "annot:obj:12.0", "field:person.name"] {
            assert_eq!(Region::parse(s).map(|r| r.to_string()).as_deref(), Some(s));
        }
        for s in [
            "",
            "field:",
            "annot:",
            "annot:x",
            "page",
            "annot:obj:1",
            "annot:obj:012.0",
            "annot:obj:12.00",
            "field:a\u{1}b",
            &format!("field:{}", "x".repeat(300)),
        ] {
            assert_eq!(Region::parse(s), None, "{s:?}");
        }
    }
}
