//! Cloud comments anchored in the document (ERPCons M5, the `erpHost` comment calls; web only,
//! `docs/erp-host.md` in erpcons-office). The shell keeps the threads on the server and shows
//! them in its panel; the app tells it where the selection is (`anchorOfSelection`), draws a
//! marker at each open thread's anchor (`showComments`), goes to an anchor (`focusAnchor`), finds
//! anchors again after the document changed (`resolveAnchors`) and says which marker was clicked
//! (`commentClicked`). Markers are drawn over the pages only: they are never document content and
//! never saved. Anchors are [`pdfcraft_engine::anchors::Anchor`]s. Without a shell calling these
//! (the desktop, an older shell) none of this exists.

mod frame;
mod markers;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex};

use pdfcraft_engine::DocId;
use pdfcraft_engine::anchors::Anchor;

pub use markers::CommentMarker;
pub(crate) use markers::paint_markers;

/// Threads read from one `showComments` or `resolveAnchors`; the list comes from the server, and
/// finding an anchor again may search a page's text.
pub const MAX_THREADS: usize = 1000;

/// The shell's side of the comment calls, called by the app.
pub trait CommentHost {
    /// `erpHost.commentClicked(threadId)`.
    fn comment_clicked(&self, thread: &str);
}

/// One thread as `showComments` and `resolveAnchors` give it.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    /// The anchor's JSON, as the shell stored it.
    pub anchor: String,
    /// `open`, `resolved` or `orphaned`: only open threads get a marker.
    pub status: String,
    pub count: u32,
}

/// Whether `resolveAnchors` found an anchor again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorStatus {
    Open,
    Orphaned,
}

impl AnchorStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Orphaned => "orphaned",
        }
    }
}

/// One answer of `resolveAnchors`: the anchor as it is now (unchanged when orphaned).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedThread {
    pub id: String,
    pub anchor: String,
    pub status: AnchorStatus,
}

/// Answers `resolveAnchors`, once.
pub type ResolveReply = Box<dyn FnOnce(Vec<ResolvedThread>)>;

/// What the shell called, applied on the next frame.
pub enum CommentEvent {
    Show(Vec<Thread>),
    /// `focusAnchor(anchor)`, the anchor's JSON.
    Focus(String),
    Resolve {
        threads: Vec<Thread>,
        reply: ResolveReply,
    },
}

pub type CommentInbox = Arc<Mutex<Vec<CommentEvent>>>;
/// The anchor of the selection as of the last frame, as JSON: `anchorOfSelection` answers with it
/// at once.
pub type SelectionAnchor = Arc<Mutex<Option<String>>>;

/// What the selection anchor was made from, to make it again only when that changes.
#[derive(Clone, Debug, PartialEq)]
enum Selected {
    Annotation { page: usize, index: usize },
    Field { name: String, widget: usize },
    Text { page: usize, quads: Vec<[f64; 8]> },
}

/// The app's side of the comment calls (web, `erpHost` v2 and later).
pub struct Comments {
    host: Box<dyn CommentHost>,
    inbox: CommentInbox,
    selection: SelectionAnchor,
    selection_key: Option<(DocId, u64, Option<Selected>)>,
    /// The threads of the last `showComments`, with their anchors read.
    threads: Vec<(Thread, Option<Anchor>)>,
    /// Bumped by every `showComments`: the markers are placed again.
    revision: u64,
    markers_key: Option<(DocId, u64, u64)>,
}

impl Comments {
    pub fn new(host: Box<dyn CommentHost>) -> Self {
        Self {
            host,
            inbox: CommentInbox::default(),
            selection: SelectionAnchor::default(),
            selection_key: None,
            threads: Vec::new(),
            revision: 0,
            markers_key: None,
        }
    }

    /// Where the shell's calls queue their events.
    pub fn inbox(&self) -> CommentInbox {
        self.inbox.clone()
    }

    /// What `anchorOfSelection` reads.
    pub fn selection(&self) -> SelectionAnchor {
        self.selection.clone()
    }
}
