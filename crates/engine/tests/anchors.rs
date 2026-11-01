//! Cloud comment anchors (ERPCons M5): made from a comment, a field or selected text, read back
//! from their JSON, and found again after edits and in a new version of the file: by region,
//! else by the text under them on the same page, else orphaned. Resolving never changes the file.

use std::sync::Arc;

use pdfcraft_engine::anchors::{Anchor, AnchorError, MAX_ANCHOR_BYTES, Resolution, quads_bounds, text_under};
use pdfcraft_engine::live::Region;
use pdfcraft_engine::{AnnotId, DocId, Edit, Session};
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};

/// Three pages. Page 1: "Hello world" (y 250) under the highlight `hl-1`, "Second line here"
/// (y 200), and the note `note-1` off the text. Page 2: "Page two text" and the field `name`.
/// Page 3: blank. `hello_y` moves the first line, `hello` replaces its words.
fn fixture_with(hello: &str, hello_y: u32) -> Vec<u8> {
    let content = format!("BT /F1 14 Tf 20 {hello_y} Td ({hello}) Tj ET BT /F1 14 Tf 20 200 Td (Second line here) Tj ET");
    let page2 = "BT /F1 14 Tf 20 250 Td (Page two text) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [10 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 6 0 R >> >> >> >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 7 0 R /Resources << /Font << /F1 6 0 R >> >> /Annots [8 0 R 9 0 R] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 11 0 R /Resources << /Font << /F1 6 0 R >> >> /Annots [10 0 R] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Annot /Subtype /Highlight /NM (hl-1) /Rect [18 244 92 266] /QuadPoints [18 266 92 266 18 244 92 244] /P 3 0 R /C [1 1 0] /Contents (Marked) >>".to_owned(),
        "<< /Type /Annot /Subtype /Text /NM (note-1) /Rect [200 40 220 60] /P 3 0 R /Contents (A note) >>".to_owned(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [20 100 180 120] /P 4 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{page2}\nendstream", page2.len()),
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

fn fixture() -> Vec<u8> {
    fixture_with("Hello world", 250)
}

fn open(bytes: &[u8]) -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_800_000_000);
    let id = s.open("anchors.pdf", None, Arc::new(bytes.to_vec()), None).unwrap();
    (s, id)
}

fn annot(s: &Session, id: DocId, name: &str) -> (usize, usize) {
    s.find_annotation(id, &AnnotId::Name(name.into())).unwrap()
}

/// The quads of `word` on page `page`, and the text under them, as the app's selection gives them.
fn select(s: &Session, id: DocId, page: usize, word: &str) -> (Vec<[f64; 8]>, String) {
    let doc = s.get(id).unwrap();
    let mut r = PageRenderer::new(doc.display.clone(), RenderConfig::default());
    let text = r.render(RenderRequest { page, kind: RequestKind::Text, scale: 1.0, ..Default::default() }).text.unwrap();
    let range = text.find(word).into_iter().next().unwrap();
    let info = &doc.info.pages[page];
    let quads: Vec<[f64; 8]> = text.line_rects(range).into_iter().map(|r| info.view_rect_to_quad(r)).collect();
    let under = text_under(&text, info, &quads);
    (quads, under)
}

fn open_anchor(r: &Resolution) -> &Anchor {
    match r {
        Resolution::Open(a) => a,
        Resolution::Orphaned => panic!("orphaned"),
    }
}

fn centre_y(a: &Anchor) -> f64 {
    let b = quads_bounds(&a.quads).unwrap();
    (b[1] + b[3]) / 2.0
}

