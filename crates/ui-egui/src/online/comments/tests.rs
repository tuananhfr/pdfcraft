//! The comment calls against a fake shell, natively: the selection's anchor, markers for open
//! threads, a click reported, going to an anchor, finding anchors again (also after another
//! person's change through the fake room of the co-editing tests), and none of it touching the
//! document.

use pdfcraft_engine::{AnnotId, DocId, Edit};
use pdfcraft_render::{PageRenderer, RenderConfig, RenderRequest, RequestKind};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::PdfCraftApp;
use crate::online::live::tests::{log, pair_with};
use crate::online::tests::{Fake, app, open_with, tick};

/// Page 1: "Hello world" and the note `note-1`. Page 2: "Page two text", "Page two" under the
/// highlight `hl-2`. Page 3: the field `name`.
fn fixture() -> Vec<u8> {
    let page1 = "BT /F1 14 Tf 20 250 Td (Hello world) Tj ET";
    let page2 = "BT /F1 14 Tf 20 250 Td (Page two text) Tj ET";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [11 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 6 0 R >> >> >> >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 7 0 R /Resources << /Font << /F1 6 0 R >> >> /Annots [9 0 R] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 8 0 R /Resources << /Font << /F1 6 0 R >> >> /Annots [10 0 R] >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Annots [11 0 R] >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{page1}\nendstream", page1.len()),
        format!("<< /Length {} >>\nstream\n{page2}\nendstream", page2.len()),
        "<< /Type /Annot /Subtype /Text /NM (note-1) /Rect [200 40 220 60] /P 3 0 R /Contents (A note) >>".to_owned(),
        "<< /Type /Annot /Subtype /Highlight /NM (hl-2) /Rect [18 244 80 266] /QuadPoints [18 266 80 266 18 244 80 244] /P 4 0 R /C [1 1 0] >>"
            .to_owned(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [20 100 180 120] /P 5 0 R /F 4 /DA (/Helv 10 Tf 0 g) >>".to_owned(),
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

#[derive(Clone, Default)]
struct Shell(Rc<RefCell<Vec<String>>>);

impl CommentHost for Shell {
    fn comment_clicked(&self, thread: &str) {
        self.0.borrow_mut().push(thread.to_owned());
    }
}

fn with_comments(app: &mut PdfCraftApp) -> (Shell, CommentInbox, SelectionAnchor) {
    let shell = Shell::default();
    app.online = Some(app.online.take().unwrap().with_comments(Box::new(shell.clone())));
    let (inbox, selection) = app.online.as_ref().unwrap().comment_channels().unwrap();
    (shell, inbox, selection)
}

/// An app with the fixture open as a host document, and the shell's comment channels.
fn setup() -> (PdfCraftApp, Fake, DocId, Shell, CommentInbox, SelectionAnchor) {
    let (mut app, fake) = app();
    let (shell, inbox, selection) = with_comments(&mut app);
    let id = open_with(&mut app, "d1", &fixture());
    tick(&mut app, &fake, 0.0);
    app.comments_step();
    (app, fake, id, shell, inbox, selection)
}

fn json(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn annot(app: &PdfCraftApp, id: DocId, name: &str) -> (usize, usize) {
    app.session.find_annotation(id, &AnnotId::Name(name.into())).unwrap()
}

/// The anchor of comment `name`, as the shell got it from `anchorOfSelection`.
fn anchor_of(app: &mut PdfCraftApp, id: DocId, selection: &SelectionAnchor, name: &str) -> String {
    let at = annot(app, id, name);
    app.views[0].comments.selected = Some(at);
    app.comments_step();
    let anchor = selection.lock().unwrap().clone().unwrap();
    app.views[0].comments.selected = None;
    app.comments_step();
    anchor
}

fn thread(id: &str, anchor: &str, status: &str) -> Thread {
    Thread { id: id.into(), anchor: anchor.into(), status: status.into(), count: 2 }
}

fn resolve(app: &mut PdfCraftApp, inbox: &CommentInbox, threads: Vec<Thread>) -> Vec<ResolvedThread> {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    inbox.lock().unwrap().push(CommentEvent::Resolve { threads, reply: Box::new(move |r| *slot.borrow_mut() = Some(r)) });
    app.comments_step();
    out.borrow_mut().take().expect("answered on the next frame")
}

#[test]
fn the_selection_anchor_follows_a_selected_comment_a_field_and_text() {
    let (mut app, _fake, id, _shell, _inbox, selection) = setup();
    assert_eq!(*selection.lock().unwrap(), None, "nothing selected");
    let hl = json(&anchor_of(&mut app, id, &selection, "hl-2"));
    assert_eq!((hl["v"].clone(), hl["region"].clone(), hl["label"].clone()), (1.into(), "annot:nm:hl-2".into(), "Page 2".into()));
    assert_eq!(hl["soft"], serde_json::json!({ "index": 1, "text": "Page two" }));
    assert_eq!(*selection.lock().unwrap(), None, "deselected");

    app.views[0].forms.focus = Some(crate::forms_ui::Focus {
        name: "name".into(),
        widget: 0,
        text: String::new(),
        picked: Vec::new(),
        request_focus: false,
        select_all: false,
        calendar: None,
        calendar_rect: None,
    });
    app.comments_step();
    let field = json(selection.lock().unwrap().as_deref().unwrap());
    assert_eq!((field["region"].clone(), field["label"].clone()), ("field:name".into(), "Page 3".into()));
    app.views[0].forms.focus = None;

    let display = app.session.get(id).unwrap().display.clone();
    let text = PageRenderer::new(display, RenderConfig::default())
        .render(RenderRequest { page: 0, kind: RequestKind::Text, scale: 1.0, ..Default::default() })
        .text
        .unwrap();
    let world = text.find("world").into_iter().next().unwrap();
    app.views[0].put_page_text(0, text);
    app.views[0].select_text(0, world.start, world.end - 1);
    app.comments_step();
    let words = json(selection.lock().unwrap().as_deref().unwrap());
    assert_eq!((words["region"].clone(), words["soft"]["text"].clone(), words["label"].clone()), (Value::Null, "world".into(), "Page 1".into()));
    assert_eq!(words["pos"]["page"], "f3.0");
    app.views[0].clear_selection();
    app.comments_step();
    assert_eq!(*selection.lock().unwrap(), None);
}

#[test]
fn open_threads_get_markers_and_a_click_reaches_the_shell() {
    let (mut app, _fake, id, shell, inbox, selection) = setup();
    let hl = anchor_of(&mut app, id, &selection, "hl-2");
    let note = anchor_of(&mut app, id, &selection, "note-1");
    inbox.lock().unwrap().push(CommentEvent::Show(vec![
        thread("t1", &hl, "open"),
        thread("t2", &note, "resolved"),
        thread("t3", &note, "orphaned"),
        thread("t4", "{not an anchor", "open"),
        thread("t5", &note, "open"),
    ]));
    app.comments_step();
    let markers: Vec<(String, usize, u32)> = app.views[0].comment_markers.iter().map(|m| (m.thread.clone(), m.page, m.count)).collect();
    assert_eq!(markers, [("t1".to_string(), 1, 2), ("t5".to_string(), 0, 2)], "open threads only");
    assert_eq!(app.views[0].comment_markers[1].rects, [[200.0, 40.0, 220.0, 60.0]]);
    assert!(shell.0.borrow().is_empty());
    app.views[0].comment_clicked = Some("t5".into());
    app.comments_step();
    assert_eq!(*shell.0.borrow(), ["t5"]);
    assert_eq!(app.views[0].comment_clicked, None);
    inbox.lock().unwrap().push(CommentEvent::Show(Vec::new()));
    app.comments_step();
    assert!(app.views[0].comment_markers.is_empty());
}

#[test]
fn markers_sit_beside_their_anchor_and_never_on_top_of_each_other() {
    let info = pdfcraft_render::DocInfo {
        pages: vec![pdfcraft_render::PageInfo { width: 300.0, height: 300.0, label: "1".into(), crop: [0.0, 0.0, 300.0, 300.0], rotation: 0 }],
        ..Default::default()
    };
    let xf =
        crate::canvas::PageXform { rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 300.0)), rot: 0, pw: 300.0, ph: 300.0 };
    let marker = |thread: &str| CommentMarker { thread: thread.into(), page: 0, rects: vec![[20.0, 240.0, 80.0, 260.0]], count: 1 };
    let bubbles = markers::bubble_rects(&xf, &info, 0, &[marker("a"), marker("b"), CommentMarker { page: 1, ..marker("c") }]);
    assert_eq!(bubbles.len(), 2, "page 1's marker isn't drawn on page 0");
    assert!((bubbles[0].left() - 84.0).abs() < 0.01, "right of the anchor: {:?}", bubbles[0]);
    assert!(!bubbles[0].intersects(bubbles[1]));
    let edge = markers::bubble_rects(&xf, &info, 0, &[CommentMarker { rects: vec![[250.0, 280.0, 299.0, 299.0]], ..marker("d") }]);
    assert!(xf.rect.contains_rect(edge[0]), "kept on the page: {:?}", edge[0]);
}

/// Anyone who may comment can pile threads on one spot: a page draws a bounded number of
/// bubbles, so placing them each frame stays cheap (the panel still lists every thread).
#[test]
fn a_page_draws_a_bounded_number_of_markers() {
    let info = pdfcraft_render::DocInfo {
        pages: vec![pdfcraft_render::PageInfo { width: 300.0, height: 300.0, label: "1".into(), crop: [0.0, 0.0, 300.0, 300.0], rotation: 0 }],
        ..Default::default()
    };
    let xf =
        crate::canvas::PageXform { rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 300.0)), rot: 0, pw: 300.0, ph: 300.0 };
    let piled: Vec<CommentMarker> =
        (0..2000).map(|i| CommentMarker { thread: format!("t{i}"), page: 0, rects: vec![[20.0, 240.0, 80.0, 260.0]], count: 1 }).collect();
    let bubbles = markers::bubble_rects(&xf, &info, 0, &piled);
    assert_eq!(bubbles.len(), markers::MAX_MARKERS_PER_PAGE);
}

