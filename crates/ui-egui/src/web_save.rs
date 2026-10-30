//! Saving on the web, where the browser writes the file: through its save picker (the File System
//! Access API: Chrome, Edge) where there is one, else as a download named in an in-app prompt.
//!
//! A document is marked saved only when the browser reports the file written, and only as it was
//! written: a cancelled picker or a failed write leaves it unsaved, and so do edits made while
//! the picker was open. The browser side is a [`BrowserSave`] (`web_save_browser` on wasm, a fake
//! in tests), so all of this runs natively.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;
use crate::editing::SaveTarget;
use crate::save_deadline::SaveDeadline;

/// A file the browser's picker chose, which later saves write to again.
pub type FileId = u64;

/// How one browser save ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Written under `name`, the name the user picked. `file` is where Save writes next; `None`
    /// for a download.
    Saved {
        name: String,
        file: Option<FileId>,
    },
    /// The user closed the picker.
    Cancelled,
    Failed(String),
}

/// Reports how a save ended, possibly on a later frame.
pub type SaveDone = Box<dyn FnOnce(SaveOutcome)>;

/// The browser's ways of writing a file.
pub trait BrowserSave {
    /// The browser can show its own save picker; without one the app asks for the name itself.
    fn has_picker(&self) -> bool;
    /// Show the save picker suggesting `name`, then write `bytes` to the chosen file.
    fn pick_and_write(&self, name: &str, bytes: &[u8], done: SaveDone);
    /// Write `bytes` to `file` again, without asking; `name` is its name as last known.
    fn write_to(&self, file: FileId, name: &str, bytes: &[u8], done: SaveDone);
    /// Offer `bytes` as a download named `name`. The browser never says whether it finished.
    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String>;
}

/// What runs once the save it waits for is written (closing the tab after Save).
type After = Box<dyn FnOnce(&mut PdfCraftApp) + Send>;

/// Where Save writes a document again without asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
    Picked(FileId),
    /// Downloaded once under its current name: Save downloads it again under that name.
    Downloaded,
    TimedOut,
}

/// A save handed to the browser that hasn't ended yet.
pub(crate) struct Pending {
    token: u64,
    deadline: SaveDeadline,
    pub(crate) doc: DocId,
    /// The document's edit generation when `bytes` were taken.
    generation: u64,
    bytes: Arc<Vec<u8>>,
    pub(crate) name: String,
    download: bool,
    after: After,
}

/// The fallback's "Save as" prompt (no picker): the file name the download gets.
pub struct NamePrompt {
    pub doc: DocId,
    pub name: String,
    /// The name field takes the keyboard when the prompt first shows.
    pub(crate) focus: bool,
    after: After,
}

/// Browser saves in flight, their outcomes, and where each document saves next.
pub struct WebSaves {
    browser: Box<dyn BrowserSave>,
    pending: Vec<Pending>,
    results: Rc<RefCell<Vec<(u64, f64, SaveOutcome)>>>,
    clock: Rc<dyn Fn() -> f64>,
    pub(crate) timed_out: Vec<Pending>,
    next_token: u64,
    destinations: HashMap<DocId, Destination>,
    pub(crate) prompt: Option<NamePrompt>,
    /// Save all without a picker: download under the document's name instead of asking for one.
    download_unnamed: bool,
}

impl WebSaves {
    pub fn new(browser: Box<dyn BrowserSave>) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let clock: Rc<dyn Fn() -> f64> = {
            let started = std::time::Instant::now();
            Rc::new(move || started.elapsed().as_secs_f64())
        };
        #[cfg(target_arch = "wasm32")]
        let clock: Rc<dyn Fn() -> f64> = Rc::new(|| js_sys::Date::now() / 1000.0);
        Self::with_clock(browser, clock)
    }

    fn with_clock(browser: Box<dyn BrowserSave>, clock: Rc<dyn Fn() -> f64>) -> Self {
        Self {
            browser,
            pending: vec![],
            results: Rc::default(),
            clock,
            timed_out: vec![],
            next_token: 0,
            destinations: HashMap::new(),
            prompt: None,
            download_unnamed: false,
        }
    }

    /// The fallback prompt is asking for a file name.
    pub fn prompt(&self) -> Option<&NamePrompt> {
        self.prompt.as_ref()
    }

    /// The browser shows its own save picker.
    pub fn has_picker(&self) -> bool {
        self.browser.has_picker()
    }

    /// A save of `doc` is waiting for the browser.
    pub fn is_pending(&self, doc: DocId) -> bool {
        self.pending.iter().any(|p| p.doc == doc)
    }

    /// Offer `bytes` as a download, outside the save bookkeeping (an export such as Share…).
    pub(crate) fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.browser.download(name, bytes)
    }
}

