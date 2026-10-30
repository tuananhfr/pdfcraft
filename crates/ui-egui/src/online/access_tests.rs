//! `erpHost` v2 against a fake shell, natively: view and comment refuse every edit family, keys,
//! paste and drop (rule 12) and send nothing (rule 13); `setMode` both ways keeps the document and
//! its undo history; `canDownload: false` keeps the content in the app; `preserveView` keeps the
//! page and the zoom; `setUser` authors new comments.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use pdfcraft_engine::{AddedText, DocId, Edit, EditError, HistoryEntry, NewAnnotation, NoteIcon, Shape, Style};
use pdfcraft_render::{RenderRequest, RequestKind};

use super::tests::{FORM, Fake, app as fake_app, edit, tick};
use super::*;
use crate::access_guard::{comment_only_notice, no_extract_notice, view_only_notice};
use crate::canvas::Fit;
use crate::{PdfCraftApp, QuickTool};

const READ_ONLY: [AccessMode; 2] = [AccessMode::View, AccessMode::Comment];

fn access(mode: AccessMode) -> Access {
    Access { mode, can_download: true }
}

/// `open(doc)` of `doc_id` with the v2 fields; returns the active document.
fn open_as(app: &mut PdfCraftApp, doc_id: &str, bytes: Vec<u8>, access: Access, preserve_view: bool) -> DocId {
    let doc = HostDocument::new(doc_id.into(), Some(1.0), "Hợp đồng.pdf".into(), "pdf".into(), bytes).with_access(access, preserve_view);
    app.online.as_ref().unwrap().inbox().lock().unwrap().push(doc);
    app.open_host_documents();
    app.active_ids().unwrap().1
}

/// `setMode(doc_id, …)`, applied by the next frame.
fn set_mode(app: &mut PdfCraftApp, fake: &Fake, doc_id: &str, access: Access) {
    app.online.as_ref().unwrap().modes().lock().unwrap().push(SetMode { doc_id: doc_id.into(), access });
    tick(app, fake, 0.0);
}

/// `n` pages saying "Page 1", "Page 2", …
fn pages(n: usize) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".into()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 200 300] >>", kids.join(" ")));
    objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for i in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>", 5 + 2 * i));
        let body = format!("BT /F1 24 Tf 20 150 Td (Page {}) Tj ET", i + 1);
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
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

/// Everything a refused change must leave as it was.
#[derive(Debug, PartialEq)]
struct Snapshot {
    bytes: Arc<Vec<u8>>,
    generation: u64,
    dirty: bool,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    pages: usize,
    annotations: usize,
}

fn snapshot(app: &PdfCraftApp, id: DocId) -> Snapshot {
    let d = app.session.get(id).unwrap();
    Snapshot {
        bytes: d.bytes.clone(),
        generation: d.edit_generation(),
        dirty: d.dirty,
        undo: d.undo_history(),
        redo: d.redo_history(),
        pages: d.info.pages.len(),
        annotations: d.info.annotations.len(),
    }
}

/// Host document "d1" (form.pdf) with an edit to undo and one to redo, then put in `mode`.
fn with_history(mode: AccessMode) -> (PdfCraftApp, Fake, DocId) {
    let (mut app, fake) = fake_app();
    let id = open_as(&mut app, "d1", FORM.to_vec(), Access::default(), false);
    edit(&mut app, id, "Ada");
    edit(&mut app, id, "Grace");
    app.undo();
    set_mode(&mut app, &fake, "d1", access(mode));
    app.toast = None;
    (app, fake, id)
}

fn toast(app: &PdfCraftApp) -> Option<&str> {
    app.toast.as_ref().map(|t| t.0.as_str())
}

/// The notice the active document refuses changes with: comment mode says commenting comes later.
fn refused_with(app: &PdfCraftApp) -> &'static str {
    let id = app.active_ids().unwrap().1;
    match app.online.as_ref().unwrap().records[&id].access.mode {
        AccessMode::Comment => comment_only_notice(),
        _ => view_only_notice(),
    }
}

fn tool_read_only(app: &mut PdfCraftApp, tool: QuickTool) {
    let why = refused_with(app);
    tool_refused(app, tool, why);
}

