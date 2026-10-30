//! Limits the embedding app puts on one open document for the session, such as an online document
//! the signed-in user may only view. They are kept apart from the document's own security and from
//! `read_only_reason` (which drops the editor) so that lifting them needs no reopening and keeps the
//! undo history.

use crate::{DocId, Document, EditError, Session};

/// What the embedding app forbids on one open document. Never written into the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionLimits {
    /// Nothing may change the document: no edit, undo, redo, revert or signature.
    pub view_only: bool,
    /// Nothing may leave the app: no copying, printing or exporting.
    pub no_extract: bool,
}

impl Session {
    /// Put `limits` on document `id` in place of earlier ones; `false` when it is not open.
    pub fn set_limits(&mut self, id: DocId, limits: SessionLimits) -> bool {
        self.docs.iter_mut().find(|d| d.id == id).map(|d| d.limits = limits).is_some()
    }
}

impl Document {
    /// What the embedding app forbids on this document ([`Session::set_limits`]).
    pub fn limits(&self) -> SessionLimits {
        self.limits
    }

    /// [`SessionLimits::view_only`] as the error a refused change returns.
    pub(crate) fn check_not_view_only(&self) -> Result<(), EditError> {
        if self.limits.view_only { Err(EditError::ViewOnly) } else { Ok(()) }
    }
}
