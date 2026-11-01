//! Co-editing by region between two sessions, with the room played by the test: payloads of
//! one side's changes applied on the other converge, touch nothing else, carry page operations
//! with their ids, survive a save and a reopen, and per-user undo never reverts the other side.

use std::collections::BTreeMap;
use std::sync::Arc;

use pdfcraft_engine::live::{Blocked, ChangeKind, Region};
use pdfcraft_engine::{AnnotId, ClientPrefix, DocId, Edit, EditError, FieldValue, NewAnnotation, NoteIcon, Session, Shape, Style};

/// Three pages. Page 1 has a note named `file-note-1` and the widget of the text field
/// `parent.child`; page 2 has a note without `/NM`.
fn fixture() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [9 0 R 12 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 6 0 R >> >> >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 7 0 R /Resources << /Font << /F1 6 0 R >> >> /Annots [8 0 R 10 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Annots [11 0 R 12 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 400] >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Length 39 >>\nstream\nBT /F1 14 Tf 20 250 Td (Page one) Tj ET\nendstream",
        "<< /Type /Annot /Subtype /Text /NM (file-note-1) /Rect [40 40 60 60] /P 3 0 R /Contents (From the file) /T (Ada) >>",
        "<< /T (parent) /Kids [10 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (child) /Parent 9 0 R /Rect [20 200 180 220] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>",
        "<< /Type /Annot /Subtype /Text /Rect [40 40 60 60] /P 4 0 R /Contents (No name) >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (other) /Rect [20 100 180 120] /P 4 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>",
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

fn open(bytes: &[u8], client: &str) -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    s.set_client_prefix(ClientPrefix::parse(client).unwrap());
    let id = s.open("live.pdf", None, Arc::new(bytes.to_vec()), None).unwrap();
    s.set_live(id, true).unwrap();
    (s, id)
}

fn pair() -> ((Session, DocId), (Session, DocId)) {
    let bytes = fixture();
    (open(&bytes, "0000aa"), open(&bytes, "0000bb"))
}

/// Send every change `from` made since the last call to `to`, as the room would.
fn relay(from: &mut (Session, DocId), to: &mut (Session, DocId)) -> Vec<String> {
    let mut sent = Vec::new();
    for change in from.0.take_live_changes(from.1) {
        for region in change.regions {
            let payload = from.0.live_payload(from.1, &region).unwrap();
            to.0.apply_remote(to.1, &region.to_string(), &payload).unwrap();
            sent.push(region.to_string());
        }
    }
    sent
}

/// Every comment's and field's payload, by region id.
fn regions(s: &mut Session, id: DocId) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut all: Vec<Region> = s.annotation_ids(id).into_iter().map(|a| Region::Annot(a.id)).collect();
    all.extend(s.get(id).unwrap().form.iter().map(|f| Region::Field(f.name.clone())));
    for r in all {
        let payload = s.live_payload(id, &r).unwrap();
        out.insert(r.to_string(), payload);
    }
    out
}

fn note_index(s: &Session, id: DocId, name: &str) -> (usize, usize) {
    s.find_annotation(id, &AnnotId::Name(name.into())).unwrap()
}

fn contents(s: &mut Session, id: DocId, name: &str) -> String {
    s.get(id).unwrap().info.annotations.iter().find(|a| a.name.as_deref() == Some(name)).and_then(|a| a.contents.clone()).unwrap_or_default()
}

fn set_contents(s: &mut Session, id: DocId, name: &str, text: &str) -> Result<(), EditError> {
    let (page, index) = note_index(s, id, name);
    s.apply(id, Edit::SetAnnotationContents { page, index, text: text.into() })
}

fn fill(s: &mut Session, id: DocId, field: &str, text: &str) -> Result<(), EditError> {
    s.apply(id, Edit::SetFieldValue { name: field.into(), value: FieldValue::Text(text.into()) })
}