/// Command `id` is greyed and, run anyway (key, palette, control channel), says why.
fn command_refused(app: &mut PdfCraftApp, id: &str, why: &str) {
    let spec = pdfcraft_engine::commands::command(id).unwrap();
    assert!(!app.command_enabled(spec), "{id} is offered");
    app.toast = None;
    assert!(!app.execute(id), "{id} ran");
    assert_eq!(toast(app), Some(why), "{id}");
}

fn commands_refused(app: &mut PdfCraftApp, ids: &[&str]) {
    for id in ids {
        let why = refused_with(app);
        command_refused(app, id, why);
    }
}

/// Tool `tool`, however it was picked, is dropped with the notice.
fn tool_refused(app: &mut PdfCraftApp, tool: QuickTool, why: &str) {
    app.quick_tool = tool;
    app.toast = None;
    app.refuse_tools_on_limited();
    assert_eq!(app.quick_tool, QuickTool::Select, "{tool:?}");
    assert_eq!(toast(app), Some(why), "{tool:?}");
}

/// An edit that reached the app anyway (a view's queued edit, a dialog) is refused by the engine.
fn edit_refused(app: &mut PdfCraftApp, edit: Edit) {
    app.toast = None;
    assert!(!app.apply_edit(edit));
    assert_eq!(toast(app), Some(refused_with(app)));
}

fn note() -> Edit {
    let shape = Shape::Note { at: [100.0, 120.0], icon: NoteIcon::Comment };
    Edit::AddAnnotation(NewAnnotation { page: 0, style: Style::default_for(&shape), shape, contents: "Ổn".into(), author: "A".into() })
}

/// Each edit family runs `refuse` in view and in comment mode on a document with history, and
/// the document and its history are untouched afterwards.
fn each_read_only_mode(refuse: impl Fn(&mut PdfCraftApp, DocId)) {
    for mode in READ_ONLY {
        let (mut app, _fake, id) = with_history(mode);
        let before = snapshot(&app, id);
        refuse(&mut app, id);
        assert_eq!(snapshot(&app, id), before, "{mode:?}");
    }
}

#[test]
fn annotations_are_refused_in_view_and_comment() {
    each_read_only_mode(|app, id| {
        assert!(!app.session.get(id).unwrap().allows_annotation());
        commands_refused(
            app,
            &["comment.note", "comment.highlight", "comment.ink", "comment.stamp", "comment.import", "sign.fill.text", "measure.distance"],
        );
        tool_read_only(app, QuickTool::Comment(crate::comments::CommentTool::Note));
        tool_read_only(app, QuickTool::Fill(crate::fill_sign::FillTool::Text));
        edit_refused(app, note());
    });
}

#[test]
fn form_fields_are_refused_in_view_and_comment() {
    each_read_only_mode(|app, id| {
        assert!(!app.session.get(id).unwrap().allows_form_filling(), "fields take no typing or clicks");
        commands_refused(app, &["form.clear", "form.add.text", "form.prepare", "form.detect", "form.flatten", "form.import_data"]);
        tool_read_only(app, QuickTool::Field(crate::prepare::FieldTool::Text));
        edit_refused(app, Edit::SetFieldValue { name: "name".into(), value: pdfcraft_engine::FieldValue::Text("Hopper".into()) });
    });
}

#[test]
fn page_operations_are_refused_in_view_and_comment() {
    each_read_only_mode(|app, id| {
        assert!(!app.session.get(id).unwrap().allows_assembly());
        commands_refused(app, &["page.rotate", "page.delete", "page.insert_blank", "page.duplicate", "page.paste", "page.crop", "page.number"]);
        tool_read_only(app, QuickTool::Crop);
        edit_refused(app, Edit::RotatePages { pages: vec![0], degrees: 90 });
        edit_refused(app, Edit::InsertBlankPage { at: 0, width: 200.0, height: 300.0 });
    });
}

#[test]
fn text_editing_is_refused_in_view_and_comment() {
    each_read_only_mode(|app, id| {
        assert!(!app.session.get(id).unwrap().allows_modification());
        commands_refused(app, &["edit.edit_text", "edit.text", "edit.image", "edit.link", "edit.watermark", "edit.header_footer"]);
        for tool in [QuickTool::EditText, QuickTool::AddText, QuickTool::Link] {
            tool_read_only(app, tool);
        }
        edit_refused(
            app,
            Edit::AddText { page: 0, text: AddedText { rect: [20.0, 20.0, 120.0, 40.0], text: "Thêm".into(), ..AddedText::default() } },
        );
    });
}