/// `name` as a PDF file name: trimmed, with `.pdf` added when it lacks it; `None` when empty.
pub(crate) fn pdf_file_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(".pdf") {
        return None;
    }
    Some(if name.to_ascii_lowercase().ends_with(".pdf") { name.to_string() } else { format!("{name}.pdf") })
}

impl PdfCraftApp {
    /// Save document `id` through the browser. Returns `true` if it was written now; a picker or
    /// the name prompt answers on a later frame, and `after` runs only once the save is written.
    pub(crate) fn web_save_then(&mut self, id: DocId, target: SaveTarget, after: After) -> bool {
        let Some(saves) = self.web_save.as_ref() else { return false };
        // One save of a document at a time: a second picker would be refused while one shows.
        if saves.prompt.is_some() || saves.is_pending(id) {
            return false;
        }
        let Some(name) = self.session.get(id).map(|d| pdf_file_name(&d.name).unwrap_or_else(|| "document.pdf".to_string())) else {
            return false;
        };
        let previous = saves.destinations.get(&id).copied();
        let destination = if target == SaveTarget::InPlace || previous == Some(Destination::TimedOut) { previous } else { None };
        let has_picker = saves.browser.has_picker();
        match destination {
            Some(Destination::Downloaded | Destination::TimedOut) => self.start_web_save(id, name, Write::Download, after),
            Some(Destination::Picked(file)) => self.start_web_save(id, name, Write::To(file), after),
            None if has_picker => self.start_web_save(id, name, Write::Pick, after),
            None if saves.download_unnamed => self.start_web_save(id, name, Write::Download, after),
            None => {
                if let Some(saves) = self.web_save.as_mut() {
                    saves.prompt = Some(NamePrompt { doc: id, name, focus: true, after });
                }
                false
            }
        }
    }

    /// While set, a document never saved downloads under its name rather than asking for one.
    pub(crate) fn set_download_unnamed(&mut self, on: bool) {
        if let Some(saves) = self.web_save.as_mut() {
            saves.download_unnamed = on;
        }
    }

    /// The name prompt's Save: download under the name typed. Returns `false` (and keeps asking)
    /// when the name is empty.
    pub(crate) fn confirm_save_name(&mut self) -> bool {
        let Some(saves) = self.web_save.as_mut() else { return false };
        let Some(name) = saves.prompt.as_ref().and_then(|p| pdf_file_name(&p.name)) else { return false };
        let Some(prompt) = saves.prompt.take() else { return false };
        self.start_web_save(prompt.doc, name, Write::Download, prompt.after);
        true
    }

    /// The name prompt's Cancel: nothing is saved.
    pub(crate) fn cancel_save_name(&mut self) {
        if let Some(saves) = self.web_save.as_mut() {
            saves.prompt = None;
        }
    }

