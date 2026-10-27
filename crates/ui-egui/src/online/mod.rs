//! Documents that live on the office server (`window.erpHost` v1, web only; `docs/erp-host.md` in
//! erpcons-office). The shell opens them with `open(doc)`; the app saves them back with
//! `erpHost.save` (AutoSave, Save and the shell's `flush`) and counts one saved only once the shell
//! answers `saved`, as of the revision it sent. Save As, export and print stay local.
//!
//! The JavaScript side is a [`Host`] (`apps/pdfcraft-web/src/erp_host` on wasm, a fake in tests),
//! so all of this runs natively. Without it (`PdfCraftApp::online` is `None`, as on the desktop)
//! nothing here runs.

mod app;
mod autosave;
mod wire;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use pdfcraft_engine::DocId;

pub use autosave::{CHECKPOINT_MS, IDLE_MS, MAX_WAKE_MS};

/// The only format the server stores for this editor.
pub const FORMAT: &str = "pdf";
/// `flush()` settles this long after it was called at the latest, saves still waiting or not.
pub const FLUSH_TIMEOUT_MS: u32 = 10_000;
/// `opened` reasons are keys of the shell's intake messages, never display text.
pub const REASON_UNSUPPORTED: &str = "unsupported";
/// Unreadable bytes, or a document over the memory budget.
pub const REASON_PARSER_FAILED: &str = "parserFailed";

/// Why a save is sent (`reason` of `erpHost.save`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveReason {
    Auto,
    Manual,
    Flush,
}

impl SaveReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::Flush => "flush",
        }
    }
}

/// How the shell answered one save (`SaveResult`).
#[derive(Clone, Debug, PartialEq)]
pub enum SaveResult {
    /// Durable on the server, as `version` (when the shell said which).
    Saved { version: Option<u64> },
    /// Someone saved since the base version; nothing was written.
    Conflict { latest_version: Option<u64> },
    /// Not sent; the shell keeps the bytes and retries.
    Offline,
    /// `message` is the shell's own text, ready to show.
    Failed { retryable: bool, message: String },
}

impl SaveResult {
    /// The answer from the fields of the shell's object. Anything unrecognised counts as offline:
    /// the document stays unsaved and AutoSave keeps going, which never loses work.
    pub fn from_fields(
        status: Option<&str>,
        version: Option<f64>,
        latest_version: Option<f64>,
        retryable: Option<bool>,
        message: Option<String>,
    ) -> Self {
        match status {
            Some("saved") => Self::Saved { version: version.and_then(whole) },
            Some("conflict") => Self::Conflict { latest_version: latest_version.and_then(whole) },
            Some("failed") => Self::Failed { retryable: retryable.unwrap_or(false), message: message.unwrap_or_default() },
            _ => Self::Offline,
        }
    }

    /// Whether AutoSave may go on sending this document after this answer (rules 4 and 6).
    pub(crate) fn keeps_saving(&self) -> bool {
        match self {
            Self::Saved { .. } | Self::Offline => true,
            Self::Failed { retryable, .. } => *retryable,
            Self::Conflict { .. } => false,
        }
    }
}

/// A version number from JavaScript: a whole, non-negative, finite number.
fn whole(n: f64) -> Option<u64> {
    // `as` saturates; the checks keep NaN, fractions and negatives out.
    (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_991.0).then_some(n as u64)
}

/// Answers one save, possibly on a later frame; called at most once.
pub type Reply = Box<dyn FnOnce(SaveResult)>;

/// The shell's side of `window.erpHost`.
pub trait Host {
    /// `erpHost.save(docId, bytes, { revision, reason })`.
    fn save(&self, doc_id: &str, bytes: &[u8], revision: u64, reason: SaveReason, reply: Reply);
    /// `erpHost.dirty(docId, dirty)`.
    fn dirty(&self, doc_id: &str, dirty: bool);
    /// `erpHost.opened(docId, ok, reason?)`.
    fn opened(&self, doc_id: &str, ok: bool, reason: Option<&str>);
    /// `erpHost.closed(docId)`.
    fn closed(&self, doc_id: &str);
    /// Wall-clock milliseconds (`Date.now()` on the web).
    fn now_ms(&self) -> f64;
}