#[test]
fn redaction_is_refused_in_view_and_comment() {
    each_read_only_mode(|app, _| {
        commands_refused(app, &["redact.mark", "redact.search", "redact.pages", "redact.apply", "redact.sanitize", "protect.remove_hidden"]);
        tool_read_only(app, QuickTool::Redact);
        let mark = app.redact_prefs.mark(0, vec![[20.0, 40.0, 120.0, 40.0, 20.0, 20.0, 120.0, 20.0]], "A");
        edit_refused(app, mark);
    });
}

#[test]
fn signing_is_refused_in_view_and_comment() {
    each_read_only_mode(|app, _| {
        commands_refused(app, &["sign.digital", "sign.certify", "sign.certify_invisible", "sign.fill.signature", "protect.password"]);
        tool_read_only(app, QuickTool::SignArea { certify: false });
        app.toast = None;
        app.start_signing(0, Some([20.0, 20.0, 140.0, 60.0]), None, None);
        assert!(app.dialog.is_none(), "no signing dialog");
        assert_eq!(toast(app), Some(refused_with(app)));
    });
}

#[test]
fn ocr_is_refused_in_view_and_comment() {
    each_read_only_mode(|app, _| {
        commands_refused(app, &["ocr.recognize", "ocr.enhance_camera"]);
        tool_read_only(app, QuickTool::EnhanceCamera);
        app.toast = None;
        app.start_ocr();
        assert!(app.ocr_run.is_none(), "nothing recognised");
        assert_eq!(toast(app), Some(refused_with(app)));
    });
}

#[test]
fn scripts_that_would_change_the_document_are_refused_in_view_and_comment() {
    each_read_only_mode(|app, id| {
        commands_refused(app, &["tools.document_js"]);
        app.session.set_javascript(true);
        let run = app.session.run_javascript(id, "this.getField('name').value = 'Hopper';", None);
        assert!(matches!(run, Err(EditError::ViewOnly)), "{run:?}");
        edit_refused(app, Edit::XfaEvent { som: "form1.button".into(), activity: "click".into() });
    });
}

#[test]
fn undo_redo_and_revert_are_refused_in_view_and_comment() {
    each_read_only_mode(|app, id| {
        assert!(app.session.get(id).unwrap().can_undo().is_some(), "the history is kept, only out of reach");
        commands_refused(app, &["edit.undo", "edit.redo", "file.revert"]);
        app.toast = None;
        app.undo();
        assert_eq!(toast(app), Some(refused_with(app)));
        app.redo();
        assert_eq!(toast(app), Some(refused_with(app)));
    });
}

/// Key input, paste and drop on a real frame: Ctrl+Z, Ctrl+Y, Delete on selected pages, a pasted
/// page and files dropped on the page grid change nothing (a dropped file opens in a tab of its own).
#[test]
fn keys_paste_and_drop_are_refused_in_view_and_comment() {
    #[derive(Debug)]
    struct Dropped(&'static str, Vec<u8>);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            std::path::Path::new(self.0)
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(self.1.clone())
        }
    }
    for mode in READ_ONLY {
        let (mut app, fake) = fake_app();
        app.set_option("language", "en").unwrap();
        let id = open_as(&mut app, "d1", pages(3), Access::default(), false);
        assert!(app.apply_edit(Edit::RotatePages { pages: vec![0], degrees: 90 }));
        app.copy_pages(false);
        assert!(app.page_clipboard.is_some());
        set_mode(&mut app, &fake, "d1", access(mode));
        app.set_option("organize", "on").unwrap();
        let before = snapshot(&app, id);
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_| app);
        h.run_steps(4);
        h.state_mut().views[0].select_pages(&[1]);
        h.run_steps(1);
        let keys = [
            (Modifiers::COMMAND, Key::Z),
            (Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
            (Modifiers::COMMAND, Key::Y),
            (Modifiers::NONE, Key::Delete),
            (Modifiers::NONE, Key::Backspace),
        ];
        for (modifiers, key) in keys {
            h.key_press_modifiers(modifiers, key);
            h.run_steps(2);
        }
        for event in [egui::Event::Paste(String::new()), egui::Event::Cut] {
            h.event(event);
            h.run_steps(2);
        }
        h.hover_at(egui::pos2(700.0, 450.0));
        h.run_steps(1);
        h.input_mut().dropped_files.push(Arc::new(Dropped("extra.pdf", pages(1))));
        h.run_steps(1);
        // A dropped file opens on a worker thread; under a loaded workspace run that takes a while.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while (!h.state().pending_opens.is_empty() || h.state().views.len() < 2) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            h.run_steps(1);
        }
        assert_eq!(snapshot(h.state(), id), before, "{mode:?}");
        assert_eq!(h.state().views.len(), 2, "the dropped file opened on its own: {mode:?}");
    }
}

