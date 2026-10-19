//! Certified documents (DocMDP, ISO 32000-2 §12.8.2.2) and fields locked by a signature (FieldMDP,
//! §12.8.2.4): which signatures saving a document's unsaved edits over its file would invalidate.
//!
//! The unsaved edits are the copy-on-write overlay of the document's object graph, so they are
//! classified by comparing it with the file as last opened or saved, in the words the signature
//! validator reports later revisions in.

use std::collections::HashSet;

use pdfcraft_cos::{Document as Cos, ObjRef};
use pdfcraft_sign::{DigestCache, Modification, SignatureInfo, Status, TrustStore};

/// A signature the unsaved edits would invalidate if saved over the file, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The signature field.
    pub field: String,
    pub signer: Option<String>,
    /// `/M` as written.
    pub date: Option<String>,
    /// DocMDP permissions of a certification signature; `None` for an approval signature.
    pub certify: Option<u8>,
    pub breaches: Vec<Breach>,
}

/// One reason a [`Conflict`] breaks its signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Breach {
    /// A change the certification does not permit, in the words of [`pdfcraft_sign::kind`].
    Change(&'static str),
    /// A change to a form field the signature locks (its full name).
    LockedField(String),
}

impl Conflict {
    /// The changes the certification does not permit.
    pub fn changes(&self) -> Vec<&'static str> {
        self.breaches.iter().filter_map(|b| if let Breach::Change(k) = b { Some(*k) } else { None }).collect()
    }

    /// The locked fields that changed.
    pub fn locked_fields(&self) -> Vec<&str> {
        self.breaches.iter().filter_map(|b| if let Breach::LockedField(f) = b { Some(f.as_str()) } else { None }).collect()
    }
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let who = self.signer.as_deref().map(|s| format!(" by {s}")).unwrap_or_default();
        let what = if self.certify.is_some() { "certification signature" } else { "signature" };
        let mut why = Vec::new();
        if !self.changes().is_empty() {
            why.push(format!("it does not permit {}", self.changes().join(", ")));
        }
        if !self.locked_fields().is_empty() {
            why.push(format!("it locks {}", self.locked_fields().join(", ")));
        }
        write!(f, "the {what} {:?}{who} ({})", self.field, why.join("; "))
    }
}

/// Whether a signature still holds, so that a later change can break it: intact, and every later
/// revision is one it permits. A signature already broken is not protected any further.
fn holds(s: &SignatureInfo) -> bool {
    s.signed && !s.doc_timestamp && s.status != Status::Invalid && !matches!(s.modification, Modification::Disallowed(_))
}

/// The certification signature that permits no changes at all (DocMDP level 1), while it holds.
pub fn no_changes_certification(sigs: &[SignatureInfo]) -> Option<&SignatureInfo> {
    sigs.iter().find(|s| s.certify == Some(1) && holds(s))
}

/// The signatures that saving `cos`'s unsaved edits over its file would invalidate. `reopen` is
/// the password the file is opened with again.
pub(crate) fn conflicts(cos: &Cos, reopen: Option<&str>, trust: &TrustStore, cache: &DigestCache) -> Vec<Conflict> {
    if !cos.is_modified() {
        return Vec::new();
    }
    // It was opened (or saved and reopened) this way already; if that stops working, saving
    // fails its own check before anything is written.
    let Ok(saved) = Cos::open_with_password(cos.bytes().clone(), reopen) else { return Vec::new() };
    let sigs = pdfcraft_sign::pdf::list_cached(&saved, saved.bytes(), trust, cache);
    let guarded: Vec<&SignatureInfo> = sigs.iter().filter(|s| holds(s) && (s.certify.is_some() || s.field_lock.is_some())).collect();
    if guarded.is_empty() {
        return Vec::new();
    }
    let kinds = pdfcraft_sign::pdf::changes_between(cos, &saved);
    let changed = if guarded.iter().any(|s| s.field_lock.is_some()) { changed_fields(cos, &saved) } else { Vec::new() };
    guarded
        .into_iter()
        .filter_map(|s| {
            let mut breaches: Vec<Breach> = match s.certify {
                Some(p) => kinds.iter().filter(|k| !pdfcraft_sign::mdp::permits(Some(p), k)).map(|k| Breach::Change(k)).collect(),
                // An approval signature reports later changes in the Signatures panel; only its
                // field lock forbids any.
                None => Vec::new(),
            };
            if let Some(lock) = &s.field_lock {
                breaches.extend(changed.iter().filter(|f| lock.locks(f)).map(|f| Breach::LockedField(f.clone())));
            }
            (!breaches.is_empty()).then(|| Conflict {
                field: s.field.clone(),
                signer: s.signer.clone(),
                date: s.date.clone(),
                certify: s.certify,
                breaches,
            })
        })
        .collect()
}

/// Full names of the form fields `cos` adds, removes or changes (value, properties or widgets)
/// from `saved`. Signature fields are left out: signing them is what DocMDP governs.
fn changed_fields(cos: &Cos, saved: &Cos) -> Vec<String> {
    use pdfcraft_forms::FieldKind;
    let differs = |r: ObjRef| cos.is_edited(r.num) && *cos.get(r) != *saved.get(r);
    let now = pdfcraft_forms::fields(cos);
    let before = pdfcraft_forms::fields(saved);
    let before_names: HashSet<&str> = before.iter().map(|f| f.name.as_str()).collect();
    let now_names: HashSet<&str> = now.iter().map(|f| f.name.as_str()).collect();
    let mut out: Vec<String> = Vec::new();
    let added_or_edited = now
        .iter()
        .filter(|f| f.kind != FieldKind::Signature)
        .filter(|f| !before_names.contains(f.name.as_str()) || differs(f.obj) || f.widgets.iter().any(|w| differs(w.obj)));
    let removed = before.iter().filter(|f| f.kind != FieldKind::Signature && !now_names.contains(f.name.as_str()));
    for f in added_or_edited.chain(removed) {
        if !out.contains(&f.name) {
            out.push(f.name.clone());
        }
    }
    out
}
