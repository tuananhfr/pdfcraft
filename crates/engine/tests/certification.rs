//! Certified (DocMDP) and field-locking (FieldMDP) signatures: what unsaved edits may be saved
//! over a signed file, on fixtures certified and signed here with the fork's own signing code.

use std::sync::Arc;

use pdfcraft_engine::sign::{Modification, kind, pkcs12};
use pdfcraft_engine::{
    CertificationBreach, DocId, Edit, EditError, FieldValue, NewAnnotation, NoteIcon, Session, Shape, SignOptions, SignatureStatus, Style,
};

/// One page with two text fields, "Amount" and "Notes", and an empty signature field "Approval"
/// that locks `lock` once signed (a `/Lock` dictionary, Table 236).
fn form(lock: Option<&str>) -> Vec<u8> {
    let lock = lock.map(|l| format!(" /Lock << /Type /SigFieldLock {l} >>")).unwrap_or_default();
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 7 0 R 8 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R 7 0 R 8 0 R] >>"
            .into(),
        "<< /Length 44 >>\nstream\nBT /F1 14 Tf 20 250 Td (Contract text) Tj ET\nendstream".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Amount) /Rect [20 200 180 220] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Notes) /Rect [20 170 180 190] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>".into(),
        format!("<< /Type /Annot /Subtype /Widget /FT /Sig /T (Approval) /Rect [150 20 280 70] /P 3 0 R /F 4{lock} >>"),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn digital_id() -> pkcs12::DigitalId {
    let p12 = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../sign/tests/data/ec-p256.p12")).unwrap();
    pkcs12::open(&p12, "test").unwrap()
}

/// `form(lock)` signed in a session: certified at DocMDP level `certify`, or an approval signature
/// in the "Approval" field. The signed file is the document's saved state.
fn signed(certify: Option<u8>, lock: Option<&str>) -> (Session, DocId, Arc<Vec<u8>>) {
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    let id = s.open("contract.pdf", None, Arc::new(form(lock)), None).unwrap();
    let opts = SignOptions {
        page: 0,
        rect: Some([20.0, 20.0, 140.0, 60.0]),
        field: certify.is_none().then(|| "Approval".to_string()),
        certify,
        ..SignOptions::default()
    };
    let bytes = s.sign(id, &digital_id(), opts).unwrap();
    s.mark_signed(id, bytes.clone(), None).unwrap();
    (s, id, bytes)
}

fn fill(s: &mut Session, id: DocId, name: &str, text: &str) -> Result<(), EditError> {
    s.apply(id, Edit::SetFieldValue { name: name.into(), value: FieldValue::Text(text.into()) })
}

fn comment(s: &mut Session, id: DocId) -> Result<(), EditError> {
    let shape = Shape::Note { at: [100.0, 120.0], icon: NoteIcon::Comment };
    s.apply(
        id,
        Edit::AddAnnotation(NewAnnotation {
            page: 0,
            style: Style::default_for(&shape),
            shape,
            contents: "Looks fine".into(),
            author: "Reviewer".into(),
        }),
    )
}

fn rotate(s: &mut Session, id: DocId) -> Result<(), EditError> {
    s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 })
}

/// The document's signed signature after saving `s`'s edits incrementally and reopening the file.
fn saved_signature(s: &Session, id: DocId, signed: &Arc<Vec<u8>>) -> pdfcraft_engine::SignatureInfo {
    let saved = s.save_bytes(id).unwrap();
    assert!(saved.starts_with(signed.as_ref()), "an incremental update of the signed file");
    let mut again = Session::new();
    let reopened = again.open("saved.pdf", None, saved, None).unwrap();
    again.get(reopened).unwrap().signatures.iter().find(|x| x.signed).cloned().expect("still signed")
}

fn breaches(s: &Session, id: DocId) -> Vec<CertificationBreach> {
    s.certification_conflicts(id).into_iter().flat_map(|c| c.breaches).collect()
}

#[test]
fn level_two_allows_form_fill_and_not_comments() {
    let (mut s, id, bytes) = signed(Some(2), None);
    fill(&mut s, id, "Amount", "1 250 000").unwrap();
    assert_eq!(s.certification_conflicts(id), vec![], "filling in a form is permitted");
    let sig = saved_signature(&s, id, &bytes);
    assert_eq!((sig.certify, sig.status), (Some(2), SignatureStatus::Unknown), "intact, the signer just isn't trusted: {:?}", sig.details);
    assert_eq!(sig.modification, Modification::Allowed(vec![kind::FORM_FILL]));

    comment(&mut s, id).unwrap();
    let conflicts = s.certification_conflicts(id);
    assert_eq!(conflicts.len(), 1);
    assert_eq!((conflicts[0].certify, conflicts[0].field.as_str()), (Some(2), "Signature1"));
    assert!(conflicts[0].signer.is_some() && conflicts[0].date.is_some(), "the certifier is named");
    assert_eq!(conflicts[0].breaches, vec![CertificationBreach::Change(kind::COMMENTS)]);
    // Signing over the comment would carry it into the signed file.
    let refused = s.sign(id, &digital_id(), SignOptions { page: 0, rect: Some([160.0, 20.0, 280.0, 60.0]), ..SignOptions::default() });
    assert!(matches!(refused, Err(EditError::BreaksSignatures(ref why)) if why.contains("comments")), "{refused:?}");
    // Undone, nothing is left to break.
    s.undo(id).unwrap();
    assert_eq!(s.certification_conflicts(id), vec![]);
}