/// Rule 13: no `dirty`, no AutoSave, Save says read-only and sends nothing, `flush` settles at once.
/// A document downgraded while dirty stays dirty and only `flush` hands it over.
#[test]
fn view_and_comment_send_nothing_to_the_shell() {
    for mode in READ_ONLY {
        let (mut app, fake) = fake_app();
        let id = open_as(&mut app, "d1", FORM.to_vec(), access(mode), false);
        assert_eq!(fake.take_calls(), ["opened(d1, true, None)"]);
        let why = refused_with(&app);
        command_refused(&mut app, "file.save", why);
        app.toast = None;
        app.save_active(crate::editing::SaveTarget::InPlace);
        assert_eq!(toast(&app), Some(refused_with(&app)), "a save prompt's Save too");
        for now in [0.0, 4000.0, 40_000.0] {
            tick(&mut app, &fake, now);
        }
        let done = Rc::new(Cell::new(false));
        let flag = done.clone();
        app.online.as_ref().unwrap().flush_handle().flush(Box::new(move || flag.set(true)));
        assert!(done.get(), "flush settles at once");
        assert!(fake.saves().is_empty() && fake.take_calls().is_empty(), "{mode:?}");
        assert!(!app.session.get(id).unwrap().dirty);

        // Edited, then switched before AutoSave ran: kept unsaved, no AutoSave nor Save, flush only.
        let (mut app, fake) = fake_app();
        let id = open_as(&mut app, "d2", FORM.to_vec(), Access::default(), false);
        tick(&mut app, &fake, 0.0);
        edit(&mut app, id, "Ada");
        tick(&mut app, &fake, 1000.0);
        set_mode(&mut app, &fake, "d2", access(mode));
        for now in [4000.0, 40_000.0] {
            tick(&mut app, &fake, now);
        }
        app.save_active(crate::editing::SaveTarget::InPlace);
        tick(&mut app, &fake, 41_000.0);
        assert!(fake.saves().is_empty(), "{mode:?}");
        assert!(app.session.get(id).unwrap().dirty, "the edits are never dropped");
        app.online.as_ref().unwrap().flush_handle().flush(Box::new(|| {}));
        let generation = app.session.get(id).unwrap().edit_generation();
        assert_eq!(fake.saves(), [("d2".to_string(), generation, SaveReason::Flush)], "{mode:?}");
    }
}

/// `setMode` down while a form field is being typed in: the typing is committed as one undo step
/// first, the document stays dirty, the shell's `flush` saves it once, and `saved` cleans it.
#[test]
fn a_downgrade_commits_open_typing_and_flush_saves_it() {
    for mode in READ_ONLY {
        let (mut app, fake) = fake_app();
        let id = open_as(&mut app, "d1", FORM.to_vec(), Access::default(), false);
        tick(&mut app, &fake, 0.0);
        fake.take_calls();
        let undo = app.session.get(id).unwrap().undo_history().len();
        app.views[0].forms.focus = Some(crate::forms_ui::Focus {
            name: "name".into(),
            widget: 0,
            text: "Ada".into(),
            picked: Vec::new(),
            request_focus: false,
            select_all: false,
            calendar: None,
            calendar_rect: None,
        });
        // The shell's downgrade sequence: flush (nothing committed yet), setMode, flush again.
        app.online.as_ref().unwrap().flush_handle().flush(Box::new(|| {}));
        assert!(fake.saves().is_empty());
        set_mode(&mut app, &fake, "d1", access(mode));
        let doc = app.session.get(id).unwrap();
        let field = doc.form.iter().find(|f| f.name == "name").unwrap();
        assert_eq!(field.value, ["Ada"], "{mode:?}");
        assert_eq!(doc.undo_history().len(), undo + 1, "one undo step");
        assert!(doc.dirty && app.views[0].forms.focus.is_none());
        assert_eq!(doc.limits(), pdfcraft_engine::SessionLimits { view_only: true, no_extract: false });
        assert_eq!(fake.take_calls(), ["dirty(d1, true)"]);
        let generation = doc.edit_generation();
        app.online.as_ref().unwrap().flush_handle().flush(Box::new(|| {}));
        assert_eq!(fake.saves(), [("d1".to_string(), generation, SaveReason::Flush)], "{mode:?}");
        fake.answer(SaveResult::Saved { version: Some(2) });
        tick(&mut app, &fake, 100.0);
        assert!(!app.session.get(id).unwrap().dirty, "saved as of the flushed revision");
        assert_eq!(fake.take_calls(), ["dirty(d1, false)"]);
        for now in [4000.0, 40_000.0] {
            tick(&mut app, &fake, now);
        }
        app.online.as_ref().unwrap().flush_handle().flush(Box::new(|| {}));
        assert_eq!(fake.saves().len(), 1, "no further saves: {mode:?}");
    }
}

