//! Documents closed in this session, kept so Reopen Closed Tab (Ctrl+Shift+T, File menu, a
//! document tab's menu) brings the last one back as it was: one closed with "Don't save" comes
//! back with its unsaved changes, which is the point. Each keeps its working file, which is
//! encrypted for a password-protected PDF, so no decrypted copy stays in memory; reopening one
//! asks for its password again.

use std::collections::HashSet;
use std::sync::Arc;

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;

/// Most closed documents kept.
pub const MAX_CLOSED: usize = 10;

/// Most bytes kept for them in all: they stay in memory, which a browser tab can't grow without
/// bound. The oldest go first.
pub const MAX_CLOSED_BYTES: usize = 64 << 20;

/// A closed document.
#[derive(Clone, Debug)]
pub(crate) struct ClosedDoc {
    name: String,
    path: Option<String>,
    /// The working file as it was closed. `None` for a saved document with a file on disk
    /// (desktop), which reopens from that file.
    bytes: Option<Arc<Vec<u8>>>,
    unsaved: bool,
}

impl ClosedDoc {
    fn size(&self) -> usize {
        self.bytes.as_ref().map_or(0, |b| b.len())
    }
}

/// The closed documents, most recent last.
#[derive(Debug, Default)]
pub(crate) struct ClosedDocs {
    stack: Vec<ClosedDoc>,
    /// One waiting for its password, with the documents that were open when it began.
    reopening: Option<(ClosedDoc, HashSet<DocId>)>,
}

impl ClosedDocs {
    fn push(&mut self, closed: ClosedDoc) {
        if closed.size() > MAX_CLOSED_BYTES {
            log::warn!("a closed {} is too large to keep for reopening", crate::log_safe::document(&closed.name, Some(closed.size() as u64)));
            return;
        }
        self.stack.push(closed);
        while self.stack.len() > MAX_CLOSED || self.total_bytes() > MAX_CLOSED_BYTES {
            self.stack.remove(0);
        }
    }

    fn total_bytes(&self) -> usize {
        self.stack.iter().map(ClosedDoc::size).fold(0, usize::saturating_add)
    }

    /// How many are kept (tests).
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.stack.len()
    }

    /// The names, oldest first (tests).
    #[cfg(test)]
    pub(crate) fn names(&self) -> Vec<&str> {
        self.stack.iter().map(|c| c.name.as_str()).collect()
    }
}

impl PdfCraftApp {
    /// Whether Reopen Closed Tab has a document to bring back.
    pub fn can_reopen_closed(&self) -> bool {
        self.closed.reopening.is_none() && !self.closed.stack.is_empty()
    }

    /// Keep document `id`, which is about to close, for reopening.
    pub(crate) fn keep_closed(&mut self, id: DocId) {
        let Some(doc) = self.session.get(id) else { return };
        let unsaved = doc.dirty;
        // A saved file reopens from disk on the desktop, as a newer version there should.
        let from_file = cfg!(not(target_arch = "wasm32")) && !unsaved && doc.path.is_some();
        let closed = ClosedDoc { name: doc.name.clone(), path: doc.path.clone(), bytes: (!from_file).then(|| doc.bytes.clone()), unsaved };
        self.closed.push(closed);
    }

