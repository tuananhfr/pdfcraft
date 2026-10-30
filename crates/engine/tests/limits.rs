//! Session limits (an online document opened for viewing only, or with downloading turned off):
//! every change is refused at the engine's entry points, and lifting the limits needs no reopening.

use std::sync::Arc;

use pdfcraft_engine::ocr::{OcrPage, PlacedWord};
use pdfcraft_engine::sign::pkcs12;
use pdfcraft_engine::{DocId, Edit, EditError, FieldValue, NewAnnotation, NoteIcon, Session, SessionLimits, Shape, SignOptions, Style};

const VIEW_ONLY: SessionLimits = SessionLimits { view_only: true, no_extract: false };

/// One page with a text field, "Amount".
fn form() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >>",
        "<< /Length 44 >>\nstream\nBT /F1 14 Tf 20 250 Td (Contract text) Tj ET\nendstream",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Amount) /Rect [20 200 180 220] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>",
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

/// `form()` open with one edit to undo and one undone edit to redo.
fn edited() -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    let id = s.open("form.pdf", None, Arc::new(form()), None).unwrap();
    fill(&mut s, id, "1").unwrap();
    fill(&mut s, id, "2").unwrap();
    s.undo(id).unwrap();
    (s, id)
}

fn fill(s: &mut Session, id: DocId, text: &str) -> Result<(), EditError> {
    s.apply(id, Edit::SetFieldValue { name: "Amount".into(), value: FieldValue::Text(text.into()) })
}

fn comment() -> Edit {
    let shape = Shape::Note { at: [100.0, 120.0], icon: NoteIcon::Comment };
    Edit::AddAnnotation(NewAnnotation { page: 0, style: Style::default_for(&shape), shape, contents: "Looks fine".into(), author: "Reviewer".into() })
}

/// What a refused change must leave as it was.
fn state(s: &Session, id: DocId) -> (Arc<Vec<u8>>, u64, bool, Vec<String>, Vec<String>) {
    let d = s.get(id).unwrap();
    let labels = |h: Vec<pdfcraft_engine::HistoryEntry>| h.iter().map(|e| format!("{e:?}")).collect();
    (d.bytes.clone(), d.edit_generation(), d.dirty, labels(d.undo_history()), labels(d.redo_history()))
}

fn digital_id() -> pkcs12::DigitalId {
    let p12 = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../sign/tests/data/ec-p256.p12")).unwrap();
    pkcs12::open(&p12, "test").unwrap()
}

#[test]
fn view_only_refuses_every_change_and_keeps_the_document_and_its_history() {
    let (mut s, id) = edited();
    let before = state(&s, id);
    assert!(s.set_limits(id, VIEW_ONLY));
    let doc = s.get(id).unwrap();
    assert!(!doc.editable() && !doc.allows_annotation() && !doc.allows_form_filling() && !doc.allows_modification() && !doc.allows_assembly());
    assert!(!doc.allows_security_change() && !doc.can_unlock());
    assert!(doc.allows_copying() && doc.allows_printing(), "viewing, copying and printing stay");
    assert_eq!(doc.can_undo(), Some("Fill in Amount"), "the history is still there, only out of reach");

    assert_eq!(fill(&mut s, id, "3"), Err(EditError::ViewOnly));
    assert_eq!(s.apply(id, comment()), Err(EditError::ViewOnly));
    assert_eq!(s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 }), Err(EditError::ViewOnly));
    assert_eq!(s.apply(id, Edit::DeletePages { pages: vec![0] }), Err(EditError::ViewOnly));
    assert_eq!(s.undo(id), Err(EditError::ViewOnly));
    assert_eq!(s.redo(id), Err(EditError::ViewOnly));
    assert_eq!(s.revert(id), Err(EditError::ViewOnly));
    let word = PlacedWord { text: "John".into(), origin: [40.0, 40.0], across: [60.0, 0.0], up: [0.0, 20.0] };
    let ocr = [OcrPage { page: 0, words: vec![word], already_on_page: 0, skipped: None }];
    assert_eq!(s.apply_ocr(id, &ocr), Err(EditError::ViewOnly));
    s.set_javascript(true);
    let script = s.run_javascript(id, "this.getField('Amount').value = '9';", None);
    assert!(matches!(script, Err(EditError::ViewOnly)), "{script:?}");
    let signing = s.sign(id, &digital_id(), SignOptions { page: 0, rect: Some([20.0, 20.0, 140.0, 60.0]), ..SignOptions::default() });
    assert_eq!(signing.err(), Some(EditError::ViewOnly));
    // Save As bakes Fill & Sign first: nothing to bake, and nothing changes.
    assert_eq!(s.apply(id, Edit::FlattenFillSign), Ok(()));
    assert_eq!(state(&s, id), before, "nothing changed");

    // Lifted in place: the same document, its undo and redo working again.
    assert!(s.set_limits(id, SessionLimits::default()));
    assert!(s.get(id).unwrap().editable());
    s.redo(id).unwrap();
    s.undo(id).unwrap();
    s.undo(id).unwrap();
    assert_eq!(s.get(id).unwrap().can_undo(), None);
    fill(&mut s, id, "4").unwrap();
}

#[test]
fn no_extract_turns_off_copying_and_printing_but_not_editing() {
    let (mut s, id) = edited();
    assert!(s.set_limits(id, SessionLimits { view_only: false, no_extract: true }));
    let doc = s.get(id).unwrap();
    assert!(!doc.allows_copying() && !doc.allows_printing());
    assert_eq!(doc.check_copying(), Err(EditError::NoExtract));
    assert!(doc.editable());
    fill(&mut s, id, "5").unwrap();
    assert!(!s.set_limits(DocId(9_999), VIEW_ONLY), "a document that is not open");
}

#[test]
fn limits_stay_when_the_document_is_reopened_in_place() {
    let (mut s, id) = edited();
    s.set_limits(id, VIEW_ONLY);
    let bytes = s.get(id).unwrap().bytes.clone();
    s.reload(id, bytes).unwrap();
    assert_eq!(s.get(id).unwrap().limits(), VIEW_ONLY);
    assert_eq!(fill(&mut s, id, "6"), Err(EditError::ViewOnly));
}
