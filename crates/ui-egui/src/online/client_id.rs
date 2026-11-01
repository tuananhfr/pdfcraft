//! `setClientId(prefix)` of `erpHost` v2: the shell's prefix for every id the session creates from
//! then on (comment names, page ids), so people editing one document at once never create the
//! same id. Without it the session keeps the prefix it picked itself.

use std::sync::{Arc, Mutex};

use pdfcraft_engine::ClientPrefix;

use crate::PdfCraftApp;

/// The latest `setClientId` not applied yet.
pub type ClientIdInbox = Arc<Mutex<Option<ClientPrefix>>>;

impl PdfCraftApp {
    /// Apply the latest `setClientId`.
    pub(crate) fn apply_host_client_id(&mut self) {
        let Some(online) = self.online.as_ref() else { return };
        let Some(prefix) = online.client_id.lock().ok().and_then(|mut p| p.take()) else { return };
        self.session.set_client_prefix(prefix);
    }
}