/// A comment still being written when the document is downgraded is posted first, as its Post
/// button would, under the user's name.
#[test]
fn a_downgrade_posts_a_comment_being_written() {
    let (mut app, fake) = fake_app();
    let id = open_as(&mut app, "d1", pages(1), Access::default(), false);
    app.comment_prefs.author = "Lan".into();
    let before = app.session.get(id).unwrap().info.annotations.len();
    app.views[0].comments.composer = Some(crate::comments::Composer {
        page: 0,
        at: [100.0, 120.0],
        kind: crate::comments::ComposerKind::Note,
        text: "Xem lại".into(),
        focus: false,
    });
    set_mode(&mut app, &fake, "d1", access(AccessMode::Comment));
    let doc = app.session.get(id).unwrap();
    assert_eq!(doc.info.annotations.len(), before + 1);
    let note = doc.info.annotations.last().unwrap();
    assert_eq!((note.contents.as_deref(), note.author.as_deref()), (Some("Xem lại"), Some("Lan")));
    assert!(app.views[0].comments.composer.is_none() && doc.dirty);
}

/// `setMode` both ways on an open document: the same tab and document, the undo history intact,
/// editing and AutoSave back once it is `edit` again.
#[test]
fn set_mode_both_ways_keeps_the_document_and_its_undo_history() {
    for mode in READ_ONLY {
        let (mut app, fake) = fake_app();
        let id = open_as(&mut app, "d1", FORM.to_vec(), Access::default(), false);
        edit(&mut app, id, "Ada");
        edit(&mut app, id, "Grace");
        app.undo();
        let before = snapshot(&app, id);
        set_mode(&mut app, &fake, "d1", access(mode));
        assert!(!app.session.get(id).unwrap().editable());
        assert_eq!(snapshot(&app, id), before, "{mode:?}");
        set_mode(&mut app, &fake, "d1", Access::default());
        assert_eq!((app.views.len(), app.active_ids().map(|(_, d)| d)), (1, Some(id)), "never reopened");
        assert_eq!(snapshot(&app, id), before);
        app.redo();
        app.undo();
        app.undo();
        assert_eq!(app.session.get(id).unwrap().can_undo(), None, "both edits were still there to undo");
        edit(&mut app, id, "Hopper");
        tick(&mut app, &fake, 10_000.0);
        tick(&mut app, &fake, 13_000.0);
        assert_eq!(fake.saves().last().map(|s| s.2), Some(SaveReason::Auto), "AutoSave runs again");
    }
}

/// `setMode` reaches a document still waiting for its password, which then opens in that mode.
#[test]
fn set_mode_reaches_a_document_waiting_for_its_password() {
    let (mut app, fake) = fake_app();
    let bytes = super::tests::encrypted(FORM, "pw");
    let doc = HostDocument::new("d1".into(), Some(1.0), "Hợp đồng.pdf".into(), "pdf".into(), bytes);
    app.online.as_ref().unwrap().inbox().lock().unwrap().push(doc);
    app.open_host_documents();
    assert!(app.password_prompt.is_some());
    set_mode(&mut app, &fake, "d1", access(AccessMode::View));
    let pending = app.password_prompt.as_ref().and_then(|p| p.host_doc.as_ref()).unwrap();
    assert_eq!(pending.access, access(AccessMode::View));
}

