//! Access modes of `erpHost` v2: what the shell lets the user do with one online document
//! (`doc.mode`, `doc.canDownload`, `setMode`), carried out as the engine's session limits so a
//! change of mode needs no reopening and keeps the document and its undo history.

use std::sync::{Arc, Mutex};

use pdfcraft_engine::{DocId, SessionLimits};

use crate::PdfCraftApp;

/// `doc.mode`: what the user may do with the document's content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    #[default]
    Edit,
    /// The content is read-only, as in `View`; comments of the shell's own come later.
    Comment,
    View,
}

impl AccessMode {
    /// Absent is `edit`; a word this app doesn't know never grants editing.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            None | Some("edit") => Self::Edit,
            Some("comment") => Self::Comment,
            Some(_) => Self::View,
        }
    }
}

/// The mode and `canDownload` of one online document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub mode: AccessMode,
    pub can_download: bool,
}

impl Default for Access {
    fn default() -> Self {
        Self { mode: AccessMode::Edit, can_download: true }
    }
}

impl Access {
    /// From the shell's fields; absent ones are `edit` and `true`.
    pub fn new(mode: Option<&str>, can_download: Option<bool>) -> Self {
        Self { mode: AccessMode::parse(mode), can_download: can_download.unwrap_or(true) }
    }

    pub(crate) fn read_only(self) -> bool {
        self.mode != AccessMode::Edit
    }

    pub(crate) fn limits(self) -> SessionLimits {
        SessionLimits { view_only: self.read_only(), no_extract: !self.can_download }
    }
}

/// `setMode(docId, mode, canDownload)`: the access of a document already open changed.
#[derive(Clone, Debug, PartialEq)]
pub struct SetMode {
    pub doc_id: String,
    pub access: Access,
}

/// `setMode` calls, applied on the next frame.
pub type ModeInbox = Arc<Mutex<Vec<SetMode>>>;

impl PdfCraftApp {
    /// Apply the `setMode` calls since the last frame to the open documents they name, and to one
    /// still waiting for its password. A downgrade first commits what is still being typed, so
    /// the shell's next `flush` saves it instead of it being stranded in a read-only editor.
    pub(crate) fn apply_set_modes(&mut self) {
        let Some(online) = self.online.as_ref() else { return };
        let changes = online.modes.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for change in changes {
            let Some(online) = self.online.as_ref() else { return };
            let targets: Vec<(DocId, bool)> = online
                .records
                .iter()
                .filter(|(_, r)| r.doc_id == change.doc_id && !r.replaced)
                .map(|(id, r)| (*id, !r.access.read_only() && change.access.read_only()))
                .collect();
            for (id, downgrade) in targets {
                if downgrade {
                    self.commit_inline_edits(id);
                }
                if let Some(record) = self.online.as_mut().and_then(|o| o.records.get_mut(&id)) {
                    record.access = change.access;
                }
                self.session.set_limits(id, change.access.limits());
            }
            if let Some(pending) = self.password_prompt.as_mut().and_then(|p| p.host_doc.as_mut()).filter(|p| p.doc_id == change.doc_id) {
                pending.access = change.access;
            }
        }
    }

    /// Host document `id` is open for viewing or commenting: Save and AutoSave send nothing.
    pub(crate) fn host_read_only(&self, id: DocId) -> bool {
        self.online.as_ref().and_then(|o| o.records.get(&id)).is_some_and(|r| r.access.read_only())
    }

    /// Host document `id` is open for commenting, which is view-only on the web for now.
    pub(crate) fn host_comment_only(&self, id: DocId) -> bool {
        self.online.as_ref().and_then(|o| o.records.get(&id)).is_some_and(|r| r.access.mode == AccessMode::Comment)
    }
}
