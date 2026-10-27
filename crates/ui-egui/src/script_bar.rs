//! Acrobat JavaScript never runs silently: a document whose scripts are not allowed says so on a
//! bar over its pages, and "Allow for this file" trusts it for the session
//! ([`pdfcraft_engine::Session::trust_scripts`]).

use pdfcraft_engine::{DocId, Document};

use crate::PdfCraftApp;
use crate::theme::Tokens;

/// One document's scripts bar.
#[derive(Clone, Debug, Default)]
pub struct ScriptsBar {
    /// Closed with ×; a script blocked after that brings it back once.
    dismissed: Option<bool>,
    /// Whether the document has scripts, by edit generation (the check walks every field).
    present: Option<(u64, bool)>,
    /// "Allow for this file" was clicked; carried out after the frame's drawing.
    allow: bool,
}

impl ScriptsBar {
    fn has_scripts(&mut self, doc: &Document) -> bool {
        let generation = doc.edit_generation();
        match self.present {
            Some((g, present)) if g == generation => present,
            _ => {
                let present = doc.has_scripts();
                self.present = Some((generation, present));
                present
            }
        }
    }

    /// Whether the bar shows for `doc`, given whether its scripts are allowed. A blocked console
    /// or Run JavaScript shows it on a document without scripts too, so they can be allowed.
    pub fn shown(&mut self, doc: &Document, allowed: bool) -> bool {
        if allowed || !(doc.scripts_skipped() || self.has_scripts(doc)) {
            return false;
        }
        match self.dismissed {
            None => true,
            Some(skipped_then) => doc.scripts_skipped() && !skipped_then,
        }
    }
}

/// The bar's message: what a blocked button or calculation adds to it.
fn message(skipped: bool) -> &'static str {
    if skipped {
        tl!("This PDF contains JavaScript. Its scripts have not been run, so the button or calculation you used did nothing.")
    } else {
        tl!("This PDF contains JavaScript. Its scripts have not been run.")
    }
}

/// Draw the bar over `doc`'s pages when it applies.
pub(crate) fn show(bar: &mut ScriptsBar, doc: &Document, allowed: bool, ui: &mut egui::Ui, t: &Tokens) {
    if !bar.shown(doc, allowed) {
        return;
    }
    let skipped = doc.scripts_skipped();
    crate::canvas::notice_bar(ui, t, "braces", t.accent_text, message(skipped).to_string(), |ui| {
        if crate::icons::button(ui, "x", 22.0, false, tl!("Dismiss")).clicked() {
            bar.dismissed = Some(skipped);
        }
        if crate::widgets::pill_button(ui, tl!("Allow for this file"), true).clicked() {
            bar.allow = true;
        }
    });
}

impl PdfCraftApp {
    /// Allow document `id`'s scripts for this session; a document without unsaved changes or undo
    /// history opens again in place so its form's opening scripts run.
    pub fn allow_scripts(&mut self, id: DocId) {
        // A field still being typed in is an entry too: committed, it keeps the document from
        // being reopened over it.
        if !self.commit_typing_in(id) {
            return;
        }
        match self.session.trust_scripts(id) {
            Ok(reopened) => {
                if reopened && let (Some(view), Some(doc)) = (self.views.iter_mut().find(|v| v.id == id), self.session.get(id)) {
                    view.document_replaced(&doc.info);
                }
                let out = self.session.take_js_output(id);
                self.handle_js(id, out);
            }
            Err(e) => {
                let name = self.session.get(id).map(|d| d.name.clone()).unwrap_or_default();
                self.notify_fmt("Couldn't reload {name}: {e}", &[("name", &name), ("e", &e.to_string())]);
            }
        }
    }

    /// Carry out an "Allow for this file" clicked on a scripts bar.
    pub(crate) fn allow_requested_scripts(&mut self) {
        if let Some(id) = self.views.iter_mut().find_map(|v| std::mem::take(&mut v.scripts_bar.allow).then_some(v.id)) {
            self.allow_scripts(id);
        }
    }
}