/// `canDownload: false` disables Save As, Export, Print, Share and copying in every mode; editing
/// stays in `edit`, and `setMode` turns it back on.
#[test]
fn can_download_false_disables_save_as_export_print_and_copy() {
    let (mut app, fake) = fake_app();
    let no_download = Access { mode: AccessMode::Edit, can_download: false };
    let id = open_as(&mut app, "d1", FORM.to_vec(), no_download, false);
    // Share… is listed only where the shell can share; greyed all the same.
    assert!(!app.command_enabled(pdfcraft_engine::commands::command("file.share").unwrap()));
    let extracting = [
        "file.save_as",
        "print.dialog",
        "export.docx",
        "export.text",
        "export.image",
        "export.all_images",
        "edit.snapshot",
        "page.copy",
        "page.extract",
        "optimize.reduce",
    ];
    for command in extracting {
        command_refused(&mut app, command, no_extract_notice());
    }
    tool_refused(&mut app, QuickTool::Snapshot, no_extract_notice());
    app.toast = None;
    assert!(!app.may_copy(id), "selected text, Copy image and comment text alike");
    assert_eq!(toast(&app), Some(no_extract_notice()));
    edit(&mut app, id, "Ada");

    set_mode(&mut app, &fake, "d1", access(AccessMode::View));
    for command in ["file.save_as", "print.dialog", "export.docx", "edit.snapshot", "page.copy"] {
        let spec = pdfcraft_engine::commands::command(command).unwrap();
        assert!(app.command_enabled(spec), "{command}: viewers may download when the owner lets them");
    }
    assert!(app.may_copy(id));
    set_mode(&mut app, &fake, "d1", Access { mode: AccessMode::View, can_download: false });
    command_refused(&mut app, "file.save_as", no_extract_notice());
}

/// `preserveView`: a newer version takes the tab's place on the page and at the zoom the reader
/// had; without it the new version opens as usual, and a page the new version lacks is clamped.
#[test]
fn preserve_view_keeps_the_page_and_the_zoom() {
    let (mut app, _fake) = fake_app();
    let first = open_as(&mut app, "d1", pages(4), access(AccessMode::View), false);
    let i = app.active.unwrap();
    app.views[i].set_zoom(1.75);
    app.views[i].go_to_page(2);
    let second = open_as(&mut app, "d1", pages(5), access(AccessMode::View), true);
    assert_ne!(first, second);
    assert_eq!(app.views.len(), 1, "the same tab");
    let view = &app.views[app.active.unwrap()];
    assert_eq!((view.current, view.zoom, view.fit), (2, 1.75, Fit::None));
    assert!(view.back.is_empty(), "no Previous view to the first page");
    assert!(!app.session.get(second).unwrap().editable(), "and still in its mode");

    let i = app.active.unwrap();
    app.views[i].set_fit(Fit::Width);
    app.views[i].go_to_page(4);
    open_as(&mut app, "d1", pages(2), access(AccessMode::View), true);
    let view = &app.views[app.active.unwrap()];
    assert_eq!((view.current, view.fit), (1, Fit::Width), "the last page the new version has");

    open_as(&mut app, "d1", pages(3), access(AccessMode::View), false);
    let view = &app.views[app.active.unwrap()];
    assert_eq!(view.current, 0, "without preserveView a reload starts at the top");
}

/// `setUser`: the account's name authors new comments in `edit` mode while the settings keep the
/// local name; `setUser(null)` brings the local name back.
#[test]
fn set_user_authors_a_new_comment() {
    let (mut app, fake) = fake_app();
    app.comment_prefs.author = "Local".into();
    let id = open_as(&mut app, "d1", pages(1), Access::default(), false);
    let user = app.online.as_ref().unwrap().user();
    *user.lock().unwrap() = Some(Some("  Nguyễn Văn An ".into()));
    tick(&mut app, &fake, 0.0);
    assert_eq!(app.saved_author(), "Local", "the settings keep the local name");

    let highlight = |app: &mut PdfCraftApp| {
        let i = app.active.unwrap();
        let doc = app.session.get(id).unwrap();
        let mut renderer = pdfcraft_render::PageRenderer::new(doc.bytes.clone(), Default::default());
        let text = renderer.render(RenderRequest { page: 0, kind: RequestKind::Text, scale: 1.0, ..Default::default() }).text.unwrap();
        app.views[i].texts.insert(0, text);
        assert!(app.views[i].select_all());
        assert!(app.execute("comment.highlight"));
        app.session.get(id).unwrap().info.annotations.last().and_then(|a| a.author.clone())
    };
    assert_eq!(highlight(&mut app).as_deref(), Some("Nguyễn Văn An"));

    *user.lock().unwrap() = Some(None);
    tick(&mut app, &fake, 0.0);
    assert_eq!(highlight(&mut app).as_deref(), Some("Local"));
    assert_eq!(app.comment_prefs.author, "Local");
}