fn value(s: &Session, id: DocId, field: &str) -> Vec<String> {
    s.get(id).unwrap().form.iter().find(|f| f.name == field).map(|f| f.value.clone()).unwrap_or_default()
}

fn note(page: usize) -> Edit {
    let shape = Shape::Note { at: [100.0, 120.0], icon: NoteIcon::Comment };
    Edit::AddAnnotation(NewAnnotation { page, style: Style::default_for(&shape), shape, contents: "New".into(), author: "Bea".into() })
}

#[test]
fn edits_name_their_regions() {
    let (mut a, _) = pair();
    set_contents(&mut a.0, a.1, "file-note-1", "Changed").unwrap();
    fill(&mut a.0, a.1, "parent.child", "42").unwrap();
    a.0.apply(a.1, Edit::RotatePages { pages: vec![1], degrees: 90 }).unwrap();
    a.0.apply(a.1, Edit::SetInfo { key: "Title".into(), value: "T".into() }).unwrap();
    let changes = a.0.take_live_changes(a.1);
    let ids: Vec<Vec<String>> = changes.iter().map(|c| c.regions.iter().map(Region::to_string).collect()).collect();
    assert_eq!(ids, [vec!["annot:nm:file-note-1".to_string()], vec!["field:parent.child".into()], vec!["pages".into()], vec!["*".into()]]);
    assert!(changes.iter().all(|c| c.kind == ChangeKind::Edit));
}

#[test]
fn concurrent_edits_of_different_regions_converge() {
    let (mut a, mut b) = pair();
    set_contents(&mut a.0, a.1, "file-note-1", "From A").unwrap();
    a.0.apply(a.1, note(2)).unwrap();
    fill(&mut b.0, b.1, "parent.child", "From B").unwrap();
    b.0.apply(b.1, note(0)).unwrap();
    relay(&mut a, &mut b);
    relay(&mut b, &mut a);
    assert_eq!(contents(&mut b.0, b.1, "file-note-1"), "From A");
    assert_eq!(value(&a.0, a.1, "parent.child"), ["From B"]);
    assert_eq!(regions(&mut a.0, a.1), regions(&mut b.0, b.1));
    assert_eq!(a.0.page_ids(a.1), b.0.page_ids(b.1));
    assert_eq!(a.0.annotation_ids(a.1).len(), 4);
}

#[test]
fn applying_a_region_leaves_every_other_region_byte_identical() {
    let (mut a, mut b) = pair();
    fill(&mut b.0, b.1, "other", "Kept").unwrap();
    b.0.take_live_changes(b.1);
    let before = regions(&mut b.0, b.1);
    set_contents(&mut a.0, a.1, "file-note-1", "Only this").unwrap();
    assert_eq!(relay(&mut a, &mut b), ["annot:nm:file-note-1"]);
    let after = regions(&mut b.0, b.1);
    for (region, payload) in &before {
        if region != "annot:nm:file-note-1" {
            assert_eq!(after.get(region), Some(payload), "{region} changed");
        }
    }
    assert_ne!(after.get("annot:nm:file-note-1"), before.get("annot:nm:file-note-1"));
    // Applied content reads back as sent.
    assert_eq!(after.get("annot:nm:file-note-1"), Some(&a.0.live_payload(a.1, &Region::parse("annot:nm:file-note-1").unwrap()).unwrap()));
    // Nothing applied remotely is the receiver's own change to send.
    assert!(b.0.take_live_changes(b.1).is_empty());
}