    fn start_web_save(&mut self, id: DocId, name: String, write: Write, after: After) -> bool {
        if !self.flatten_fill_sign_for_save(id) {
            return false;
        }
        if !self.signature_write_confirmed(id) {
            self.hold_breaking_save(id, SaveTarget::As, after);
            return false;
        }
        let bytes = match self.session.save_copy_bytes(id) {
            Ok(b) => b,
            Err(e) => {
                self.save_failed(id, "Couldn't save {name}: {e}", &[("name", &name), ("e", &e.to_string())]);
                return false;
            }
        };
        let Some(generation) = self.session.get(id).map(|d| d.edit_generation()) else { return false };
        let ctx = self.ctx.clone();
        let Some(saves) = self.web_save.as_mut() else { return false };
        saves.next_token = saves.next_token.wrapping_add(1);
        let token = saves.next_token;
        let results = saves.results.clone();
        let clock = saves.clock.clone();
        let deadline = SaveDeadline::new(clock());
        let done: SaveDone = Box::new(move |outcome| {
            if let Ok(mut r) = results.try_borrow_mut() {
                r.push((token, clock(), outcome));
            }
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        let download = matches!(write, Write::Download);
        saves.pending.push(Pending { token, deadline, doc: id, generation, bytes: bytes.clone(), name: name.clone(), download, after });
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint_after(crate::save_deadline::TIMEOUT);
        }
        match write {
            Write::Pick => saves.browser.pick_and_write(&name, &bytes, done),
            Write::To(file) => saves.browser.write_to(file, &name, &bytes, done),
            // The browser never reports a download's end: counting it saved once handed over keeps
            // the tab from warning forever about work the user just downloaded.
            Write::Download => done(match saves.browser.download(&name, &bytes) {
                Ok(()) => SaveOutcome::Saved { name: name.clone(), file: None },
                Err(e) => SaveOutcome::Failed(e),
            }),
        }
        self.settle_web_saves().contains(&token)
    }

    /// Apply how the browser's saves ended (every frame). Returns the tokens that were written.
    pub(crate) fn settle_web_saves(&mut self) -> Vec<u64> {
        let Some(saves) = self.web_save.as_mut() else { return vec![] };
        let results = match saves.results.try_borrow_mut() {
            Ok(mut r) => std::mem::take(&mut *r),
            Err(_) => return vec![],
        };
        let open: Vec<DocId> = self.views.iter().map(|v| v.id).collect();
        saves.destinations.retain(|id, _| open.contains(id));
        saves.timed_out.retain(|p| open.contains(&p.doc));
        saves.pending.retain(|p| open.contains(&p.doc));
        let mut written = vec![];
        for (token, completed_at, outcome) in results {
            let Some(saves) = self.web_save.as_mut() else { break };
            let Some(i) = saves.pending.iter().position(|p| p.token == token) else { continue };
            let p = saves.pending.remove(i);
            if p.deadline.expired(completed_at) {
                self.timeout_web_save(p);
                continue;
            }
            match outcome {
                SaveOutcome::Saved { name, file } => {
                    written.push(token);
                    self.finish_web_save(p, name, file);
                }
                SaveOutcome::Cancelled => {}
                SaveOutcome::Failed(e) if p.download => {
                    self.save_failed(p.doc, "Couldn't download {name}: {e}", &[("name", &p.name), ("e", &e)]);
                }
                SaveOutcome::Failed(e) => self.save_failed(p.doc, "Couldn't save {name}: {e}", &[("name", &p.name), ("e", &e)]),
            }
        }
        let expired = if let Some(saves) = self.web_save.as_mut() {
            let now = (saves.clock)();
            let mut expired = vec![];
            let mut i = 0;
            while i < saves.pending.len() {
                if saves.pending.get(i).is_some_and(|p| p.deadline.expired(now)) {
                    expired.push(saves.pending.remove(i));
                } else {
                    i += 1;
                }
            }
            expired
        } else {
            vec![]
        };
        for pending in expired {
            self.timeout_web_save(pending);
        }
        written
    }

    fn timeout_web_save(&mut self, pending: Pending) {
        self.save_failed(pending.doc, "Save failed", &[]);
        if let Some(saves) = self.web_save.as_mut() {
            // Ignore late callbacks and download future saves: the original write may still finish.
            saves.destinations.insert(pending.doc, Destination::TimedOut);
            saves.timed_out.retain(|p| p.doc != pending.doc);
            saves.timed_out.push(pending);
        }
    }

    pub(crate) fn download_timed_out_save(&mut self, id: DocId) {
        let result = self.web_save.as_ref().and_then(|saves| {
            let pending = saves.timed_out.iter().find(|p| p.doc == id)?;
            Some((pending.name.clone(), saves.browser.download(&pending.name, &pending.bytes)))
        });
        match result {
            Some((name, Ok(()))) => {
                if let Some(saves) = self.web_save.as_mut() {
                    saves.timed_out.retain(|p| p.doc != id);
                }
                self.notify_fmt("Downloaded {name}", &[("name", &name)]);
            }
            Some((name, Err(e))) => self.save_failed(id, "Couldn't download {name}: {e}", &[("name", &name), ("e", &e)]),
            None => {}
        }
    }

    /// The file is written: adopt the name the user picked, and mark the document saved if it is
    /// still as it was written (later edits stay unsaved).
    fn finish_web_save(&mut self, p: Pending, name: String, file: Option<FileId>) {
        let Some(doc) = self.session.get(p.doc) else { return };
        let name = if name.trim().is_empty() { p.name } else { name };
        let unchanged = doc.edit_generation() == p.generation;
        // Save As of a server document writes a local copy: it stays the server's document, as
        // unsaved as before and under its own name (erp-host rule 9).
        if self.is_host_doc(p.doc) {
            self.notify_fmt(if p.download { "Downloaded {name}" } else { "Saved {name}" }, &[("name", &name)]);
            if unchanged {
                (p.after)(self);
            }
            return;
        }
        if unchanged && let Err(e) = self.session.mark_saved(p.doc, p.bytes, None) {
            self.save_failed(p.doc, "Saved, but reopening failed: {e}", &[("e", &e.to_string())]);
            return;
        }
        let _ = self.session.rename(p.doc, name.clone());
        if let Some(saves) = self.web_save.as_mut() {
            saves.timed_out.retain(|pending| pending.doc != p.doc);
            if saves.destinations.get(&p.doc) != Some(&Destination::TimedOut) {
                saves.destinations.insert(p.doc, file.map_or(Destination::Downloaded, Destination::Picked));
            }
        }
        if let Some(doc) = self.session.get(p.doc)
            && let Some(view) = self.views.iter_mut().find(|v| v.id == p.doc)
        {
            view.document_changed(&doc.info);
        }
        if p.download {
            self.notify_fmt("Downloaded {name}", &[("name", &name)]);
        } else {
            self.notify_fmt("Saved {name}", &[("name", &name)]);
        }
        if unchanged {
            (p.after)(self);
        }
    }
}

enum Write {
    Pick,
    To(FileId),
    Download,
}

#[cfg(test)]
#[path = "web_save_timeout_tests.rs"]
mod timeout_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_engine::{Edit, FieldValue};