#[test]
fn anchors_round_trip_through_their_json_and_bad_ones_are_refused() {
    let (mut s, id) = open(&fixture());
    let (page, index) = annot(&s, id, "hl-1");
    let a = s.anchor_of_annotation(id, page, index, None).unwrap();
    let json = a.to_json("Trang 1");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["region"], "annot:nm:hl-1");
    assert_eq!(value["pos"]["page"], "f3.0");
    assert_eq!(value["soft"], serde_json::json!({ "index": 0, "text": "Hello world" }));
    assert_eq!(value["label"], "Trang 1");
    assert_eq!(Anchor::parse(&json).unwrap(), a);
    let (quads, text) = select(&s, id, 0, "Second");
    let t = s.anchor_of_text(id, 0, quads, &text).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&t.to_json("")).unwrap()["region"], serde_json::Value::Null);
    assert_eq!(Anchor::parse(&t.to_json("x")).unwrap(), t);

    assert_eq!(Anchor::parse(r#"{"v":2,"soft":{"index":0}}"#), Err(AnchorError::Version));
    for bad in ["", "[]", "{}", r#"{"v":1}"#, r#"{"v":1,"soft":{"index":-1}}"#, r#"{"v":"1","soft":{"index":0}}"#] {
        assert!(matches!(Anchor::parse(bad), Err(AnchorError::Malformed(_))), "{bad}");
    }
    assert!(Anchor::parse(&format!(r#"{{"v":1,"soft":{{"index":0,"text":"{}"}}}}"#, "x".repeat(MAX_ANCHOR_BYTES))).is_err());
    // Quads that aren't eight finite numbers are dropped; order locks name no place.
    let lenient = Anchor::parse(
        r#"{"v":1,"region":"pages","pos":{"page":"zz","quads":[[1,2,3],[0,0,1,0,0,1,1,1],"x"]},"soft":{"index":2,"text":"  a \n b "}}"#,
    )
    .unwrap();
    assert_eq!(lenient, Anchor { region: None, page: None, quads: vec![[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0]], index: 2, text: "a b".into() });
}

#[test]
fn a_comment_field_or_text_gives_its_region_place_and_text() {
    let (mut s, id) = open(&fixture());
    let (page, index) = annot(&s, id, "note-1");
    let note = s.anchor_of_annotation(id, page, index, None).unwrap();
    assert_eq!(note.region, Some(Region::Annot(AnnotId::Name("note-1".into()))));
    assert_eq!(quads_bounds(&note.quads), Some([200.0, 40.0, 220.0, 60.0]));
    assert_eq!(note.text, "", "a note marks no text");
    let field = s.anchor_of_field(id, "name", 0).unwrap();
    assert_eq!(
        (field.region.clone(), field.index, quads_bounds(&field.quads)),
        (Some(Region::Field("name".into())), 1, Some([20.0, 100.0, 180.0, 120.0]))
    );
    let (quads, text) = select(&s, id, 0, "line");
    assert_eq!(text, "line");
    let t = s.anchor_of_text(id, 0, quads, &text).unwrap();
    assert_eq!((t.region.clone(), t.index, t.text.as_str()), (None, 0, "line"));
    assert!(s.anchor_of_text(id, 0, Vec::new(), "x").is_none(), "no quads, nothing selected");
    assert!(s.anchor_of_annotation(id, 0, 9, None).is_none());
    assert!(s.anchor_of_field(id, "nobody", 0).is_none());
}

#[test]
fn a_region_that_still_exists_keeps_the_anchor_and_follows_its_page() {
    let (mut s, id) = open(&fixture());
    let (page, index) = annot(&s, id, "hl-1");
    let hl = s.anchor_of_annotation(id, page, index, None).unwrap();
    let field = s.anchor_of_field(id, "name", 0).unwrap();
    s.apply(id, Edit::MovePages { pages: vec![0], to: 3 }).unwrap();
    s.apply(id, Edit::InsertBlankPage { at: 0, width: 300.0, height: 300.0 }).unwrap();
    let out = s.resolve_anchors(id, &[hl.clone(), field.clone()]);
    let moved = open_anchor(&out[0]);
    assert_eq!((moved.region.clone(), moved.page, moved.index), (hl.region.clone(), hl.page, 3));
    assert_eq!(moved.quads, hl.quads);
    assert_eq!(open_anchor(&out[1]).index, 1);
    assert_eq!(s.anchor_place(id, &hl).map(|p| p.page), Some(3));
}

#[test]
fn a_deleted_highlight_falls_back_to_its_text_and_a_deleted_note_is_orphaned() {
    let (mut s, id) = open(&fixture());
    let (page, index) = annot(&s, id, "hl-1");
    let hl = s.anchor_of_annotation(id, page, index, None).unwrap();
    let (page, index) = annot(&s, id, "note-1");
    let note = s.anchor_of_annotation(id, page, index, None).unwrap();
    for name in ["hl-1", "note-1"] {
        let (page, index) = annot(&s, id, name);
        s.apply(id, Edit::DeleteAnnotation { page, index }).unwrap();
    }
    let out = s.resolve_anchors(id, &[hl.clone(), note]);
    let kept = open_anchor(&out[0]);
    assert_eq!((kept.region.clone(), kept.page, kept.index, kept.text.as_str()), (None, hl.page, 0, "Hello world"));
    assert_eq!(out[1], Resolution::Orphaned);
}

#[test]
fn text_anchors_follow_their_page_and_find_their_text_in_a_new_version() {
    let (mut s, id) = open(&fixture());
    let (quads, text) = select(&s, id, 0, "world");
    let a = s.anchor_of_text(id, 0, quads, &text).unwrap();
    s.apply(id, Edit::MovePages { pages: vec![0], to: 2 }).unwrap();
    let out = s.resolve_anchors(id, std::slice::from_ref(&a));
    assert_eq!((open_anchor(&out[0]).index, open_anchor(&out[0]).quads.clone()), (2, a.quads.clone()), "by its page id, quads unchanged");

    // A newer version moved the line down the page: same page, the text found where it went.
    let (mut v2, id2) = open(&fixture_with("Hello world", 150));
    let out = v2.resolve_anchors(id2, std::slice::from_ref(&a));
    let found = open_anchor(&out[0]);
    assert_eq!((found.index, found.text.as_str()), (0, "world"));
    assert!((centre_y(found) - 154.0).abs() < 8.0, "moved with the line: {}", centre_y(found));
    assert!((centre_y(&a) - 254.0).abs() < 8.0);

    // Another version without it: orphaned.
    let (mut v3, id3) = open(&fixture_with("Goodbye", 250));
    assert_eq!(v3.resolve_anchors(id3, std::slice::from_ref(&a)), [Resolution::Orphaned]);
}

#[test]
fn an_unknown_page_id_falls_back_to_the_page_index_checked_by_its_text() {
    let (mut s, id) = open(&fixture());
    let (quads, text) = select(&s, id, 1, "two");
    let mut a = s.anchor_of_text(id, 1, quads, &text).unwrap();
    a.page = pdfcraft_engine::PageId::parse("f99.0");
    assert_eq!(open_anchor(&s.resolve_anchors(id, std::slice::from_ref(&a))[0]).index, 1);
    // The page id now names another page whose text differs: the index's page wins.
    a.page = s.page_ids(id).first().copied();
    let r = s.resolve_anchors(id, std::slice::from_ref(&a));
    assert_eq!((open_anchor(&r[0]).index, open_anchor(&r[0]).page), (1, s.page_ids(id).get(1).copied()));
    s.apply(id, Edit::DeletePages { pages: vec![1] }).unwrap();
    assert_eq!(s.resolve_anchors(id, std::slice::from_ref(&a)), [Resolution::Orphaned], "the page and its text are gone");
}

#[test]
fn resolving_and_placing_change_nothing_in_the_document() {
    let (mut s, id) = open(&fixture());
    let (page, index) = annot(&s, id, "hl-1");
    let hl = s.anchor_of_annotation(id, page, index, None).unwrap();
    let (quads, text) = select(&s, id, 0, "Second");
    let t = s.anchor_of_text(id, 0, quads, &text).unwrap();
    let (bytes, generation) = (s.get(id).unwrap().bytes.clone(), s.get(id).unwrap().edit_generation());
    s.resolve_anchors(id, &[hl.clone(), t.clone()]);
    s.anchor_place(id, &hl);
    s.anchor_place(id, &t);
    let doc = s.get(id).unwrap();
    assert_eq!((doc.bytes.as_slice(), doc.edit_generation(), doc.dirty), (bytes.as_slice(), generation, false));
}