    /// Open the most recently closed document in a new tab, under its name and as unsaved as it
    /// was. An encrypted one asks for its password first. Returns `false` when nothing opened.
    pub fn reopen_closed(&mut self) -> bool {
        if self.closed.reopening.is_some() {
            return false;
        }
        let Some(closed) = self.closed.stack.pop() else { return false };
        self.startup_superseded = true;
        let open_at = |app: &Self, path: &str| app.views.iter().position(|v| app.session.get(v.id).is_some_and(|d| d.path.as_deref() == Some(path)));
        // A saved file may have been opened again meanwhile: show that tab rather than a second one.
        if !closed.unsaved
            && let Some(index) = closed.path.as_deref().and_then(|p| open_at(self, p))
        {
            self.active = Some(index);
            return true;
        }
        let Some(bytes) = closed.bytes.clone() else {
            return self.reopen_from_file(closed);
        };
        let before: HashSet<DocId> = self.views.iter().map(|v| v.id).collect();
        match self.try_open(&closed.name, closed.path.clone(), bytes, None, true) {
            Err(e) => {
                self.notify_fmt("Couldn't open {name}: {e}", &[("name", &closed.name), ("e", &e)]);
                self.closed.stack.push(closed);
                false
            }
            Ok(()) if self.password_prompt.is_some() => {
                self.closed.reopening = Some((closed, before));
                true
            }
            Ok(()) => {
                self.adopt_reopened(closed, &before);
                true
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn reopen_from_file(&mut self, closed: ClosedDoc) -> bool {
        let Some(path) = closed.path.clone() else { return false };
        let tabs = self.views.len();
        self.open_path(&path);
        // open_path reports failures as notices; keep the entry until a tab has opened.
        let opened = self.views.len() > tabs;
        if !opened {
            self.closed.stack.push(closed);
        }
        opened
    }

    #[cfg(target_arch = "wasm32")]
    fn reopen_from_file(&mut self, closed: ClosedDoc) -> bool {
        self.closed.stack.push(closed);
        false
    }

    /// Finish a reopen that waited for its password (every frame): adopted once its tab is open,
    /// kept for later when the password was cancelled.
    pub(crate) fn poll_reopening(&mut self) {
        let Some((closed, before)) = self.closed.reopening.take() else { return };
        if self.views.iter().any(|v| !before.contains(&v.id)) {
            self.adopt_reopened(closed, &before);
            return;
        }
        let asking = self
            .password_prompt
            .as_ref()
            .is_some_and(|p| p.name == closed.name && closed.bytes.as_ref().is_some_and(|b| Arc::ptr_eq(b, &p.bytes) || **b == *p.bytes));
        #[cfg(not(target_arch = "wasm32"))]
        let asking = asking || !self.pending_opens.is_empty();
        if asking {
            self.closed.reopening = Some((closed, before));
        } else {
            self.closed.stack.push(closed);
        }
    }

    /// The tab just opened from `closed`'s bytes (the newest one not in `before`) takes back its
    /// file and its unsaved state.
    fn adopt_reopened(&mut self, closed: ClosedDoc, before: &HashSet<DocId>) {
        let Some(id) = self.views.iter().rev().map(|v| v.id).find(|id| !before.contains(id)) else { return };
        if closed.unsaved {
            self.session.mark_reopened(id, closed.path.clone());
        }
        // Its file on disk is what it was opened over, not a change to offer reloading.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = closed.path.as_deref() {
            self.watch.known_now(id, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, bytes: usize) -> ClosedDoc {
        ClosedDoc { name: name.into(), path: None, bytes: Some(Arc::new(vec![0; bytes])), unsaved: true }
    }

    #[test]
    fn at_most_ten_are_kept_the_oldest_dropped_first() {
        let mut closed = ClosedDocs::default();
        for n in 0..12 {
            closed.push(record(&format!("{n}.pdf"), 10));
        }
        assert_eq!(closed.len(), MAX_CLOSED);
        assert_eq!(closed.names().first().copied(), Some("2.pdf"));
        assert_eq!(closed.names().last().copied(), Some("11.pdf"));
    }

    #[test]
    fn at_most_64_mb_are_kept_the_oldest_dropped_first() {
        let mut closed = ClosedDocs::default();
        let third = MAX_CLOSED_BYTES / 3 + 1;
        for name in ["a.pdf", "b.pdf", "c.pdf"] {
            closed.push(record(name, third));
        }
        assert_eq!(closed.names(), ["b.pdf", "c.pdf"], "three thirds and a bit don't fit");
        assert!(closed.total_bytes() <= MAX_CLOSED_BYTES);
        closed.push(record("huge.pdf", MAX_CLOSED_BYTES + 1));
        assert_eq!(closed.names(), ["b.pdf", "c.pdf"], "one larger than the cap is not kept, and drops nothing");
        closed.push(record("exact.pdf", MAX_CLOSED_BYTES));
        assert_eq!(closed.names(), ["exact.pdf"]);
    }

    #[test]
    fn a_saved_file_on_disk_costs_no_bytes() {
        let mut closed = ClosedDocs::default();
        closed.push(ClosedDoc { name: "a.pdf".into(), path: Some("/tmp/a.pdf".into()), bytes: None, unsaved: false });
        assert_eq!((closed.len(), closed.total_bytes()), (1, 0));
    }
}