/// A document the shell handed in with `open(doc)`.
#[derive(Clone, Debug)]
pub struct HostDocument {
    pub doc_id: String,
    pub version: Option<u64>,
    /// The display name, never a path.
    pub title: String,
    pub format: String,
    pub bytes: Vec<u8>,
}

impl HostDocument {
    pub fn new(doc_id: String, version: Option<f64>, title: String, format: String, bytes: Vec<u8>) -> Self {
        Self { doc_id, version: version.and_then(whole), title, format, bytes }
    }
}

/// Documents `open` delivered, opened on the next frame.
pub type OpenInbox = Arc<Mutex<Vec<HostDocument>>>;

/// `acknowledged(docId, revision, version)`: the shell stored, on its own retry, a revision that
/// was answered `offline` or a retryable `failed`.
#[derive(Clone, Debug, PartialEq)]
pub struct Acknowledged {
    pub doc_id: String,
    pub revision: u64,
    pub version: Option<u64>,
}

impl Acknowledged {
    /// `None` for a revision that is not a whole, non-negative number: no save of ours had it.
    pub fn new(doc_id: String, revision: f64, version: Option<f64>) -> Option<Self> {
        Some(Self { doc_id, revision: whole(revision)?, version: version.and_then(whole) })
    }
}

/// Acknowledgements `acknowledged` delivered, applied on the next frame.
pub type AckInbox = Arc<Mutex<Vec<Acknowledged>>>;

/// What a password prompt for a host document carries until the document opens.
#[derive(Clone, Debug)]
pub(crate) struct PendingOpen {
    doc_id: String,
    version: Option<u64>,
}

/// Why AutoSave stopped for a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Conflict,
    Failed,
}

/// What runs once a save is written (closing the tab after Save).
type After = crate::cert_guard::After;

/// One open host document.
struct Record {
    doc_id: String,
    /// Tells this opening of `doc_id` from an earlier one whose save may still be answered.
    slot: u64,
    version: Option<u64>,
    timer: autosave::Timer,
    stopped: Option<Stop>,
    /// The latest revision handed to the shell: AutoSave doesn't send it again.
    sent: Option<u64>,
    /// The latest revision the shell said it stored (`saved` or `acknowledged`).
    acked: Option<u64>,
    /// Bytes the shell kept to retry, by revision: what an `acknowledged` marks saved.
    retried: Option<(u64, Arc<Vec<u8>>)>,
    reported_dirty: bool,
    /// A reload of the same document took this tab's place: closing it is not the user's.
    replaced: bool,
    /// Save asked for while another save was in flight (rule 3).
    queued: Option<After>,
    /// Save sent, waiting for its answer: (revision, what runs once it is saved).
    awaiting: Option<(u64, After)>,
    signed_notice: bool,
}

/// The app's side of `erpHost` (web only; `None` elsewhere).
pub struct Online {
    host: Rc<dyn Host>,
    wire: wire::Wire,
    inbox: OpenInbox,
    acks: AckInbox,
    records: HashMap<DocId, Record>,
    next_slot: u64,
}

impl Online {
    pub fn new(host: Box<dyn Host>) -> Self {
        let host: Rc<dyn Host> = Rc::from(host);
        Self {
            wire: wire::Wire::new(host.clone()),
            host,
            inbox: OpenInbox::default(),
            acks: AckInbox::default(),
            records: HashMap::new(),
            next_slot: 0,
        }
    }

    /// Where `open(doc)` queues documents.
    pub fn inbox(&self) -> OpenInbox {
        self.inbox.clone()
    }

    /// Where `acknowledged(docId, revision, version)` queues the shell's late confirmations.
    pub fn acknowledgements(&self) -> AckInbox {
        self.acks.clone()
    }

    /// What the shell's `flush()` calls, outside the app's frame.
    pub fn flush_handle(&self) -> FlushHandle {
        FlushHandle(self.wire.clone())
    }
}

/// The shell's `flush()`: shared with the page's handler, which runs outside the app's frame.
#[derive(Clone)]
pub struct FlushHandle(wire::Wire);

impl FlushHandle {
    /// Hand every document with changes the server lacks, as of the last frame, to `erpHost.save`;
    /// `done` runs once each has been handed over, a save in flight first (the caller adds the
    /// [`FLUSH_TIMEOUT_MS`] limit). The shell tracks the answers itself.
    pub fn flush(&self, done: Box<dyn FnOnce()>) {
        self.0.flush(done);
    }
}