#[test]
fn modes_the_app_does_not_know_never_grant_editing() {
    assert_eq!(Access::new(None, None), Access { mode: AccessMode::Edit, can_download: true });
    assert_eq!(AccessMode::parse(Some("edit")), AccessMode::Edit);
    assert_eq!(AccessMode::parse(Some("comment")), AccessMode::Comment);
    assert_eq!(AccessMode::parse(Some("view")), AccessMode::View);
    for other in ["", "owner", "EDIT", "suggest"] {
        assert_eq!(AccessMode::parse(Some(other)), AccessMode::View, "{other:?}");
    }
    let limits = Access::new(Some("view"), Some(false)).limits();
    assert_eq!(limits, pdfcraft_engine::SessionLimits { view_only: true, no_extract: true });
}

/// Tool groups whose tools change the document, and ones that only view it or make a new file.
const EDITING_GROUPS: [&str; 11] =
    ["edit", "fill_sign", "organize", "comment", "scan", "protect", "redact", "form", "stamp", "certificate", "measure"];
const VIEWING_GROUPS: [&str; 5] = ["export", "create", "combine", "compress", "compare"];

fn group(id: &str) -> &'static pdfcraft_engine::catalog::ToolGroup {
    pdfcraft_engine::catalog::group(id).unwrap()
}

/// Rule 14: All tools greys the groups that would change the document, saying why in the mode's
/// words, and keeps the ones that only view it; downloading off greys the export tools instead.
#[test]
fn all_tools_greys_the_editing_tools_in_view_and_comment() {
    for mode in READ_ONLY {
        let (app, _fake, _id) = with_history(mode);
        let why = refused_with(&app);
        for id in EDITING_GROUPS {
            assert_eq!(app.tool_group_refusal(group(id)), Some(why), "{id} in {mode:?}");
        }
        for id in VIEWING_GROUPS {
            assert_eq!(app.tool_group_refusal(group(id)), None, "{id} in {mode:?}");
        }
    }
    let (mut app, fake) = fake_app();
    open_as(&mut app, "d1", FORM.to_vec(), Access::default(), false);
    assert!(pdfcraft_engine::catalog::TOOL_GROUPS.iter().all(|g| app.tool_group_refusal(g).is_none()), "editing offers every tool");
    set_mode(&mut app, &fake, "d1", Access { mode: AccessMode::Edit, can_download: false });
    assert_eq!(app.tool_group_refusal(group("export")), Some(no_extract_notice()));
    assert_eq!(app.tool_group_refusal(group("edit")), None);
}

/// The All tools panel itself: a refused group is a disabled row that opens nothing.
#[test]
fn all_tools_rows_of_editing_tools_are_disabled_in_view_mode() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let (mut app, fake) = fake_app();
    open_as(&mut app, "d1", FORM.to_vec(), access(AccessMode::View), false);
    tick(&mut app, &fake, 0.0);
    app.left_open = true;
    app.left = crate::LeftPanel::AllTools;
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(move |_| app);
    h.run_steps(3);
    let disabled = |h: &Harness<'_, PdfCraftApp>, label: &str| {
        let nodes: Vec<bool> = h.query_all_by_label(crate::i18n::t(label)).map(|n| n.accesskit_node().is_disabled()).collect();
        assert!(!nodes.is_empty(), "{label} is listed");
        nodes.contains(&true)
    };
    assert!(disabled(&h, "Edit a PDF") && disabled(&h, "Fill & Sign"));
    assert!(!disabled(&h, "Export a PDF"));
    h.query_all_by_label(crate::i18n::t("Edit a PDF")).find(|n| n.accesskit_node().is_disabled()).unwrap().click();
    h.run_steps(2);
    assert_eq!(h.state().left, crate::LeftPanel::AllTools, "a refused tool doesn't open");
}

