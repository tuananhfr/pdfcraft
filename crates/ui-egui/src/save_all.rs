//! Save all on the web, from the prompt for several unsaved documents. A browser shows its save
//! picker only in answer to a click, so after the first document each one waits for its own click
//! ("Save “<next>”") while the prompt lists what is saved and what is being saved. A cancelled
//! picker or a failed save ends the run: the saved documents are closed, nothing still unsaved is.
//! A browser without a picker downloads them all on the one click (`downloads_all`).

use std::collections::HashSet;

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;
use crate::editing::{CloseRequest, SaveTarget};

/// A Save all that goes one picker (and one click) at a time.
pub(crate) struct SaveAllRun {
    req: CloseRequest,
    /// The documents the prompt listed, in tab order, with their names then.
    docs: Vec<(DocId, String)>,
    saved: HashSet<DocId>,
    /// The document whose picker is open.
    current: Option<DocId>,
}

/// How a listed document stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Unsaved,
    Saving,
    Saved,
}

impl SaveAllRun {
    /// The listed documents and how each stands.
    pub(crate) fn rows(&self) -> Vec<(String, Row)> {
        self.docs
            .iter()
            .map(|(id, name)| {
                let row = if self.saved.contains(id) {
                    Row::Saved
                } else if self.current == Some(*id) {
                    Row::Saving
                } else {
                    Row::Unsaved
                };
                (name.clone(), row)
            })
            .collect()
    }

    /// A picker is open.
    pub(crate) fn busy(&self) -> bool {
        self.current.is_some()
    }
}

impl PdfCraftApp {
    /// Save all needs a click for each save picker: the browser's own picker saves here.
    pub(crate) fn saves_one_click_each(&self) -> bool {
        self.host_save.is_none() && self.web_save.as_ref().is_some_and(|w| w.has_picker())
    }

    /// Save all with no picker at all: every document downloads under its name.
    pub(crate) fn downloads_all(&self) -> bool {
        self.host_save.is_none() && self.web_save.as_ref().is_some_and(|w| !w.has_picker())
    }

    /// Start a one-click-each Save all for `req` (Quit or Close all), saving the first now.
    pub(crate) fn begin_save_all(&mut self, ctx: &egui::Context, req: CloseRequest) {
        let docs = self
            .unsaved_tabs()
            .into_iter()
            .filter_map(|i| self.views.get(i).map(|v| v.id))
            .filter_map(|id| self.session.get(id).map(|d| (id, d.name.clone())))
            .collect();
        self.save_all = Some(SaveAllRun { req, docs, saved: HashSet::new(), current: None });
        self.save_next_of_all(ctx);
    }

    /// The name of the next document Save all would save.
    pub(crate) fn save_all_next(&self) -> Option<String> {
        let run = self.save_all.as_ref()?;
        let id = self.next_of_all()?;
        run.docs.iter().find(|(d, _)| *d == id).map(|(_, name)| name.clone())
    }

    fn next_of_all(&self) -> Option<DocId> {
        let run = self.save_all.as_ref()?;
        run.docs.iter().map(|(id, _)| *id).find(|id| !run.saved.contains(id) && self.views.iter().any(|v| v.id == *id))
    }

    /// The prompt's "Save “<next>”": open the next document's picker (this click allows it).
    pub(crate) fn save_next_of_all(&mut self, ctx: &egui::Context) {
        if self.save_all.as_ref().is_none_or(SaveAllRun::busy) {
            return;
        }
        let Some(id) = self.next_of_all() else { return self.finish_save_all(ctx) };
        let Some(index) = self.views.iter().position(|v| v.id == id) else { return };
        self.active = Some(index);
        if let Some(run) = self.save_all.as_mut() {
            run.current = Some(id);
        }
        let ctx = ctx.clone();
        self.save_then(index, SaveTarget::InPlace, move |app| {
            if let Some(i) = app.views.iter().position(|v| v.id == id) {
                app.close_tab(i);
            }
            if let Some(run) = app.save_all.as_mut() {
                run.saved.insert(id);
                run.current = None;
            }
            if app.save_all.is_some() && app.next_of_all().is_none() {
                app.finish_save_all(&ctx);
            }
        });
    }

    /// Every document is saved: quit, for Quit.
    fn finish_save_all(&mut self, ctx: &egui::Context) {
        if let Some(run) = self.save_all.take()
            && run.req == CloseRequest::Quit
        {
            self.quit(ctx);
        }
    }

    /// The prompt's Cancel: what is saved stays closed, the rest stays open.
    pub(crate) fn stop_save_all(&mut self) {
        self.save_all = None;
    }

    /// The prompt's Don't save: close the documents not saved yet, without saving them.
    pub(crate) fn discard_rest_of_save_all(&mut self, ctx: &egui::Context) {
        let Some(run) = self.save_all.take() else { return };
        for (id, _) in run.docs.iter().filter(|(id, _)| !run.saved.contains(id)) {
            if let Some(i) = self.views.iter().position(|v| v.id == *id) {
                self.close_tab(i);
            }
        }
        if run.req == CloseRequest::Quit {
            self.quit(ctx);
        }
    }

    /// The prompt's answer while Save all runs: `Some(true)` the next save, `Some(false)` Don't
    /// save, `None` Cancel. Nothing is answered while a picker is open.
    pub(crate) fn answer_save_all(&mut self, ctx: &egui::Context, choice: Option<bool>) {
        if self.save_all.as_ref().is_some_and(SaveAllRun::busy) {
            return;
        }
        match choice {
            Some(true) => self.save_next_of_all(ctx),
            Some(false) => self.discard_rest_of_save_all(ctx),
            None => self.stop_save_all(),
        }
    }

    /// A save of the run that ended unwritten (its picker cancelled, the write failed, or it was
    /// held for a signature) ends the run (every frame).
    pub(crate) fn poll_save_all(&mut self) {
        let Some(current) = self.save_all.as_ref().and_then(|r| r.current) else { return };
        if !self.web_save.as_ref().is_some_and(|w| w.is_pending(current)) {
            self.save_all = None;
        }
    }
}
