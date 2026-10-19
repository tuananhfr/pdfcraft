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
}

/// A save handed to the browser that hasn't ended yet.
struct Pending {
    token: u64,
    doc: DocId,
    /// The document's edit generation when `bytes` were taken.
    generation: u64,
    bytes: Arc<Vec<u8>>,
    name: String,
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
    results: Rc<RefCell<Vec<(u64, SaveOutcome)>>>,
    next_token: u64,
    destinations: HashMap<DocId, Destination>,
    pub(crate) prompt: Option<NamePrompt>,
}

impl WebSaves {
    pub fn new(browser: Box<dyn BrowserSave>) -> Self {
        Self { browser, pending: vec![], results: Rc::default(), next_token: 0, destinations: HashMap::new(), prompt: None }
    }

    /// The fallback prompt is asking for a file name.
    pub fn prompt(&self) -> Option<&NamePrompt> {
        self.prompt.as_ref()
    }

    /// A save of `doc` is waiting for the browser.
    pub fn is_pending(&self, doc: DocId) -> bool {
        self.pending.iter().any(|p| p.doc == doc)
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
        let destination = if target == SaveTarget::InPlace { saves.destinations.get(&id).copied() } else { None };
        let has_picker = saves.browser.has_picker();
        match destination {
            Some(Destination::Downloaded) => self.start_web_save(id, name, Write::Download, after),
            Some(Destination::Picked(file)) => self.start_web_save(id, name, Write::To(file), after),
            None if has_picker => self.start_web_save(id, name, Write::Pick, after),
            None => {
                if let Some(saves) = self.web_save.as_mut() {
                    saves.prompt = Some(NamePrompt { doc: id, name, focus: true, after });
                }
                false
            }
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
        let bytes = match self.session.save_bytes(id) {
            Ok(b) => b,
            Err(e) => {
                self.notify_fmt("Couldn't save {name}: {e}", &[("name", &name), ("e", &e.to_string())]);
                return false;
            }
        };
        let Some(generation) = self.session.get(id).map(|d| d.edit_generation()) else { return false };
        let ctx = self.ctx.clone();
        let Some(saves) = self.web_save.as_mut() else { return false };
        saves.next_token = saves.next_token.wrapping_add(1);
        let token = saves.next_token;
        let results = saves.results.clone();
        let done: SaveDone = Box::new(move |outcome| {
            if let Ok(mut r) = results.try_borrow_mut() {
                r.push((token, outcome));
            }
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        let download = matches!(write, Write::Download);
        saves.pending.push(Pending { token, doc: id, generation, bytes: bytes.clone(), name: name.clone(), download, after });
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
        let mut written = vec![];
        for (token, outcome) in results {
            let Some(saves) = self.web_save.as_mut() else { break };
            let Some(i) = saves.pending.iter().position(|p| p.token == token) else { continue };
            let p = saves.pending.remove(i);
            match outcome {
                SaveOutcome::Saved { name, file } => {
                    written.push(token);
                    self.finish_web_save(p, name, file);
                }
                SaveOutcome::Cancelled => {}
                SaveOutcome::Failed(e) if p.download => {
                    self.notify_fmt("Couldn't download {name}: {e}", &[("name", &p.name), ("e", &e)]);
                }
                SaveOutcome::Failed(e) => self.notify_fmt("Couldn't save {name}: {e}", &[("name", &p.name), ("e", &e)]),
            }
        }
        written
    }

    /// The file is written: adopt the name the user picked, and mark the document saved if it is
    /// still as it was written (later edits stay unsaved).
    fn finish_web_save(&mut self, p: Pending, name: String, file: Option<FileId>) {
        let Some(doc) = self.session.get(p.doc) else { return };
        let name = if name.trim().is_empty() { p.name } else { name };
        let unchanged = doc.edit_generation() == p.generation;
        if unchanged && let Err(e) = self.session.mark_saved(p.doc, p.bytes, None) {
            self.notify_fmt("Saved, but reopening failed: {e}", &[("e", &e.to_string())]);
            return;
        }
        let _ = self.session.rename(p.doc, name.clone());
        if let Some(saves) = self.web_save.as_mut() {
            saves.destinations.insert(p.doc, file.map_or(Destination::Downloaded, Destination::Picked));
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
mod tests {
    use super::*;
    use pdfcraft_engine::{Edit, FieldValue};

    const FORM: &[u8] = include_bytes!("../tests/data/form.pdf");

    #[derive(Default)]
    struct Log {
        picker: bool,
        calls: Vec<String>,
        waiting: Vec<SaveDone>,
    }

    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<Log>>);

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
        fn download(&self, name: &str, _: &[u8]) -> Result<(), String> {
            self.0.borrow_mut().calls.push(format!("Download({name:?})"));
            Ok(())
        }
    }

    impl Fake {
        /// The browser answers the oldest waiting save.
        fn answer(&self, outcome: SaveOutcome) {
            let done = self.0.borrow_mut().waiting.remove(0);
            done(outcome);
        }
        fn calls(&self) -> String {
            format!("[{}]", self.0.borrow().calls.join(", "))
        }
    }

    fn app(picker: bool) -> (PdfCraftApp, Fake, DocId) {
        let fake = Fake::default();
        fake.0.borrow_mut().picker = picker;
        let mut app = PdfCraftApp::new();
        app.web_save = Some(WebSaves::new(Box::new(fake.clone())));
        app.open_bytes("Report.pdf", None, FORM.to_vec()).unwrap();
        let id = app.views[0].id;
        (app, fake, id)
    }

    fn edit(app: &mut PdfCraftApp, value: &str) {
        assert!(app.apply_edit(Edit::SetFieldValue { name: "name".into(), value: FieldValue::Text(value.into()) }));
    }

    fn dirty(app: &PdfCraftApp, id: DocId) -> bool {
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