    const FORM: &[u8] = include_bytes!("../tests/data/form.pdf");

    #[derive(Default)]
    struct Log {
        picker: bool,
        calls: Vec<String>,
        waiting: Vec<SaveDone>,
        downloads: Vec<Vec<u8>>,
        download_error: Option<String>,
    }

    #[derive(Clone, Default)]
    pub(super) struct Fake(Rc<RefCell<Log>>);

    impl BrowserSave for Fake {
        fn has_picker(&self) -> bool {
            self.0.borrow().picker
        }
        fn pick_and_write(&self, name: &str, _: &[u8], done: SaveDone) {
            let mut log = self.0.borrow_mut();
            log.calls.push(format!("Pick({name:?})"));
            log.waiting.push(done);
        }
        fn write_to(&self, file: FileId, _: &str, _: &[u8], done: SaveDone) {
            let mut log = self.0.borrow_mut();
            log.calls.push(format!("WriteTo({file})"));
            log.waiting.push(done);
        }
        fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
            let mut log = self.0.borrow_mut();
            log.calls.push(format!("Download({name:?})"));
            log.downloads.push(bytes.to_vec());
            log.download_error.clone().map_or(Ok(()), Err)
        }
    }

    impl Fake {
        /// The browser answers the oldest waiting save.
        pub(super) fn answer(&self, outcome: SaveOutcome) {
            let done = self.0.borrow_mut().waiting.remove(0);
            done(outcome);
        }
        pub(super) fn calls(&self) -> String {
            format!("[{}]", self.0.borrow().calls.join(", "))
        }
        pub(super) fn downloads(&self) -> Vec<Vec<u8>> {
            self.0.borrow().downloads.clone()
        }
        pub(super) fn fail_download(&self) {
            self.0.borrow_mut().download_error = Some("NotAllowedError".into());
        }
    }

    pub(super) fn app(picker: bool) -> (PdfCraftApp, Fake, DocId) {
        let fake = Fake::default();
        fake.0.borrow_mut().picker = picker;
        let mut app = PdfCraftApp::new();
        app.web_save = Some(WebSaves::new(Box::new(fake.clone())));
        app.open_bytes("Report.pdf", None, FORM.to_vec()).unwrap();
        let id = app.views[0].id;
        (app, fake, id)
    }

    pub(super) fn edit(app: &mut PdfCraftApp, value: &str) {
        assert!(app.apply_edit(Edit::SetFieldValue { name: "name".into(), value: FieldValue::Text(value.into()) }));
    }

    pub(super) fn dirty(app: &PdfCraftApp, id: DocId) -> bool {
        app.session.get(id).unwrap().dirty
    }

    #[test]
    fn a_cancelled_picker_leaves_the_document_unsaved() {
        let (mut app, fake, id) = app(true);
        edit(&mut app, "Ada");
        assert!(!app.save_active(SaveTarget::InPlace), "the picker answers later");
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf")]"#);
        assert!(dirty(&app, id), "unsaved while the picker shows");
        fake.answer(SaveOutcome::Cancelled);
        app.settle_web_saves();
        assert!(dirty(&app, id));
        assert!(app.has_unsaved_work(0));
        assert!(!app.web_save.as_ref().unwrap().is_pending(id));
        // Nothing remembered: the next Save asks again.
        app.save_active(SaveTarget::InPlace);
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), Pick("Report.pdf")]"#);
    }

    #[test]
    fn a_failed_write_leaves_the_document_unsaved_and_says_why() {
        let (mut app, fake, id) = app(true);
        edit(&mut app, "Ada");
        app.save_active(SaveTarget::InPlace);
        fake.answer(SaveOutcome::Failed("NotAllowedError: disk full".into()));
        app.settle_web_saves();
        assert!(dirty(&app, id));
        assert_eq!(app.session.get(id).unwrap().name, "Report.pdf");
        let notice = app.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
        assert!(notice.contains("Couldn't save Report.pdf") && notice.contains("disk full"), "{notice}");
    }

    #[test]
    fn edits_made_while_the_picker_shows_stay_unsaved() {
        let (mut app, fake, id) = app(true);
        edit(&mut app, "Ada");
        app.save_active(SaveTarget::InPlace);
        edit(&mut app, "Grace");
        let after = app.session.get(id).unwrap().bytes.clone();
        fake.answer(SaveOutcome::Saved { name: "Report.pdf".into(), file: Some(7) });
        app.settle_web_saves();
        let doc = app.session.get(id).unwrap();
        assert!(doc.dirty, "Grace was not in the written file");
        assert_eq!(doc.bytes, after, "the newer edit is kept");
        // The written file is still where Save goes next.
        app.save_active(SaveTarget::InPlace);
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), WriteTo(7)]"#);
        fake.answer(SaveOutcome::Saved { name: "Report.pdf".into(), file: Some(7) });
        app.settle_web_saves();
        assert!(!dirty(&app, id));
    }

    #[test]
    fn save_as_adopts_the_picked_name_for_the_title_tab_and_next_save() {
        let (mut app, fake, id) = app(true);
        edit(&mut app, "Ada");
        app.save_active(SaveTarget::As);
        fake.answer(SaveOutcome::Saved { name: "Contract v2.pdf".into(), file: Some(3) });
        app.settle_web_saves();
        let doc = app.session.get(id).unwrap();
        assert!(!doc.dirty);
        assert_eq!(doc.name, "Contract v2.pdf");
        assert_eq!(doc.display_name(), "Contract v2.pdf", "the window title and tab show it");
        edit(&mut app, "Grace");
        app.save_active(SaveTarget::InPlace);
        fake.answer(SaveOutcome::Saved { name: "Contract v2.pdf".into(), file: Some(3) });
        app.settle_web_saves();
        assert!(!dirty(&app, id));
        // Save As always asks, suggesting the current name.
        app.save_active(SaveTarget::As);
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), WriteTo(3), Pick("Contract v2.pdf")]"#);
    }

    #[test]
    fn closing_after_save_waits_for_the_written_file() {
        let (mut app, fake, _) = app(true);
        edit(&mut app, "Ada");
        let ctx = egui::Context::default();
        app.request_close_tab(0);
        app.resolve_close(&ctx, Some(true));
        assert_eq!(app.views.len(), 1, "still open while the picker shows");
        fake.answer(SaveOutcome::Cancelled);
        app.settle_web_saves();
        assert_eq!(app.views.len(), 1, "a cancelled save closes nothing");
        app.request_close_tab(0);
        app.resolve_close(&ctx, Some(true));
        fake.answer(SaveOutcome::Saved { name: "Report.pdf".into(), file: Some(1) });
        app.settle_web_saves();
        assert!(app.views.is_empty());
    }

    #[test]
    fn the_tab_says_saving_while_the_picker_shows_then_failed_until_the_next_edit() {
        let (mut app, fake, id) = app(true);
        edit(&mut app, "Ada");
        assert_eq!(app.save_state(id), None);
        app.save_active(SaveTarget::InPlace);
        assert_eq!(app.save_state(id), Some(crate::SaveState::Saving));
        fake.answer(SaveOutcome::Failed("NotAllowedError: disk full".into()));
        app.settle_web_saves();
        let Some(crate::SaveState::Failed(why)) = app.save_state(id) else { panic!("{:?}", app.save_state(id)) };
        assert!(why.contains("disk full"), "the reason stays reachable: {why}");
        // Another Save shows Saving again; the failure is over once the document changes.
        app.save_active(SaveTarget::InPlace);
        assert_eq!(app.save_state(id), Some(crate::SaveState::Saving));
        fake.answer(SaveOutcome::Cancelled);
        app.settle_web_saves();
        assert!(matches!(app.save_state(id), Some(crate::SaveState::Failed(_))), "a cancelled retry is no success");
        edit(&mut app, "Grace");
        assert_eq!(app.save_state(id), None);
    }

    #[test]
    fn a_successful_save_ends_the_failed_state() {
        let (mut app, fake, id) = app(true);
        edit(&mut app, "Ada");
        app.save_active(SaveTarget::InPlace);
        fake.answer(SaveOutcome::Failed("NotAllowedError".into()));
        app.settle_web_saves();
        assert!(matches!(app.save_state(id), Some(crate::SaveState::Failed(_))));
        app.save_active(SaveTarget::InPlace);
        fake.answer(SaveOutcome::Saved { name: "Report.pdf".into(), file: Some(2) });
        app.settle_web_saves();
        assert_eq!(app.save_state(id), None);
        assert!(!dirty(&app, id));
    }

    /// Three edited documents, `Report.pdf`, `Second.pdf` and `Third.pdf`, with Close all asking.
    fn three_unsaved(picker: bool) -> (PdfCraftApp, Fake) {
        let (mut app, fake, _) = app(picker);
        edit(&mut app, "Ada");
        for name in ["Second.pdf", "Third.pdf"] {
            app.open_bytes(name, None, FORM.to_vec()).unwrap();
            edit(&mut app, "Grace");
        }
        app.close_all();
        assert_eq!(app.unsaved_tabs(), [0, 1, 2]);
        (app, fake)
    }

    fn rows(app: &PdfCraftApp) -> Vec<(String, crate::save_all::Row)> {
        app.save_all.as_ref().map(|r| r.rows()).unwrap_or_default()
    }

    /// Save all on the web: the click on Save all opens the first picker; each later picker waits
    /// for its own click on "Save “<next>”", since the browser refuses one opened without a click.
    #[test]
    fn save_all_takes_a_click_for_each_picker() {
        use crate::save_all::Row::{Saved, Saving, Unsaved};
        let (mut app, fake) = three_unsaved(true);
        let ctx = egui::Context::default();
        app.resolve_close(&ctx, Some(true));
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf")]"#, "only the first picker shows");
        assert_eq!(rows(&app).iter().map(|r| r.1).collect::<Vec<_>>(), [Saving, Unsaved, Unsaved]);
        assert!(app.modal_open());
        fake.answer(SaveOutcome::Saved { name: "Report.pdf".into(), file: Some(1) });
        app.settle_web_saves();
        app.poll_save_all();
        assert_eq!(app.views.len(), 2, "the first is written and closed");
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf")]"#, "the next picker waits for a click");
        assert_eq!(rows(&app).iter().map(|r| r.1).collect::<Vec<_>>(), [Saved, Unsaved, Unsaved]);
        assert_eq!(app.save_all_next().as_deref(), Some("Second.pdf"));
        // While a picker is open the prompt takes no answer.
        app.answer_save_all(&ctx, Some(true));
        app.answer_save_all(&ctx, None);
        assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), Pick("Second.pdf")]"#);
        assert!(app.save_all.is_some(), "Cancel waits for the open picker");
        fake.answer(SaveOutcome::Saved { name: "Second.pdf".into(), file: Some(2) });
        app.settle_web_saves();
        app.answer_save_all(&ctx, Some(true));
        fake.answer(SaveOutcome::Saved { name: "Third.pdf".into(), file: Some(3) });
        app.settle_web_saves();
        app.poll_save_all();
        assert!(app.views.is_empty());
        assert!(app.save_all.is_none(), "done");
        assert!(!app.modal_open());
    }

    /// A cancelled picker, or a failed write, stops Save all: what is still unsaved stays open.
    #[test]
    fn a_cancelled_or_failed_picker_stops_save_all_and_closes_nothing_unsaved() {
        for outcome in [SaveOutcome::Cancelled, SaveOutcome::Failed("NotAllowedError".into())] {
            let (mut app, fake) = three_unsaved(true);
            let ctx = egui::Context::default();
            app.resolve_close(&ctx, Some(true));
            fake.answer(SaveOutcome::Saved { name: "Report.pdf".into(), file: Some(1) });
            app.settle_web_saves();
            app.answer_save_all(&ctx, Some(true));
            fake.answer(outcome.clone());
            app.settle_web_saves();
            app.poll_save_all();
            assert!(app.save_all.is_none(), "{outcome:?} ends the run");
            assert_eq!(fake.calls(), r#"[Pick("Report.pdf"), Pick("Second.pdf")]"#, "Third.pdf is not asked about");
            assert_eq!(app.views.len(), 2);
            assert_eq!(app.unsaved_tabs(), [0, 1], "nothing still unsaved was closed");
        }
    }

    /// Don't save in the middle of Save all closes the rest unsaved; they can be reopened.
    #[test]
    fn dont_save_in_the_middle_of_save_all_closes_the_rest() {
        let (mut app, fake) = three_unsaved(true);
        let ctx = egui::Context::default();
        app.resolve_close(&ctx, Some(true));
        fake.answer(SaveOutcome::Saved { name: "Report.pdf".into(), file: Some(1) });
        app.settle_web_saves();
        app.answer_save_all(&ctx, Some(false));
        assert!(app.views.is_empty() && app.save_all.is_none());
        assert!(app.can_reopen_closed());
    }

    /// Without a picker, Save all downloads every document under its name on the one click.
    #[test]
    fn without_a_picker_save_all_downloads_them_all_at_once() {
        let (mut app, fake) = three_unsaved(false);
        app.resolve_close(&egui::Context::default(), Some(true));
        assert_eq!(fake.calls(), r#"[Download("Report.pdf"), Download("Second.pdf"), Download("Third.pdf")]"#);
        assert!(app.views.is_empty());
        assert!(app.web_save.as_ref().unwrap().prompt().is_none(), "no name is asked for");
        // A later Save of a new document asks for its name again.
        app.open_bytes("Fourth.pdf", None, FORM.to_vec()).unwrap();
        edit(&mut app, "Ada");
        app.save_active(SaveTarget::InPlace);
        assert!(app.web_save.as_ref().unwrap().prompt().is_some());
    }

    #[test]
    fn without_a_picker_save_asks_for_the_name_then_downloads() {
        let (mut app, fake, id) = app(false);
        edit(&mut app, "Ada");
        assert!(!app.save_active(SaveTarget::InPlace), "a never-saved document asks first");
        assert_eq!(app.web_save.as_ref().unwrap().prompt().map(|p| p.name.as_str()), Some("Report.pdf"));
        assert!(app.modal_open());
        assert_eq!(fake.calls(), "[]");
        app.cancel_save_name();
        assert!(dirty(&app, id), "cancelling the prompt saves nothing");

        app.save_active(SaveTarget::InPlace);
        app.web_save.as_mut().unwrap().prompt.as_mut().unwrap().name = "  ".into();
        assert!(!app.confirm_save_name(), "an empty name keeps asking");
        app.web_save.as_mut().unwrap().prompt.as_mut().unwrap().name = " Minutes ".into();
        assert!(app.confirm_save_name());
        assert_eq!(fake.calls(), r#"[Download("Minutes.pdf")]"#, ".pdf is kept");
        let doc = app.session.get(id).unwrap();
        assert!(!doc.dirty);
        assert_eq!(doc.name, "Minutes.pdf");
        // Downloaded once: Save downloads under that name again; Save As asks again.
        edit(&mut app, "Grace");
        assert!(app.save_active(SaveTarget::InPlace));
        assert_eq!(fake.calls(), r#"[Download("Minutes.pdf"), Download("Minutes.pdf")]"#);
        app.save_active(SaveTarget::As);
        assert_eq!(app.web_save.as_ref().unwrap().prompt().map(|p| p.name.as_str()), Some("Minutes.pdf"));
    }

    #[test]
    fn pdf_file_names() {
        assert_eq!(pdf_file_name("a"), Some("a.pdf".into()));
        assert_eq!(pdf_file_name(" a.PDF "), Some("a.PDF".into()));
        assert_eq!(pdf_file_name("notes.txt"), Some("notes.txt.pdf".into()));
        assert_eq!(pdf_file_name(""), None);
        assert_eq!(pdf_file_name(".pdf"), None);
    }
}
