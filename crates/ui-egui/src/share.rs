//! File ▸ Share… (web, inside the office shell): the document, as Save a copy would write it, goes
//! to the system share sheet through `window.erpShare`, or downloads when the shell can't share.
//! The command shows only while a [`ShareFn`] is set, so never on the desktop.

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;

/// Hands `(bytes, name, mime)` to the share sheet. `false` when the shell did not take it, and
/// the app downloads the file instead. Must run inside the click: browsers share only then.
pub type ShareFn = Box<dyn Fn(&[u8], &str, &str) -> bool>;

/// The only type PdfCraft shares.
pub const PDF_MIME: &str = "application/pdf";

impl PdfCraftApp {
    /// File ▸ Share… on the active document.
    pub(crate) fn share_active(&mut self) {
        if let Some((_, id)) = self.active_ids() {
            self.share_document(id, self.share_by_pointer);
        }
    }

    /// Share document `id`; one not asked for by a pointer click downloads instead.
    pub(crate) fn share_document(&mut self, id: DocId, by_pointer: bool) {
        if self.share.is_none() || !self.commit_typing_in(id) || !self.flatten_fill_sign_for_save(id) {
            return;
        }
        // The question a copy of a signed file asks before it is written; its confirm shares.
        if !self.signature_write_confirmed(id) {
            self.hold_share(id);
            return;
        }
        let Some(name) = self.session.get(id).map(|d| crate::web_save::pdf_file_name(&d.name).unwrap_or_else(|| "document.pdf".to_string())) else {
            return;
        };
        let bytes = match self.session.save_copy_bytes(id) {
            Ok(b) => b,
            Err(e) => {
                self.notify_fmt("Couldn't share {name}: {e}", &[("name", &name), ("e", &e.to_string())]);
                return;
            }
        };
        if by_pointer && self.share.as_ref().is_some_and(|share| share(&bytes, &name, PDF_MIME)) {
            return;
        }
        // An export, not a save: the document's saved state is left alone.
        let downloaded = match self.web_save.as_ref() {
            Some(saves) => saves.download(&name, &bytes),
            None => Err("no browser to download with".to_string()),
        };
        if let Err(e) = downloaded {
            self.notify_fmt("Couldn't share {name}: {e}", &[("name", &name), ("e", &e)]);
        }
    }
}