#[test]
fn focusing_an_anchor_goes_to_its_page_and_selects_its_comment() {
    let (mut app, _fake, id, _shell, inbox, selection) = setup();
    let hl = anchor_of(&mut app, id, &selection, "hl-2");
    assert_eq!(app.views[0].current, 0);
    inbox.lock().unwrap().push(CommentEvent::Focus(hl));
    app.comments_step();
    assert_eq!(app.views[0].current, 1);
    assert_eq!(app.views[0].comments.selected, Some(annot(&app, id, "hl-2")));
    assert_eq!(app.views[0].flash.map(|f| f.0), Some(1));
    inbox.lock().unwrap().push(CommentEvent::Focus("garbage".into()));
    app.comments_step();
    assert_eq!(app.views[0].current, 1, "an anchor that can't be read goes nowhere");
}

#[test]
fn resolving_answers_each_readable_thread_kept_moved_or_orphaned() {
    let (mut app, _fake, id, _shell, inbox, selection) = setup();
    let hl = anchor_of(&mut app, id, &selection, "hl-2");
    let note = anchor_of(&mut app, id, &selection, "note-1");
    let newer = r#"{"v":9,"soft":{"index":0}}"#;
    let out = resolve(&mut app, &inbox, vec![thread("t1", &hl, "open"), thread("t2", newer, "open"), thread("t3", &note, "resolved")]);
    assert_eq!(out.iter().map(|r| (r.id.as_str(), r.status)).collect::<Vec<_>>(), [("t1", AnchorStatus::Open), ("t3", AnchorStatus::Open)]);
    assert_eq!(json(&out[0].anchor), json(&hl), "nothing changed");

    app.active = Some(0);
    let (page, index) = annot(&app, id, "hl-2");
    assert!(app.apply_edit(Edit::DeleteAnnotation { page, index }));
    let (page, index) = annot(&app, id, "note-1");
    assert!(app.apply_edit(Edit::DeleteAnnotation { page, index }));
    let out = resolve(&mut app, &inbox, vec![thread("t1", &hl, "open"), thread("t3", &note, "open")]);
    let kept = json(&out[0].anchor);
    assert_eq!((out[0].status, kept["region"].clone(), kept["soft"]["text"].clone()), (AnchorStatus::Open, Value::Null, "Page two".into()));
    assert_eq!((out[1].status, out[1].anchor.as_str()), (AnchorStatus::Orphaned, note.as_str()), "an orphan keeps its anchor");
}