#[test]
fn a_region_someone_else_holds_refuses_the_edit_untouched() {
    let (_, mut b) = pair();
    let region = Region::parse("annot:nm:file-note-1").unwrap();
    b.0.set_live_blocked(b.1, Blocked { regions: [region.clone()].into(), ..Blocked::default() });
    let before = regions(&mut b.0, b.1);
    assert_eq!(set_contents(&mut b.0, b.1, "file-note-1", "Mine"), Err(EditError::RegionLocked(region.to_string())));
    assert_eq!(regions(&mut b.0, b.1), before);
    assert!(b.0.get(b.1).unwrap().can_undo().is_none());
    // Other regions stay editable; `*` waits until nobody holds anything.
    fill(&mut b.0, b.1, "parent.child", "Fine").unwrap();
    b.0.set_live_blocked(b.1, Blocked { any: true, ..Blocked::default() });
    assert_eq!(b.0.apply(b.1, Edit::SetInfo { key: "Title".into(), value: "T".into() }), Err(EditError::RegionLocked("*".into())));
}

#[test]
fn page_operations_apply_on_the_other_side_with_the_same_ids() {
    let (mut a, mut b) = pair();
    a.0.apply(a.1, Edit::InsertBlankPage { at: 1, width: 500.0, height: 700.0 }).unwrap();
    a.0.apply(a.1, Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
    a.0.apply(a.1, Edit::MovePages { pages: vec![3], to: 0 }).unwrap();
    a.0.apply(a.1, Edit::DeletePages { pages: vec![3] }).unwrap();
    relay(&mut a, &mut b);
    assert_eq!(a.0.page_ids(a.1), b.0.page_ids(b.1));
    let (pa, pb) = (&a.0.get(a.1).unwrap().info.pages, &b.0.get(b.1).unwrap().info.pages);
    assert_eq!(pa.len(), 3);
    for (x, y) in pa.iter().zip(pb) {
        assert_eq!((x.width, x.height, x.rotation), (y.width, y.height, y.rotation));
    }
    // And back: B duplicates the blank page and turns it.
    let blank = b.0.page_ids(b.1).iter().position(|p| p.to_string().starts_with("n0000aa")).unwrap();
    b.0.apply(b.1, Edit::DuplicatePages { pages: vec![blank] }).unwrap();
    b.0.apply(b.1, Edit::RotatePages { pages: vec![blank + 1], degrees: 180 }).unwrap();
    relay(&mut b, &mut a);
    assert_eq!(a.0.page_ids(a.1), b.0.page_ids(b.1));
    assert_eq!(a.0.get(a.1).unwrap().info.pages.len(), 4);
    assert_eq!(regions(&mut a.0, a.1), regions(&mut b.0, b.1));
}

#[test]
fn undo_skips_steps_someone_else_changed_since() {
    let (mut a, mut b) = pair();
    set_contents(&mut a.0, a.1, "file-note-1", "A1").unwrap();
    fill(&mut a.0, a.1, "parent.child", "A2").unwrap();
    relay(&mut a, &mut b);
    // B changes the note A edited first, then a comment of its own.
    set_contents(&mut b.0, b.1, "file-note-1", "B1").unwrap();
    b.0.apply(b.1, note(1)).unwrap();
    relay(&mut b, &mut a);
    // A's last step (the field) is undone alone: B's note and comment stay.
    a.0.undo(a.1).unwrap();
    assert_eq!(value(&a.0, a.1, "parent.child"), Vec::<String>::new());
    assert_eq!(contents(&mut a.0, a.1, "file-note-1"), "B1");
    assert_eq!(a.0.annotation_ids(a.1).len(), 3);
    // A's first step was on the note B changed since: skipped, never reverting B.
    assert_eq!(a.0.undo(a.1), Err(EditError::NothingToUndo));
    assert_eq!(a.0.take_live_skipped(a.1), 1);
    assert_eq!(contents(&mut a.0, a.1, "file-note-1"), "B1");
    // The undo travels like any change, and redo brings A's value back.
    relay(&mut a, &mut b);
    assert_eq!(value(&b.0, b.1, "parent.child"), Vec::<String>::new());
    a.0.redo(a.1).unwrap();
    relay(&mut a, &mut b);
    assert_eq!(value(&b.0, b.1, "parent.child"), ["A2"]);
    assert_eq!(regions(&mut a.0, a.1), regions(&mut b.0, b.1));
}

#[test]
fn undoing_a_page_delete_brings_the_page_back_everywhere() {
    let (mut a, mut b) = pair();
    a.0.apply(a.1, Edit::DeletePages { pages: vec![1] }).unwrap();
    relay(&mut a, &mut b);
    fill(&mut b.0, b.1, "parent.child", "B").unwrap();
    relay(&mut b, &mut a);
    a.0.undo(a.1).unwrap();
    assert_eq!(value(&a.0, a.1, "parent.child"), ["B"]);
    relay(&mut a, &mut b);
    assert_eq!(a.0.page_ids(a.1), b.0.page_ids(b.1));
    assert_eq!(b.0.get(b.1).unwrap().info.pages.len(), 3);
    assert_eq!(regions(&mut a.0, a.1), regions(&mut b.0, b.1));
}

#[test]
fn a_refused_change_is_taken_back_keeping_the_others() {
    let (mut a, mut b) = pair();
    set_contents(&mut a.0, a.1, "file-note-1", "Refused").unwrap();
    let change = a.0.take_live_changes(a.1).remove(0);
    fill(&mut b.0, b.1, "parent.child", "B").unwrap();
    relay(&mut b, &mut a);
    a.0.revert_live_change(a.1, &change).unwrap();
    assert_eq!(contents(&mut a.0, a.1, "file-note-1"), "From the file");
    assert_eq!(value(&a.0, a.1, "parent.child"), ["B"]);
    assert!(a.0.get(a.1).unwrap().can_undo().is_none());
    // Taken back at once, before anything else came in: the whole state goes back.
    fill(&mut a.0, a.1, "other", "x").unwrap();
    let change = a.0.take_live_changes(a.1).remove(0);
    a.0.revert_live_change(a.1, &change).unwrap();
    assert_eq!(value(&a.0, a.1, "other"), Vec::<String>::new());
}

#[test]
fn ids_survive_a_save_and_a_reopen_after_remote_changes() {
    let (mut a, mut b) = pair();
    a.0.apply(a.1, note(0)).unwrap();
    a.0.apply(a.1, Edit::InsertBlankPage { at: 3, width: 300.0, height: 300.0 }).unwrap();
    relay(&mut a, &mut b);
    // B leads: it saves, and a third person opens what it saved.
    let bytes = b.0.save_bytes(b.1).unwrap();
    b.0.mark_saved(b.1, bytes.clone(), None).unwrap();
    let mut c = open(&bytes, "0000cc");
    assert_eq!(c.0.page_ids(c.1), a.0.page_ids(a.1));
    assert_eq!(regions(&mut c.0, c.1), regions(&mut a.0, a.1));
    // Later updates from A still find their place on C and on B.
    a.0.apply(a.1, note(3)).unwrap();
    let changes = a.0.take_live_changes(a.1);
    for change in changes {
        for region in change.regions {
            let payload = a.0.live_payload(a.1, &region).unwrap();
            c.0.apply_remote(c.1, &region.to_string(), &payload).unwrap();
            b.0.apply_remote(b.1, &region.to_string(), &payload).unwrap();
        }
    }
    assert_eq!(regions(&mut c.0, c.1), regions(&mut a.0, a.1));
    assert_eq!(regions(&mut b.0, b.1), regions(&mut a.0, a.1));
}

#[test]
fn payloads_from_a_newer_version_or_garbage_are_refused() {
    let (_, mut b) = pair();
    for (region, payload) in [("annot:nm:x", r#"{"v":2}"#), ("annot:nm:x", "nope"), ("bogus", r#"{"v":1}"#), ("pages", r#"{"v":1,"order":[]}"#)] {
        assert!(b.0.apply_remote(b.1, region, payload).is_err(), "{region} {payload}");
    }
    assert!(b.0.take_live_changes(b.1).is_empty());
}
