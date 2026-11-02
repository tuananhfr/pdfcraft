//! Stable ids for co-editing: new comments' `/NM` carry the session's client prefix, ids in the
//! file are never rewritten, pages keep a session id through insert, delete, reorder, undo, redo
//! and saves, and form fields are addressed by their full name.

use std::collections::HashSet;
use std::sync::Arc;

use pdfcraft_engine::{AnnotId, ClientPrefix, DocId, Edit, FieldValue, NewAnnotation, NoteIcon, PageId, Session, Shape, Style};

/// Three pages. Page 1 has a note named `file-note-1` and the widget of the text field
/// `parent.child`; page 2 has a note without `/NM`.
fn fixture() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [9 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 6 0 R >> >> >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 7 0 R /Resources << /Font << /F1 6 0 R >> >> /Annots [8 0 R 10 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Annots [11 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Length 39 >>\nstream\nBT /F1 14 Tf 20 250 Td (Page one) Tj ET\nendstream",
        "<< /Type /Annot /Subtype /Text /NM (file-note-1) /Rect [40 40 60 60] /P 3 0 R /Contents (From the file) /T (Ada) >>",
        "<< /T (parent) /Kids [10 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (child) /Parent 9 0 R /Rect [20 200 180 220] /P 3 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>",
        "<< /Type /Annot /Subtype /Text /Rect [40 40 60 60] /P 4 0 R /Contents (No name) >>",
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

fn prefix(s: &str) -> ClientPrefix {
    ClientPrefix::parse(s).unwrap()
}

fn open(bytes: Vec<u8>, client: &str) -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    s.set_client_prefix(prefix(client));
    let id = s.open("ids.pdf", None, Arc::new(bytes), None).unwrap();
    (s, id)
}

fn note(page: usize) -> Edit {
    let shape = Shape::Note { at: [100.0, 120.0], icon: NoteIcon::Comment };
    Edit::AddAnnotation(NewAnnotation { page, style: Style::default_for(&shape), shape, contents: "Note".into(), author: "Bea".into() })
}

fn names(s: &Session, id: DocId) -> Vec<String> {
    s.annotation_ids(id)
        .into_iter()
        .filter_map(|a| match a.id {
            AnnotId::Name(n) => Some(n),
            AnnotId::Object { .. } => None,
        })
        .collect()
}

/// The ids of what `s` created (names it doesn't share with the file).
fn created(s: &Session, id: DocId) -> Vec<String> {
    names(s, id).into_iter().filter(|n| n != "file-note-1").collect()
}

fn save(s: &mut Session, id: DocId) -> Arc<Vec<u8>> {
    let bytes = s.save_bytes(id).unwrap();
    s.mark_saved(id, bytes.clone(), None).unwrap();
    bytes
}

#[test]
fn client_prefixes_are_six_lowercase_hex_digits() {
    assert_eq!(prefix("0a1b2c").to_string(), "0a1b2c");
    for bad in ["", "0a1b2", "0a1b2c3", "0A1B2C", "0a1b2g", " a1b2c", "0a1b2é"] {
        assert_eq!(ClientPrefix::parse(bad), None, "{bad:?}");
    }
    let random: HashSet<String> = (0..8).map(|_| ClientPrefix::random().to_string()).collect();
    assert!(random.iter().all(|p| ClientPrefix::parse(p).is_some()));
    assert!(random.len() > 1, "a session picks its own prefix: {random:?}");
    assert_eq!(Session::new().client_prefix().to_string().len(), 6);
}

#[test]
fn new_comments_are_named_with_the_client_prefix() {
    let (mut s, id) = open(fixture(), "a1b2c3");
    s.apply(id, note(0)).unwrap();
    s.apply(id, note(2)).unwrap();
    let new = created(&s, id);
    assert_eq!(new.len(), 2);
    for n in &new {
        assert_eq!(n.len(), 36, "{n}");
        assert!(n.starts_with("a1b2c3"), "{n}");
        let groups: Vec<usize> = n.split('-').map(str::len).collect();
        assert_eq!(groups, [8, 4, 4, 4, 12], "a UUID layout, as Acrobat writes: {n}");
        assert_eq!(n.as_bytes()[14], b'4');
    }
    assert_ne!(new[0], new[1]);

    s.set_client_prefix(prefix("ffffff"));
    s.apply(id, note(1)).unwrap();
    assert!(created(&s, id).iter().any(|n| n.starts_with("ffffff")), "a later prefix names later comments");
    assert_eq!(created(&s, id).iter().filter(|n| n.starts_with("a1b2c3")).count(), 2, "earlier names stay");
}

#[test]
fn two_clients_creating_at_once_never_collide() {
    // Same file, same clock, same edits: only the prefix tells the two sessions apart.
    let (mut a, ida) = open(fixture(), "00000a");
    let (mut b, idb) = open(fixture(), "00000b");
    for _ in 0..40 {
        a.apply(ida, note(0)).unwrap();
        b.apply(idb, note(0)).unwrap();
    }
    a.apply(ida, Edit::InsertBlankPage { at: 1, width: 200.0, height: 200.0 }).unwrap();
    b.apply(idb, Edit::InsertBlankPage { at: 1, width: 200.0, height: 200.0 }).unwrap();
    a.apply(ida, Edit::DuplicatePages { pages: vec![0] }).unwrap();
    b.apply(idb, Edit::DuplicatePages { pages: vec![0] }).unwrap();

    let (na, nb) = (created(&a, ida), created(&b, idb));
    let all: HashSet<&String> = na.iter().chain(&nb).collect();
    assert_eq!(all.len(), na.len() + nb.len(), "no comment name is created twice");
    assert!(na.len() > 80);

    let (pa, pb) = (a.page_ids(ida), b.page_ids(idb));
    let new_a: HashSet<PageId> = pa.iter().copied().filter(|p| matches!(p, PageId::New { .. })).collect();
    let new_b: HashSet<PageId> = pb.iter().copied().filter(|p| matches!(p, PageId::New { .. })).collect();
    assert_eq!(new_a.len(), 2, "the inserted and the duplicated page: {pa:?}");
    assert!(new_a.is_disjoint(&new_b), "{pa:?} {pb:?}");
    let file_a: Vec<PageId> = pa.iter().copied().filter(|p| matches!(p, PageId::File { .. })).collect();
    let file_b: Vec<PageId> = pb.iter().copied().filter(|p| matches!(p, PageId::File { .. })).collect();
    assert_eq!(file_a, file_b, "both sessions name the file's pages alike");
}

#[test]
fn ids_in_the_file_are_never_rewritten() {
    let (mut s, id) = open(fixture(), "123456");
    let before = s.annotation_ids(id);
    assert_eq!(before.len(), 2, "{before:?}");
    assert_eq!(before[0].id, AnnotId::Name("file-note-1".into()));
    assert_eq!(before[1].id, AnnotId::Object { num: 11, generation: 0 }, "a note without /NM is named by its object");

    s.apply(id, Edit::MoveAnnotation { page: 0, index: 0, dx: 10.0, dy: 5.0 }).unwrap();
    s.apply(id, Edit::SetAnnotationContents { page: 0, index: 0, text: "Edited".into() }).unwrap();
    s.apply(id, Edit::SetAnnotationContents { page: 1, index: 0, text: "Edited too".into() }).unwrap();
    s.apply(id, Edit::ReplyToAnnotation { page: 1, index: 0, text: "Reply".into(), author: "Bea".into() }).unwrap();
    let after = s.annotation_ids(id);
    assert_eq!(after[0].id, AnnotId::Name("file-note-1".into()));
    assert!(after.iter().any(|a| a.id == AnnotId::Object { num: 11, generation: 0 }), "{after:?}");

    let bytes = save(&mut s, id);
    let (reopened, rid) = open(bytes.to_vec(), "654321");
    let ids = reopened.annotation_ids(rid);
    assert!(ids.iter().any(|a| a.id == AnnotId::Name("file-note-1".into())));
    assert!(ids.iter().any(|a| a.id == AnnotId::Object { num: 11, generation: 0 }), "the note is still the file's object 11: {ids:?}");
}

#[test]
fn comment_names_survive_save_open_edit_undo_and_redo() {
    let (mut s, id) = open(fixture(), "abcdef");
    s.apply(id, note(2)).unwrap();
    let mine = created(&s, id);
    assert_eq!(mine.len(), 1);
    let named = AnnotId::Name(mine[0].clone());

    s.apply(id, Edit::MoveAnnotation { page: 2, index: 0, dx: 20.0, dy: 0.0 }).unwrap();
    assert_eq!(s.find_annotation(id, &named), Some((2, 0)), "an edit keeps the name");
    s.undo(id).unwrap();
    s.undo(id).unwrap();
    assert_eq!(s.find_annotation(id, &named), None);
    s.redo(id).unwrap();
    assert_eq!(s.find_annotation(id, &named), Some((2, 0)), "redo brings the same name back");
    s.apply(id, Edit::MovePages { pages: vec![2], to: 0 }).unwrap();
    assert_eq!(s.find_annotation(id, &named), Some((0, 0)), "it follows its page");

    let bytes = save(&mut s, id);
    assert_eq!(created(&s, id), mine, "saving keeps it");
    let (other, oid) = open(bytes.to_vec(), "fedcba");
    assert_eq!(other.find_annotation(oid, &named), Some((0, 0)), "and so does opening the saved file");
}

#[test]
fn copied_comments_get_their_own_names() {
    let (mut s, id) = open(fixture(), "c0ffee");
    s.apply(id, Edit::DuplicatePages { pages: vec![0, 1] }).unwrap();
    let ids = s.annotation_ids(id);
    assert_eq!(ids.len(), 4, "{ids:?}");
    assert_eq!(ids[0].id, AnnotId::Name("file-note-1".into()), "the original keeps its name");
    assert_eq!(ids[1].id, AnnotId::Object { num: 11, generation: 0 }, "and the unnamed original stays unnamed");
    for copy in &ids[2..] {
        assert!(matches!(&copy.id, AnnotId::Name(n) if n.starts_with("c0ffee")), "{copy:?}");
    }
    assert_ne!(ids[2].id, ids[3].id);

    // Pages inserted from a file whose names this document doesn't have keep them.
    let (mut t, tid) = open(fixture(), "beef00");
    let other = {
        let (mut o, oid) = open(fixture(), "000000");
        o.apply(oid, note(2)).unwrap();
        (created(&o, oid), save(&mut o, oid))
    };
    t.apply(tid, Edit::DeletePages { pages: vec![0] }).unwrap();
    t.apply(tid, Edit::InsertPagesFrom { name: "other.pdf".into(), bytes: other.1, pages: None, at: 0, field_conflicts: Default::default() })
        .unwrap();
    let names = names(&t, tid);
    assert!(names.contains(&other.0[0]), "an inserted comment keeps a name the document doesn't use: {names:?}");
    assert!(names.contains(&"file-note-1".to_string()));
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len(), "every name once: {names:?}");
}

#[test]
fn page_ids_survive_insert_delete_reorder_undo_redo_and_saves() {
    let (mut s, id) = open(fixture(), "0f0f0f");
    let file = s.page_ids(id);
    assert_eq!(file.iter().map(ToString::to_string).collect::<Vec<_>>(), ["f3.0", "f4.0", "f5.0"]);
    let [p1, p2, p3] = file[..] else { panic!() };

    s.apply(id, Edit::InsertBlankPage { at: 1, width: 200.0, height: 200.0 }).unwrap();
    let ids = s.page_ids(id);
    let blank = ids[1];
    assert!(matches!(blank, PageId::New { prefix: p, .. } if p == prefix("0f0f0f")), "{ids:?}");
    assert!(blank.to_string().starts_with("n0f0f0f."));
    assert_eq!(PageId::parse(&blank.to_string()), Some(blank));
    assert_eq!(PageId::parse("f3.0"), Some(p1));
    // Ids are lock keys: another spelling of the same page is not an id.
    assert_eq!(PageId::parse("f03.0"), None);
    assert_eq!(PageId::parse(&blank.to_string().replace('.', ".0")), None);
    assert_eq!(ids, [p1, blank, p2, p3]);

    s.apply(id, Edit::MovePages { pages: vec![3], to: 0 }).unwrap();
    assert_eq!(s.page_ids(id), [p3, p1, blank, p2]);
    s.apply(id, Edit::DeletePages { pages: vec![1] }).unwrap();
    assert_eq!(s.page_ids(id), [p3, blank, p2]);
    assert_eq!(s.page_index(id, p1), None);
    assert_eq!(s.page_index(id, p2), Some(2));
    s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
    assert_eq!(s.page_ids(id), [p3, blank, p2], "changing a page keeps its id");

    s.undo(id).unwrap();
    s.undo(id).unwrap();
    assert_eq!(s.page_ids(id), [p3, p1, blank, p2], "undoing the delete brings the same id back");
    s.redo(id).unwrap();
    assert_eq!(s.page_ids(id), [p3, blank, p2]);

    s.apply(id, Edit::DuplicatePages { pages: vec![0] }).unwrap();
    let ids = s.page_ids(id);
    assert_eq!(ids.len(), 4);
    assert!(matches!(ids[1], PageId::New { .. }) && ids[1] != blank, "a copy is a new page: {ids:?}");
    let before = ids.clone();

    save(&mut s, id);
    assert_eq!(s.page_ids(id), before, "an incremental save keeps every id");
    let full = s.save_full_bytes(id).unwrap();
    s.mark_saved(id, full, None).unwrap();
    assert_eq!(s.page_ids(id), before, "so does a rewrite that renumbers the objects");
    s.apply(id, Edit::MovePages { pages: vec![0], to: 3 }).unwrap();
    assert_eq!(s.page_ids(id), [before[1], before[2], before[3], before[0]]);
}

#[test]
fn an_undone_page_does_not_lend_its_id_to_the_next_one() {
    let (mut s, id) = open(fixture(), "aaaaaa");
    s.apply(id, Edit::InsertBlankPage { at: 0, width: 200.0, height: 200.0 }).unwrap();
    let first = s.page_ids(id)[0];
    s.undo(id).unwrap();
    s.apply(id, Edit::InsertBlankPage { at: 0, width: 300.0, height: 300.0 }).unwrap();
    let second = s.page_ids(id)[0];
    assert!(matches!(second, PageId::New { .. }));
    assert_ne!(first, second, "the new page reuses the undone page's object number, not its id");
}

#[test]
fn form_fields_are_addressed_by_their_full_name() {
    let (mut s, id) = open(fixture(), "f1e1d1");
    let names = |s: &Session| s.get(id).unwrap().form.iter().map(|f| f.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&s), ["parent.child"]);
    s.apply(id, Edit::SetFieldValue { name: "parent.child".into(), value: FieldValue::Text("42".into()) }).unwrap();
    s.apply(id, Edit::DuplicatePages { pages: vec![2] }).unwrap();
    s.undo(id).unwrap();
    s.undo(id).unwrap();
    s.redo(id).unwrap();
    assert_eq!(names(&s), ["parent.child"]);
    let bytes = save(&mut s, id);
    let (other, oid) = open(bytes.to_vec(), "000001");
    let field = &other.get(oid).unwrap().form[0];
    assert_eq!((field.name.as_str(), field.value.as_slice()), ("parent.child", ["42".to_string()].as_slice()));
}