#[test]
fn markers_focus_and_resolving_never_change_the_document() {
    let (mut app, fake, id, _shell, inbox, selection) = setup();
    let hl = anchor_of(&mut app, id, &selection, "hl-2");
    let doc = app.session.get(id).unwrap();
    let (bytes, display, generation) = (doc.bytes.clone(), doc.display.clone(), doc.edit_generation());
    inbox.lock().unwrap().push(CommentEvent::Show(vec![thread("t1", &hl, "open")]));
    inbox.lock().unwrap().push(CommentEvent::Focus(hl.clone()));
    resolve(&mut app, &inbox, vec![thread("t1", &hl, "open")]);
    assert_eq!(app.views[0].comment_markers.len(), 1);
    tick(&mut app, &fake, 60_000.0);
    let doc = app.session.get(id).unwrap();
    assert_eq!(
        (doc.bytes.as_slice(), doc.display.as_slice(), doc.edit_generation(), doc.dirty),
        (bytes.as_slice(), display.as_slice(), generation, false)
    );
    assert!(fake.saves().is_empty(), "nothing to save");
    assert!(!fake.take_calls().iter().any(|c| c.starts_with("dirty")));
}

/// The thread list comes from the server: the app reads and resolves a bounded number of them.
#[test]
fn a_bounded_number_of_threads_is_read_and_resolved() {
    let (mut app, _fake, _id, _shell, inbox, _selection) = setup();
    let anchor = r#"{"v":1,"pos":{"page":null,"quads":[[1,2,3,2,1,1,3,1]]},"soft":{"index":0,"text":"nowhere to be found"}}"#;
    let many: Vec<Thread> = (0..MAX_THREADS + 500).map(|i| thread(&format!("t{i}"), anchor, "open")).collect();
    inbox.lock().unwrap().push(CommentEvent::Show(many.clone()));
    app.comments_step();
    assert_eq!(app.online.as_ref().unwrap().comments.as_ref().unwrap().threads.len(), MAX_THREADS);
    assert_eq!(resolve(&mut app, &inbox, many).len(), MAX_THREADS);
}

