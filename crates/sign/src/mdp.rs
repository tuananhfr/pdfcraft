//! Modification detection and prevention (ISO 32000-2 §12.8.2.2 and §12.8.2.4): which later
//! changes a signature permits (DocMDP), and which form fields it locks (FieldMDP).

use pdfcraft_cos::{Dict, Document, Object};

use crate::pdf::kind;

/// Whether a change of `what` (a word of [`kind`]) made after a signature keeps it valid, under
/// DocMDP permissions `p` (1–3), or `None` for an approval signature.
///
/// Signatures, the Document Security Store and the `/Info` dictionary every save stamps are
/// permitted at every level; level 2 adds form fill, level 3 comments.
pub fn permits(p: Option<u8>, what: &str) -> bool {
    matches!(
        (what, p),
        (kind::SIGNATURE | kind::SECURITY_STORE | kind::METADATA, _) | (kind::FORM_FILL, Some(2 | 3) | None) | (kind::COMMENTS, Some(3) | None)
    )
}

/// The form fields a signature locks once it is signed (FieldMDP `/Action` and `/Fields`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldLock {
    /// Every field.
    All,
    /// Only these fields (fully qualified names).
    Include(Vec<String>),
    /// Every field except these.
    Exclude(Vec<String>),
}

impl FieldLock {
    /// Whether the field named `field` (fully qualified) is locked.
    pub fn locks(&self, field: &str) -> bool {
        // Listing a parent field covers its kids, whose full names extend it.
        let covered = |names: &[String]| names.iter().any(|n| field == n || field.strip_prefix(n.as_str()).is_some_and(|rest| rest.starts_with('.')));
        match self {
            FieldLock::All => true,
            FieldLock::Include(names) => covered(names),
            FieldLock::Exclude(names) => !covered(names),
        }
    }
}

/// The lock of a signed field: the FieldMDP transform in its signature's `/Reference`, else the
/// field's own `/Lock` dictionary (the instruction the signer was given, Table 236).
pub(crate) fn field_lock(doc: &Document, value: &Dict, field: &Dict) -> Option<FieldLock> {
    let references = value.get(b"Reference").map(|r| doc.resolve(r)).and_then(|r| r.as_array().cloned()).unwrap_or_default();
    let transform = references
        .iter()
        .filter_map(|r| doc.resolve(r).as_dict().cloned())
        .find(|r| r.name(b"TransformMethod") == Some(b"FieldMDP"))
        .and_then(|r| r.get(b"TransformParams").map(|p| doc.resolve(p)).and_then(|p| p.as_dict().cloned()));
    let params = transform.or_else(|| field.get(b"Lock").map(|l| doc.resolve(l)).and_then(|l| l.as_dict().cloned()))?;
    let names = || -> Vec<String> {
        let fields = params.get(b"Fields").map(|f| doc.resolve(f));
        let list = fields.as_deref().and_then(Object::as_array).cloned().unwrap_or_default();
        list.iter()
            .filter_map(|f| match &*doc.resolve(f) {
                Object::String(s) => Some(s.to_text().trim_matches('\0').trim().to_string()),
                _ => None,
            })
            .filter(|n| !n.is_empty())
            .collect()
    };
    match params.name(b"Action")? {
        b"All" => Some(FieldLock::All),
        b"Include" => Some(FieldLock::Include(names())),
        b"Exclude" => Some(FieldLock::Exclude(names())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_cos::PdfString;

    fn params(action: &str, fields: &[&str]) -> Dict {
        let mut p = Dict::new();
        p.set(b"Type".to_vec(), Object::name("TransformParams"));
        p.set(b"Action".to_vec(), Object::name(action));
        p.set(b"Fields".to_vec(), Object::Array(fields.iter().map(|f| Object::String(PdfString::text(f))).collect()));
        p
    }

    #[test]
    fn levels_permit_what_the_standard_lists() {
        for p in [Some(1), Some(2), Some(3), None] {
            assert!(permits(p, kind::METADATA) && permits(p, kind::SECURITY_STORE) && permits(p, kind::SIGNATURE));
            assert!(!permits(p, kind::PAGE_CONTENT) && !permits(p, kind::PAGES) && !permits(p, kind::OTHER));
        }
        assert!(!permits(Some(1), kind::FORM_FILL) && permits(Some(2), kind::FORM_FILL) && permits(None, kind::FORM_FILL));
        assert!(!permits(Some(2), kind::COMMENTS) && permits(Some(3), kind::COMMENTS) && permits(None, kind::COMMENTS));
    }

    #[test]
    fn a_lock_covers_listed_fields_and_their_kids() {
        let include = FieldLock::Include(vec!["Amount".into(), "Party".into()]);
        assert!(include.locks("Amount") && include.locks("Party.Name"));
        assert!(!include.locks("AmountDue") && !include.locks("Notes"));
        let exclude = FieldLock::Exclude(vec!["Notes".into()]);
        assert!(exclude.locks("Amount") && !exclude.locks("Notes") && !exclude.locks("Notes.Extra"));
        assert!(FieldLock::All.locks("anything"));
    }

    #[test]
    fn the_signatures_field_mdp_wins_over_the_fields_lock() {
        let doc = Document::new_empty();
        let mut reference = Dict::new();
        reference.set(b"TransformMethod".to_vec(), Object::name("FieldMDP"));
        reference.set(b"TransformParams".to_vec(), Object::Dict(params("Exclude", &["Notes"])));
        let mut value = Dict::new();
        value.set(b"Reference".to_vec(), Object::Array(vec![Object::Dict(reference)]));
        let mut field = Dict::new();
        field.set(b"Lock".to_vec(), Object::Dict(params("Include", &["Amount"])));
        assert_eq!(field_lock(&doc, &value, &field), Some(FieldLock::Exclude(vec!["Notes".into()])));
        // Without the transform, the field's lock is read.
        assert_eq!(field_lock(&doc, &Dict::new(), &field), Some(FieldLock::Include(vec!["Amount".into()])));
        // An unknown action locks nothing rather than guessing.
        let mut odd = Dict::new();
        odd.set(b"Lock".to_vec(), Object::Dict(params("Some", &[])));
        assert_eq!(field_lock(&doc, &Dict::new(), &odd), None);
        assert_eq!(field_lock(&doc, &Dict::new(), &Dict::new()), None);
    }
}
