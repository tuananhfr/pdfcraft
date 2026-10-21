//! Host clipboard callbacks and delivery of asynchronous reads to the app thread.

use std::sync::{Arc, Mutex};

use crate::{Clip, PdfCraftApp};

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;

#[derive(Debug)]
pub enum Contents {
    Text(String),
    Image { name: String, bytes: Vec<u8> },
}

pub type ReadInbox = Arc<Mutex<Vec<Result<Option<Contents>, ()>>>>;

pub type TextWriter = Box<dyn Fn(&str)>;

pub struct HostClipboard {
    pub read: Box<dyn Fn()>,
    pub write: TextWriter,
    pub inbox: ReadInbox,
}

impl PdfCraftApp {
    pub(crate) fn poll_clipboard(&mut self) {
        let results = self.clipboard.as_ref().and_then(|c| c.inbox.lock().ok().map(|mut q| std::mem::take(&mut *q))).unwrap_or_default();
        for result in results {
            let created = match result {
                Ok(Some(Contents::Text(text))) => self.create_from_clip(Clip::Text(text)),
                Ok(Some(Contents::Image { name, bytes })) => self
                    .open_converted(&name, &bytes)
                    .unwrap_or_else(|| Err(tl!("The clipboard has no image or text").to_string())),
                Ok(None) => {
                    self.notify_tr("The clipboard has no image or text");
                    continue;
                }
                Err(()) => {
                    self.notify_tr("The browser won't allow reading the clipboard — use Ctrl+V");
                    continue;
                }
            };
            if let Err(e) = created {
                self.notify_fmt("Couldn't create a PDF: {e}", &[("e", &e)]);
            }
        }
    }
}