#[test]
fn without_the_comment_calls_nothing_changes() {
    let (mut app, fake) = app();
    let id = open_with(&mut app, "d1", &fixture());
    tick(&mut app, &fake, 0.0);
    assert!(app.online.as_ref().unwrap().comment_channels().is_none());
    app.views[0].comments.selected = Some(annot(&app, id, "hl-2"));
    app.comments_tick();
    assert!(app.views[0].comment_markers.is_empty());
    // The desktop has no shell at all.
    let mut desktop = PdfCraftApp::new();
    desktop.comments_tick();
    assert!(desktop.online.is_none());
}

/// Another person moves the page, then deletes the highlight, then the page; this side finds
/// the anchor on the moved page, then by its text, then orphans it.
#[test]
fn another_persons_changes_move_the_anchor_through_the_room() {
    let (mut a, mut b, room) = pair_with(&fixture());
    let (_shell, inbox, selection) = with_comments(&mut a.app);
    a.app.comments_step();
    let hl = anchor_of(&mut a.app, a.id, &selection, "hl-2");
    inbox.lock().unwrap().push(CommentEvent::Show(vec![thread("t1", &hl, "open")]));
    a.app.comments_step();
    assert_eq!(a.app.views[0].comment_markers.iter().map(|m| m.page).collect::<Vec<_>>(), [1]);

    assert!(b.edit(Edit::MovePages { pages: vec![1], to: 0 }));
    b.tick(1.0);
    a.tick(1.0);
    assert_eq!(log(&room), ["1: willEdit pages", "1: changed pages"]);
    a.app.comments_step();
    assert_eq!(a.app.views[0].comment_markers.iter().map(|m| m.page).collect::<Vec<_>>(), [0], "the marker follows the page");
    let moved = resolve(&mut a.app, &inbox, vec![thread("t1", &hl, "open")]);
    let anchor = json(&moved[0].anchor);
    assert_eq!(
        (moved[0].status, anchor["region"].clone(), anchor["soft"]["index"].clone(), anchor["label"].clone()),
        (AnchorStatus::Open, "annot:nm:hl-2".into(), 0.into(), "Page 1".into())
    );
    assert_eq!(anchor["pos"]["page"], json(&hl)["pos"]["page"], "the same page, by its id");

    let (page, index) = annot(&b.app, b.id, "hl-2");
    assert!(b.edit(Edit::DeleteAnnotation { page, index }));
    b.tick(2.0);
    a.tick(2.0);
    assert!(a.app.session.find_annotation(a.id, &AnnotId::Name("hl-2".into())).is_none(), "the deletion reached this side");
    let kept = resolve(&mut a.app, &inbox, vec![thread("t1", &moved[0].anchor, "open")]);
    let anchor = json(&kept[0].anchor);
    assert_eq!((kept[0].status, anchor["region"].clone(), anchor["soft"]["text"].clone()), (AnchorStatus::Open, Value::Null, "Page two".into()));

    b.tick(2.0 + crate::online::live::IDLE_END_MS + 1.0);
    a.tick(2.0 + crate::online::live::IDLE_END_MS + 1.0);
    assert!(b.edit(Edit::DeletePages { pages: vec![0] }));
    b.tick(2.0 * crate::online::live::IDLE_END_MS + 3.0);
    a.tick(2.0 * crate::online::live::IDLE_END_MS + 3.0);
    assert_eq!(a.app.session.get(a.id).unwrap().info.pages.len(), 2);
    let gone = resolve(&mut a.app, &inbox, vec![thread("t1", &kept[0].anchor, "open")]);
    assert_eq!(gone[0].status, AnchorStatus::Orphaned);
}

/// The shell's "Nhận xét" panel lists the comments: opening a file that has some, or picking a
/// comment tool, leaves the app's own Comments pane closed (opening it narrowed the page and
/// refitted its zoom); asking for it still opens it. Without the shell's panel both open it.
#[test]
fn with_the_shells_comment_panel_the_comments_pane_opens_only_when_asked() {
    let (mut plain, fake) = app();
    open_with(&mut plain, "d1", &fixture());
    tick(&mut plain, &fake, 0.0);
    assert_eq!(plain.right, Some(crate::RightPanel::Comments), "Acrobat's behaviour without the shell's panel");
    plain.right = None;
    assert!(plain.execute("comment.note"));
    assert_eq!(plain.right, Some(crate::RightPanel::Comments));

    let (mut app, _fake, _id, _shell, _inbox, _selection) = setup();
    assert_eq!(app.right, None, "opening a file with comments");
    assert!(app.execute("comment.note"));
    assert_eq!(app.quick_tool, crate::QuickTool::Comment(crate::comments::CommentTool::Note));
    assert_eq!(app.right, None, "picking a comment tool");
    assert!(app.execute("comment.list"));
    assert_eq!(app.right, Some(crate::RightPanel::Comments), "the user's own choice");
}