/// Owner decision: inside the office shell (`erpHost` v2) its own Share is the only one; File ▸
/// Share… stays where there is no `erpHost` v2.
#[test]
fn file_share_is_hidden_inside_the_shell_with_erp_host_v2() {
    let spec = pdfcraft_engine::commands::command("file.share").unwrap();
    let share = || -> Option<crate::share::ShareFn> { Some(Box::new(|_: &[u8], _: &str, _: &str| true)) };
    let mut free = PdfCraftApp::new();
    free.share = share();
    assert!(free.command_shown(spec), "the free web build keeps it");
    let (mut v1, _fake) = fake_app();
    v1.share = share();
    assert!(v1.command_shown(spec), "erpHost v1 keeps it");
    let (mut app, _fake) = fake_app();
    app.online = app.online.take().map(Online::with_shell_share);
    app.share = share();
    open_as(&mut app, "d1", FORM.to_vec(), Access::default(), false);
    assert!(!app.command_shown(spec));
    assert!(!app.execute("file.share"));
}

/// Owner decision: comment mode stays view-only on the web for now and says so in its own words;
/// view mode keeps its notice.
#[test]
fn comment_mode_says_commenting_comes_later() {
    use egui_kittest::kittest::Queryable;
    let vi = crate::i18n::Lang::from_code("vi").unwrap();
    assert_eq!(
        crate::i18n::tr(vi, "Commenting on the web is coming later; for now this document can only be viewed."),
        "Nhận xét trên web sẽ có sau, hiện chỉ xem được"
    );
    assert_eq!(crate::i18n::tr(vi, "This document is open for viewing only."), "Tài liệu này chỉ được mở để xem.");
    for (mode, notice) in [(AccessMode::Comment, comment_only_notice as fn() -> &'static str), (AccessMode::View, view_only_notice)] {
        let (mut app, fake) = fake_app();
        let id = open_as(&mut app, "d1", FORM.to_vec(), access(mode), false);
        tick(&mut app, &fake, 0.0);
        assert_eq!(app.read_only_notice(id), notice());
        command_refused(&mut app, "comment.note", notice());
        edit_refused(&mut app, note());
        // The toast says the same; only the notice bar should be found.
        app.toast = None;
        let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(move |_| app);
        h.run_steps(3);
        assert!(h.query_by_label(notice()).is_some(), "the notice bar says it in {mode:?}");
    }
}

/// A downgrade applies a paragraph being retyped (Edit text) and text being added (Add text), each
/// as one undo step, before the switch.
#[test]
fn a_downgrade_applies_text_being_edited_and_added() {
    let (mut app, fake) = fake_app();
    let bytes = app.session.create_from_text("Doc", "Original line").unwrap();
    let id = open_as(&mut app, "d1", bytes.as_ref().clone(), Access::default(), false);
    let doc = app.session.get(id).unwrap();
    let block = doc.text_blocks(0).iter().position(|b| b.text.contains("Original line")).unwrap();
    let undo = doc.undo_history().len();
    app.views[0].line_editor = Some(crate::edit_text_ui::LineEditor::retyped(doc, 0, block, "line Original line"));
    set_mode(&mut app, &fake, "d1", access(AccessMode::View));
    let doc = app.session.get(id).unwrap();
    assert!(doc.text_blocks(0).iter().any(|b| b.text.contains("line Original line")), "the retyped paragraph is in the document");
    assert_eq!(doc.undo_history().len(), undo + 1);
    assert!(app.views[0].line_editor.is_none() && doc.dirty && doc.limits().view_only);

    let (mut app, fake) = fake_app();
    let id = open_as(&mut app, "d2", pages(1), Access::default(), false);
    let undo = app.session.get(id).unwrap().undo_history().len();
    app.views[0].content.draft = Some(crate::content_ui::TextDraft {
        page: 0,
        index: None,
        rect: [20.0, 20.0, 160.0, 40.0],
        text: "Added line".into(),
        style: crate::content_ui::default_style(),
        focus: false,
    });
    set_mode(&mut app, &fake, "d2", access(AccessMode::Comment));
    let doc = app.session.get(id).unwrap();
    assert!(doc.added.iter().any(|a| matches!(&a.content, pdfcraft_engine::AddedContent::Text(t) if t.text == "Added line")));
    assert_eq!(doc.undo_history().len(), undo + 1);
    assert!(app.views[0].content.draft.is_none() && doc.dirty && doc.limits().view_only);
}