#[test]
fn level_three_allows_comments_and_not_page_changes() {
    let (mut s, id, bytes) = signed(Some(3), None);
    comment(&mut s, id).unwrap();
    assert_eq!(s.certification_conflicts(id), vec![]);
    let sig = saved_signature(&s, id, &bytes);
    assert_eq!(sig.status, SignatureStatus::Unknown, "{:?}", sig.details);
    assert_eq!(sig.modification, Modification::Allowed(vec![kind::COMMENTS]));

    rotate(&mut s, id).unwrap();
    assert_eq!(breaches(&s, id), vec![CertificationBreach::Change(kind::PAGE_CONTENT)]);
    // What the guard keeps from happening: saved anyway, the certification no longer holds.
    let broken = saved_signature(&s, id, &bytes);
    assert_eq!(broken.status, SignatureStatus::Invalid);
}

#[test]
fn level_one_refuses_every_edit_but_keeps_saving_as_it_is() {
    let (mut s, id, bytes) = signed(Some(1), None);
    let doc = s.get(id).unwrap();
    assert_eq!(doc.no_changes_certification().map(|c| c.field.as_str()), Some("Signature1"));
    assert!(!doc.allows_annotation() && !doc.allows_form_filling() && !doc.allows_modification() && !doc.allows_assembly());
    assert!(doc.allows_printing() && doc.allows_copying(), "viewing, printing and copying stay");
    assert_eq!(fill(&mut s, id, "Amount", "1"), Err(EditError::CertifiedNoChanges));
    assert_eq!(comment(&mut s, id), Err(EditError::CertifiedNoChanges));
    assert_eq!(rotate(&mut s, id), Err(EditError::CertifiedNoChanges));
    assert!(!s.get(id).unwrap().dirty);
    // Save and Save As write the certified file as it is; baking Fill & Sign is skipped.
    assert_eq!(s.apply(id, Edit::FlattenFillSign), Ok(()));
    assert_eq!(s.save_bytes(id).unwrap(), bytes);
    let signing = s.sign(id, &digital_id(), SignOptions { page: 0, rect: Some([160.0, 20.0, 280.0, 60.0]), ..SignOptions::default() });
    assert_eq!(signing, Err(EditError::CertifiedNoChanges));
}

#[test]
fn an_approval_signature_never_blocks_and_reports_changes_after_signing() {
    let (mut s, id, bytes) = signed(None, None);
    comment(&mut s, id).unwrap();
    rotate(&mut s, id).unwrap();
    assert_eq!(s.certification_conflicts(id), vec![], "no DocMDP, no field lock: nothing to refuse");
    s.undo(id).unwrap();
    let sig = saved_signature(&s, id, &bytes);
    assert_eq!(sig.modification, Modification::Allowed(vec![kind::COMMENTS]));
    assert!(
        sig.details
            .iter()
            .any(|d| d.message.contains("has been modified since this signature was applied, but the changes are permitted (comments)")),
        "the Signatures panel reports the change: {:?}",
        sig.details
    );
}

#[test]
fn a_field_lock_names_the_locked_field_that_changed() {
    let (mut s, id, bytes) = signed(None, Some("/Action /Include /Fields [(Amount)]"));
    let sig = s.get(id).unwrap().signatures.iter().find(|x| x.signed).cloned().unwrap();
    assert_eq!(sig.field_lock, Some(pdfcraft_engine::sign::FieldLock::Include(vec!["Amount".into()])));
    fill(&mut s, id, "Notes", "Delivered on site").unwrap();
    assert_eq!(s.certification_conflicts(id), vec![], "an unlocked field may change");
    assert_eq!(saved_signature(&s, id, &bytes).status, SignatureStatus::Unknown);
    fill(&mut s, id, "Amount", "990 000").unwrap();
    let conflicts = s.certification_conflicts(id);
    assert_eq!(conflicts.len(), 1);
    assert_eq!((conflicts[0].field.as_str(), conflicts[0].certify), ("Approval", None));
    assert_eq!(conflicts[0].breaches, vec![CertificationBreach::LockedField("Amount".into())]);
    assert!(conflicts[0].to_string().contains("locks Amount"), "{}", conflicts[0]);

    let (mut s, id, _) = signed(None, Some("/Action /All"));
    fill(&mut s, id, "Notes", "x").unwrap();
    assert_eq!(breaches(&s, id), vec![CertificationBreach::LockedField("Notes".into())]);
}
