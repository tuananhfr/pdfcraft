//! What a document's tab says about saving it: "Saving…" while the browser writes it (its save
//! picker included), and "Save failed", with the reason on hover, from a failed save until the
//! document is saved or edited again. The desktop writes a file within the frame that starts the
//! write, so only failures show there.

use std::collections::HashMap;

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;

/// A document's save, as its tab shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    Saving,
    /// The last save failed; the message says why, in the UI language.
    Failed(String),
}

/// Failed saves: the message and the edit generation the document had then.
#[derive(Debug, Default)]
pub(crate) struct SaveFailures(HashMap<DocId, (String, u64)>);

impl PdfCraftApp {
    /// How the save of document `id` stands, if there is anything to say.
    pub fn save_state(&self, id: DocId) -> Option<SaveState> {
        if self.web_save.as_ref().is_some_and(|s| s.is_pending(id)) {
            return Some(SaveState::Saving);
        }
        let (message, generation) = self.save_failures.0.get(&id)?;
        // Saving and editing both bump the generation, which ends the failed state.
        let current = self.session.get(id)?.edit_generation();
        (*generation == current).then(|| SaveState::Failed(message.clone()))
    }

    /// A save of document `id` failed: tell the user now, and keep saying so on its tab.
    pub(crate) fn save_failed(&mut self, id: DocId, template: &str, args: &[(&str, &str)]) {
        let message = crate::i18n::fmt(tl!(template), args);
        self.notify(message.clone());
        let open: Vec<DocId> = self.views.iter().map(|v| v.id).collect();
        self.save_failures.0.retain(|doc, _| open.contains(doc));
        if let Some(generation) = self.session.get(id).map(|d| d.edit_generation()) {
            self.save_failures.0.insert(id, (message, generation));
        }
    }
}

/// The words a tab shows for `state`.
pub(crate) fn label(state: &SaveState) -> &'static str {
    match state {
        SaveState::Saving => tl!("Saving…"),
        SaveState::Failed(_) => tl!("Save failed"),
    }
}
