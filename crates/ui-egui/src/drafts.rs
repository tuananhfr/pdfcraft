//! Web draft autosave (ERPCons office shell): unsaved documents are handed to the page's draft
//! store (`window.erpDrafts`) soon after each change, so a crash or a closed tab doesn't lose
//! them, and the store hands drafts back to reopen.
//!
//! The timing is a pure [`Planner`] (it takes `now`), tested natively. The web app supplies the
//! [`DraftSink`] that talks to JavaScript and flushes [`Flush`] when the page is hidden. The
//! shell owns all wording; nothing here is shown to the user. Desktop keeps [`crate::RecoveryStore`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;

/// The only format this app drafts: the working file, already encrypted for protected PDFs.
pub const FORMAT: &str = "pdf";
/// Put once the document has been left alone this long.
pub const IDLE_MS: f64 = 2500.0;
/// Put at least this often while changes keep coming.
pub const CHECKPOINT_MS: f64 = 30_000.0;
/// Space puts at least this many times the last put's duration apart.
pub const ADAPTIVE_FACTOR: f64 = 10.0;
/// Longest time between ticks while a put is due later.
pub const MAX_WAKE_MS: f64 = 1000.0;

/// The page's draft store. Every call looks the store up afresh: without it the feature is off.
pub trait DraftSink {
    fn available(&self) -> bool;
    /// The app can open documents: the store may now offer recovery (and call restore).
    fn ready(&self) -> Result<(), String>;
    fn put(&self, key: &str, name: &str, format: &str, bytes: &[u8]) -> Result<(), String>;
    fn remove(&self, key: &str) -> Result<(), String>;
    /// Wall-clock milliseconds (`Date.now()` on the web).
    fn now_ms(&self) -> f64;
}

/// A draft the store asked to reopen.
#[derive(Clone, Debug)]
pub struct DraftRestore {
    pub key: String,
    pub name: String,
    pub format: String,
    pub bytes: Vec<u8>,
}

/// Restores delivered by the store, opened on the next frame.
pub type DraftInbox = std::sync::Arc<std::sync::Mutex<Vec<DraftRestore>>>;

/// What one open document looks like this frame.
#[derive(Clone, Copy, Debug)]
pub struct DocState {
    pub id: DocId,
    /// Changes with every change to the working file.
    pub generation: u64,
    /// Dirty, or a save of it hasn't settled.
    pub unsaved: bool,
    /// Typing not committed to the document yet.
    pub transient: bool,
}

/// What to do this frame.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub put: Vec<DocId>,
    pub remove: Vec<String>,
    /// Run again within this many milliseconds, even without input.
    pub wake_in_ms: Option<f64>,
}

#[derive(Debug)]
struct Tracked {
    key: String,
    /// The store holds a draft under `key`.
    stored: bool,
    /// The generation of the stored draft.
    drafted: Option<u64>,
    seen: u64,
    last_change: f64,
    first_unput_change: Option<f64>,
    last_put: f64,
    last_put_cost_ms: f64,
    retry_at: f64,
}

/// Decides which documents to put or remove, given their state and the time.
#[derive(Debug, Default)]
pub struct Planner {
    docs: HashMap<DocId, Tracked>,
    /// Every key used in this page, so no key is ever reused for another document.
    used_keys: HashSet<String>,
    /// Stored keys no document uses any more, removed on the next tick.
    orphaned: Vec<String>,
}

impl Planner {
    pub fn key(&self, id: DocId) -> Option<&str> {
        self.docs.get(&id).map(|t| t.key.as_str())
    }

    pub fn observe(&mut self, docs: &[DocState], now: f64) -> Plan {
        let mut plan = Plan { remove: std::mem::take(&mut self.orphaned), ..Plan::default() };
        let open: HashSet<DocId> = docs.iter().map(|d| d.id).collect();
        self.docs.retain(|id, t| {
            let keep = open.contains(id);
            if !keep && t.stored {
                plan.remove.push(t.key.clone());
            }
            keep
        });
        let mut wake: Option<f64> = None;
        for d in docs {
            if !self.docs.contains_key(&d.id) {
                let key = self.fresh_key(d.id);
                self.docs.insert(d.id, Tracked::new(key, d.generation, now));
            }
            let Some(t) = self.docs.get_mut(&d.id) else { continue };
            if d.generation != t.seen {
                t.seen = d.generation;
                t.last_change = now;
            }
            if !d.unsaved {
                if t.stored {
                    plan.remove.push(t.key.clone());
                    t.stored = false;
                }
                t.drafted = None;
                t.first_unput_change = None;
                continue;
            }
            if t.drafted == Some(d.generation) {
                continue;
            }
            let first = *t.first_unput_change.get_or_insert(now);
            // The commit that ends the typing is a change of its own, and input wakes the app.
            if d.transient {
                continue;
            }
            let due = (t.last_change + IDLE_MS).min(first + CHECKPOINT_MS).max(t.last_put + ADAPTIVE_FACTOR * t.last_put_cost_ms).max(t.retry_at);
            if now >= due {
                plan.put.push(d.id);
            } else {
                let wait = due - now;
                wake = Some(wake.map_or(wait, |w: f64| w.min(wait)));
            }
        }
        plan.wake_in_ms = wake.map(|w| w.min(MAX_WAKE_MS));
        plan
    }

    /// `generation` of `id` is in the store now; the put took `cost_ms` (unknown: unchanged).
    pub fn put_done(&mut self, id: DocId, generation: u64, now: f64, cost_ms: Option<f64>) {
        let Some(t) = self.docs.get_mut(&id) else { return };
        t.stored = true;
        t.drafted = Some(generation);
        t.last_put = now;
        if let Some(cost) = cost_ms.filter(|c| c.is_finite()) {
            t.last_put_cost_ms = cost.max(0.0);
        }
        t.retry_at = f64::NEG_INFINITY;
        if generation == t.seen {
            t.first_unput_change = None;
        }
    }

    /// The store refused the put: try again after an idle period, not every frame.
    pub fn put_failed(&mut self, id: DocId, now: f64) {
        if let Some(t) = self.docs.get_mut(&id) {
            t.retry_at = now + IDLE_MS;
        }
    }

    /// `id` was opened from the store's draft `key` at `generation`: it keeps that key and is
    /// already drafted.
    pub fn adopt(&mut self, id: DocId, key: String, generation: u64, now: f64) {
        // Another open document using the key moves to a fresh one and is put again.
        let clash = self.docs.iter().find(|(other, t)| **other != id && t.key == key).map(|(other, _)| *other);
        if let Some(other) = clash {
            let fresh = self.fresh_key(other);
            if let Some(t) = self.docs.get_mut(&other) {
                t.key = fresh;
                t.stored = false;
                t.drafted = None;
            }
        }
        if let Some(old) = self.docs.get(&id)
            && old.stored
            && old.key != key
        {
            self.orphaned.push(old.key.clone());
        }
        self.used_keys.insert(key.clone());
        let mut tracked = Tracked::new(key, generation, now);
        tracked.stored = true;
        tracked.drafted = Some(generation);
        tracked.last_put = now;
        self.docs.insert(id, tracked);
    }

    /// Unsaved documents whose latest change isn't in the store, whatever the timing: what a
    /// flush puts when the page is hidden.
    pub fn unput<'a>(&'a self, docs: &'a [DocState]) -> impl Iterator<Item = (&'a DocState, &'a str)> + 'a {
        docs.iter().filter(|d| d.unsaved).filter_map(|d| {
            let t = self.docs.get(&d.id)?;
            (t.drafted != Some(d.generation)).then_some((d, t.key.as_str()))
        })
    }

    fn fresh_key(&mut self, id: DocId) -> String {
        let base = format!("doc-{}", id.0);
        let mut key = base.clone();
        let mut n: u64 = 2;
        while self.used_keys.contains(&key) {
            key = format!("{base}-{n}");
            n = n.saturating_add(1);
        }
        self.used_keys.insert(key.clone());
        key
    }
}

impl Tracked {
    fn new(key: String, generation: u64, now: f64) -> Self {
        Self {
            key,
            stored: false,
            drafted: None,
            seen: generation,
            last_change: now,
            first_unput_change: None,
            last_put: f64::NEG_INFINITY,
            last_put_cost_ms: 0.0,
            retry_at: f64::NEG_INFINITY,
        }
    }
}

/// One draft waiting to be put, as of the end of the last frame.
#[derive(Clone, Debug)]
pub struct FlushItem {
    pub doc: DocId,
    pub generation: u64,
    pub key: String,
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
}

/// Shared with the page's `visibilitychange` / `pagehide` handlers, which run outside the app's
/// frame and must not reach into it.
#[derive(Debug, Default)]
pub struct Flush {
    pub pending: Vec<FlushItem>,
    /// Put by a flush since the last frame, for the planner to record.
    pub flushed: Vec<(DocId, u64)>,
}

pub type FlushCell = Rc<RefCell<Flush>>;

impl Flush {
    /// Put every pending draft now, ignoring the idle wait (the page is being hidden or left).
    pub fn flush(&mut self, sink: &dyn DraftSink) {
        if !sink.available() {
            return;
        }
        for item in std::mem::take(&mut self.pending) {
            match sink.put(&item.key, &item.name, FORMAT, &item.bytes) {
                Ok(()) => self.flushed.push((item.doc, item.generation)),
                Err(e) => log::warn!("draft put failed for {}: {e}", item.key),
            }
        }
    }
}

/// The app's side of draft autosave (web only; `None` elsewhere and in `?host=parent` mode).
pub struct Drafts {
    sink: Box<dyn DraftSink>,
    planner: Planner,
    ready_sent: bool,
    restores: DraftInbox,
    flush: FlushCell,
}

impl Drafts {
    pub fn new(sink: Box<dyn DraftSink>, restores: DraftInbox) -> Self {
        Self { sink, planner: Planner::default(), ready_sent: false, restores, flush: FlushCell::default() }
    }

    pub fn flush_cell(&self) -> FlushCell {
        self.flush.clone()
    }
}

/// The name the store shows: the file name, with the `.pdf` a save would give it.
fn draft_name(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".pdf") { name.to_string() } else { format!("{name}.pdf") }
}

impl PdfCraftApp {
    /// Open drafts the store asked to restore, as unsaved documents keeping the store's key.
    pub(crate) fn open_draft_restores(&mut self) {
        let Some(inbox) = self.drafts.as_ref().map(|d| d.restores.clone()) else { return };
        // One password prompt at a time: an encrypted draft waits for the one before it.
        while self.password_prompt.is_none() {
            let next = inbox.lock().ok().and_then(|mut q| (!q.is_empty()).then(|| q.remove(0)));
            let Some(restore) = next else { break };
            if restore.format != FORMAT || restore.key.is_empty() {
                log::warn!("ignoring draft {:?} in format {:?}", restore.key, restore.format);
                continue;
            }
            self.startup_superseded = true;
            // Not `open_bytes`: a draft is always a PDF, never a file to convert.
            match self.try_open(&restore.name, None, Arc::new(restore.bytes), None, true) {
                Ok(()) if self.password_prompt.is_none() => self.finish_draft_restore(restore.key),
                Ok(()) => {
                    if let Some(prompt) = self.password_prompt.as_mut() {
                        prompt.draft_key = Some(restore.key);
                    }
                }
                Err(e) => self.notify_fmt("Couldn't open {name}: {e}", &[("name", &restore.name), ("e", &e)]),
            }
        }
    }

    /// The most recently opened tab came from draft `key`: it is unsaved work stored under that key.
    pub(crate) fn finish_draft_restore(&mut self, key: String) {
        let Some(id) = self.views.last().map(|v| v.id) else { return };
        self.session.mark_recovered(id, None);
        let Some(generation) = self.session.get(id).map(|d| d.edit_generation()) else { return };
        if let Some(drafts) = self.drafts.as_mut() {
            let now = drafts.sink.now_ms();
            drafts.planner.adopt(id, key, generation, now);
        }
    }

    /// Put and remove drafts as documents change, at the end of every frame.
    pub(crate) fn drafts_tick(&mut self, ctx: &egui::Context) {
        if let Some(ms) = self.drafts_step() {
            // `as` saturates (NaN gives 0), so this never panics.
            ctx.request_repaint_after(std::time::Duration::from_millis(ms.clamp(0.0, MAX_WAKE_MS) as u64));
        }
    }

    /// One tick; returns how soon the next one is needed.
    pub(crate) fn drafts_step(&mut self) -> Option<f64> {
        if !self.drafts.as_ref().is_some_and(|d| d.sink.available()) {
            return None;
        }
        let states: Vec<DocState> = (0..self.views.len())
            .filter_map(|i| {
                let id = self.views.get(i)?.id;
                let doc = self.session.get(id)?;
                // A browser save leaves the document dirty until the file is written (`web_save`),
                // and a host's save settles at once, so dirty is all of Unsaved.
                Some(DocState { id, generation: doc.edit_generation(), unsaved: doc.dirty, transient: self.has_uncommitted_input(i) })
            })
            .collect();
        let drafts = self.drafts.as_mut()?;
        if !drafts.ready_sent {
            drafts.ready_sent = true;
            if let Err(e) = drafts.sink.ready() {
                log::warn!("draft store ready() failed: {e}");
            }
        }
        let now = drafts.sink.now_ms();
        let flushed = drafts.flush.try_borrow_mut().map(|mut f| std::mem::take(&mut f.flushed)).unwrap_or_default();
        for (id, generation) in flushed {
            drafts.planner.put_done(id, generation, now, None);
        }
        let plan = drafts.planner.observe(&states, now);
        for key in &plan.remove {
            if let Err(e) = drafts.sink.remove(key) {
                log::warn!("draft remove failed for {key}: {e}");
            }
        }
        for id in plan.put {
            let (Some(doc), Some(key)) = (self.session.get(id), drafts.planner.key(id)) else { continue };
            let started = drafts.sink.now_ms();
            match drafts.sink.put(key, &draft_name(&doc.name), FORMAT, &doc.bytes) {
                Ok(()) => {
                    let cost = drafts.sink.now_ms() - started;
                    drafts.planner.put_done(id, doc.edit_generation(), now, Some(cost));
                }
                Err(e) => {
                    log::warn!("draft put failed for {key}: {e}");
                    drafts.planner.put_failed(id, now);
                }
            }
        }
        // Includes documents with typing open: their working file holds only committed edits,
        // and the page may be gone before the typing is.
        let pending: Vec<FlushItem> = drafts
            .planner
            .unput(&states)
            .filter_map(|(state, key)| {
                let doc = self.session.get(state.id)?;
                Some(FlushItem {
                    doc: state.id,
                    generation: state.generation,
                    key: key.to_string(),
                    name: draft_name(&doc.name),
                    bytes: doc.bytes.clone(),
                })
            })
            .collect();
        if let Ok(mut flush) = drafts.flush.try_borrow_mut() {
            flush.pending = pending;
        }
        plan.wake_in_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: u64, generation: u64, unsaved: bool) -> DocState {
        DocState { id: DocId(id), generation, unsaved, transient: false }
    }

    #[test]
    fn clean_documents_are_never_put_and_need_no_wake() {
        let mut p = Planner::default();
        for now in [0.0, 5000.0, 60_000.0] {
            assert_eq!(p.observe(&[doc(1, 0, false)], now), Plan::default());
        }
    }

    #[test]
    fn a_change_is_put_once_the_document_is_left_alone() {
        let mut p = Planner::default();
        p.observe(&[doc(1, 0, false)], 0.0);
        let plan = p.observe(&[doc(1, 1, true)], 1000.0);
        assert!(plan.put.is_empty());
        assert_eq!(plan.wake_in_ms, Some(MAX_WAKE_MS), "a frame is needed before the idle moment");
        let plan = p.observe(&[doc(1, 1, true)], 3000.0);
        assert!(plan.put.is_empty());
        assert_eq!(plan.wake_in_ms, Some(500.0));
        let plan = p.observe(&[doc(1, 1, true)], 3500.0);
        assert_eq!(plan.put, vec![DocId(1)]);
        assert_eq!(p.key(DocId(1)), Some("doc-1"));
        p.put_done(DocId(1), 1, 3500.0, Some(1.0));
        assert_eq!(p.observe(&[doc(1, 1, true)], 9000.0), Plan::default(), "nothing new: no put, no wake");
    }

    #[test]
    fn continuous_changes_still_reach_the_store_at_the_checkpoint() {
        let mut p = Planner::default();
        let mut put_at = None;
        for step in 1..=40u64 {
            let now = step as f64 * 1000.0;
            if !p.observe(&[doc(1, step, true)], now).put.is_empty() {
                put_at = Some(now);
                break;
            }
        }
        assert_eq!(put_at, Some(31_000.0), "30 s after the first unput change");
    }

    #[test]
    fn puts_are_spaced_by_ten_times_the_last_put_duration() {
        let mut p = Planner::default();
        p.observe(&[doc(1, 1, true)], 0.0);
        assert_eq!(p.observe(&[doc(1, 1, true)], 2500.0).put, vec![DocId(1)]);
        p.put_done(DocId(1), 1, 2500.0, Some(2000.0));
        p.observe(&[doc(1, 2, true)], 3000.0);
        let plan = p.observe(&[doc(1, 2, true)], 6000.0);
        assert!(plan.put.is_empty(), "idle, but 20 s haven't passed since the last put");
        assert_eq!(p.observe(&[doc(1, 2, true)], 22_500.0).put, vec![DocId(1)]);
    }

    #[test]
    fn open_typing_waits_for_its_commit() {
        let mut p = Planner::default();
        let typing = DocState { transient: true, ..doc(1, 1, true) };
        p.observe(&[typing], 0.0);
        let plan = p.observe(&[typing], 60_000.0);
        assert!(plan.put.is_empty());
        assert_eq!(plan.wake_in_ms, None, "the commit is input, which wakes the app by itself");
        p.observe(&[doc(1, 2, true)], 61_000.0);
        assert_eq!(p.observe(&[doc(1, 2, true)], 63_500.0).put, vec![DocId(1)]);
    }

    #[test]
    fn a_stored_draft_is_removed_once_when_saved_or_undone() {
        let mut p = Planner::default();
        p.observe(&[doc(1, 1, true)], 0.0);
        p.observe(&[doc(1, 1, true)], 2500.0);
        p.put_done(DocId(1), 1, 2500.0, None);
        let plan = p.observe(&[doc(1, 2, false)], 3000.0);
        assert_eq!(plan.remove, vec!["doc-1".to_string()]);
        assert!(plan.put.is_empty());
        assert_eq!(p.observe(&[doc(1, 2, false)], 4000.0), Plan::default(), "removed only once");
        // Edited again: same key, put again.
        p.observe(&[doc(1, 3, true)], 5000.0);
        let plan = p.observe(&[doc(1, 3, true)], 7500.0);
        assert_eq!(plan.put, vec![DocId(1)]);
        assert_eq!(p.key(DocId(1)), Some("doc-1"));
    }

    #[test]
    fn nothing_is_removed_for_a_document_never_put() {
        let mut p = Planner::default();
        p.observe(&[doc(1, 1, true)], 0.0);
        assert!(p.observe(&[doc(1, 2, false)], 1000.0).remove.is_empty());
        p.observe(&[doc(1, 3, true)], 2000.0);
        assert!(p.observe(&[], 3000.0).remove.is_empty(), "closed before its first put");
    }

    #[test]
    fn closing_a_document_removes_its_draft() {
        let mut p = Planner::default();
        p.observe(&[doc(1, 1, true), doc(2, 1, true)], 0.0);
        let plan = p.observe(&[doc(1, 1, true), doc(2, 1, true)], 2500.0);
        assert_eq!(plan.put.len(), 2);
        p.put_done(DocId(1), 1, 2500.0, None);
        p.put_done(DocId(2), 1, 2500.0, None);
        let plan = p.observe(&[doc(2, 1, true)], 3000.0);
        assert_eq!(plan.remove, vec!["doc-1".to_string()]);
        assert_eq!(p.observe(&[doc(2, 1, true)], 4000.0), Plan::default());
    }

    #[test]
    fn an_adopted_key_is_kept_and_counts_as_drafted() {
        let mut p = Planner::default();
        p.adopt(DocId(4), "doc-9".into(), 3, 0.0);
        assert_eq!(p.observe(&[doc(4, 3, true)], 10_000.0), Plan::default(), "restored bytes aren't put back");
        p.observe(&[doc(4, 4, true)], 11_000.0);
        assert_eq!(p.observe(&[doc(4, 4, true)], 13_500.0).put, vec![DocId(4)]);
        assert_eq!(p.key(DocId(4)), Some("doc-9"));
        p.put_done(DocId(4), 4, 13_500.0, None);
        assert_eq!(p.observe(&[], 14_000.0).remove, vec!["doc-9".to_string()]);
    }

    #[test]
    fn keys_are_never_shared_or_reused() {
        let mut p = Planner::default();
        // A restore adopted "doc-2" before document 2 of this page existed.
        p.adopt(DocId(1), "doc-2".into(), 1, 0.0);
        p.observe(&[doc(1, 1, true), doc(2, 1, true)], 0.0);
        assert_eq!(p.key(DocId(2)), Some("doc-2-2"));
        // A restore adopting a key a live document uses: the live one moves and is put again.
        p.observe(&[doc(1, 1, true), doc(2, 1, true)], 2500.0);
        p.put_done(DocId(2), 1, 2500.0, None);
        p.adopt(DocId(3), "doc-2-2".into(), 5, 3000.0);
        let moved = p.key(DocId(2)).map(str::to_string);
        assert!(moved.as_deref().is_some_and(|k| k != "doc-2-2" && k != "doc-2"), "{moved:?}");
        let plan = p.observe(&[doc(1, 1, true), doc(2, 1, true), doc(3, 5, true)], 3000.0);
        assert_eq!(plan.put, vec![DocId(2)]);
        assert!(plan.remove.is_empty(), "the shared key belongs to the restored document now");
        // A closed document's key isn't handed to a later one.
        p.observe(&[doc(1, 1, true), doc(3, 5, true)], 4000.0);
        p.observe(&[doc(1, 1, true), doc(3, 5, true), doc(2, 1, true)], 5000.0);
        assert_ne!(p.key(DocId(2)), moved.as_deref());
    }

    #[test]
    fn a_refused_put_is_retried_after_a_pause_not_every_frame() {
        let mut p = Planner::default();
        p.observe(&[doc(1, 1, true)], 0.0);
        assert_eq!(p.observe(&[doc(1, 1, true)], 2500.0).put, vec![DocId(1)]);
        p.put_failed(DocId(1), 2500.0);
        assert!(p.observe(&[doc(1, 1, true)], 2600.0).put.is_empty());
        assert_eq!(p.observe(&[doc(1, 1, true)], 5000.0).put, vec![DocId(1)]);
    }

    #[test]
    fn unput_ignores_timing_and_open_typing() {
        let mut p = Planner::default();
        let typing = DocState { transient: true, ..doc(1, 1, true) };
        let states = [typing, doc(2, 1, false), doc(3, 1, true)];
        p.observe(&states, 0.0);
        let unput: Vec<(DocId, String)> = p.unput(&states).map(|(d, k)| (d.id, k.to_string())).collect();
        assert_eq!(unput, vec![(DocId(1), "doc-1".to_string()), (DocId(3), "doc-3".to_string())]);
        p.put_done(DocId(3), 1, 0.0, None);
        assert_eq!(p.unput(&states).count(), 1);
    }

    #[derive(Default)]
    struct Log {
        available: bool,
        now: f64,
        readies: usize,
        puts: Vec<(String, String, String, Vec<u8>)>,
        removes: Vec<String>,
        refuse: bool,
    }

    #[derive(Clone, Default)]
    struct Fake(Rc<RefCell<Log>>);

    impl DraftSink for Fake {
        fn available(&self) -> bool {
            self.0.borrow().available
        }
        fn ready(&self) -> Result<(), String> {
            self.0.borrow_mut().readies += 1;
            Ok(())
        }
        fn put(&self, key: &str, name: &str, format: &str, bytes: &[u8]) -> Result<(), String> {
            let mut log = self.0.borrow_mut();
            if log.refuse {
                return Err("QuotaExceededError".into());
            }
            log.puts.push((key.into(), name.into(), format.into(), bytes.to_vec()));
            Ok(())
        }
        fn remove(&self, key: &str) -> Result<(), String> {
            self.0.borrow_mut().removes.push(key.into());
            Ok(())
        }
        fn now_ms(&self) -> f64 {
            self.0.borrow().now
        }
    }

    impl Fake {
        fn at(&self, now: f64) -> &Self {
            self.0.borrow_mut().now = now;
            self
        }
    }

    const FORM: &[u8] = include_bytes!("../tests/data/form.pdf");

    fn app_with(fake: &Fake) -> (PdfCraftApp, DraftInbox) {
        fake.0.borrow_mut().available = true;
        let inbox = DraftInbox::default();
        let mut app = PdfCraftApp::new();
        app.drafts = Some(Drafts::new(Box::new(fake.clone()), inbox.clone()));
        (app, inbox)
    }

    fn edit(app: &mut PdfCraftApp, value: &str) {
        use pdfcraft_engine::{Edit, FieldValue};
        assert!(app.apply_edit(Edit::SetFieldValue { name: "name".into(), value: FieldValue::Text(value.into()) }));
    }

    fn restore(inbox: &DraftInbox, key: &str, name: &str, format: &str, bytes: Vec<u8>) {
        inbox.lock().unwrap().push(DraftRestore { key: key.into(), name: name.into(), format: format.into(), bytes });
    }

    #[test]
    fn without_the_store_nothing_happens() {
        let fake = Fake::default();
        let (mut app, _) = app_with(&fake);
        fake.0.borrow_mut().available = false;
        app.open_bytes("a.pdf", None, FORM.to_vec()).unwrap();
        edit(&mut app, "x");
        assert_eq!(app.drafts_step(), None);
        assert_eq!(fake.at(60_000.0).0.borrow().readies, 0);
        assert_eq!(app.drafts_step(), None);
        assert!(fake.0.borrow().puts.is_empty());
    }

    #[test]
    fn edits_are_put_after_the_idle_wait_and_removed_once_saved() {
        let fake = Fake::default();
        let (mut app, _) = app_with(&fake);
        app.open_bytes("Report", None, FORM.to_vec()).unwrap();
        app.drafts_step();
        assert_eq!(fake.0.borrow().readies, 1);
        edit(&mut app, "Ada");
        fake.at(1000.0);
        assert!(app.drafts_step().is_some());
        assert!(fake.0.borrow().puts.is_empty());
        fake.at(3500.0);
        app.drafts_step();
        let id = app.views[0].id;
        {
            let log = fake.0.borrow();
            assert_eq!(log.readies, 1, "ready() is called once");
            let (key, name, format, bytes) = log.puts.first().expect("put after the idle wait");
            assert_eq!((key.as_str(), name.as_str(), format.as_str()), (format!("doc-{}", id.0).as_str(), "Report.pdf", "pdf"));
            assert_eq!(bytes, app.session.get(id).unwrap().bytes.as_ref());
        }
        // A save on the web (download) settles at once.
        let bytes = app.session.save_bytes(id).unwrap();
        app.session.mark_saved(id, bytes, None).unwrap();
        fake.at(4000.0);
        app.drafts_step();
        fake.at(5000.0);
        app.drafts_step();
        assert_eq!(fake.0.borrow().removes, vec![format!("doc-{}", id.0)]);
    }

    #[test]
    fn draft_autosave_and_explicit_save_preserve_undo() {
        let fake = Fake::default();
        let (mut app, _) = app_with(&fake);
        app.open_bytes("Report", None, FORM.to_vec()).unwrap();
        let id = app.views[0].id;
        let before = app.session.get(id).unwrap().info.fields.iter().find(|f| f.name == "name").unwrap().value.clone();
        edit(&mut app, "Ada");
        app.drafts_step();
        fake.at(3500.0);
        app.drafts_step();
        assert_eq!(fake.0.borrow().puts.len(), 1);
        let history = app.session.get(id).unwrap().undo_history();
        assert_eq!(history.len(), 1);
        assert_eq!(app.session.autosave_snapshots().len(), 1);
        let bytes = app.session.save_bytes(id).unwrap();
        app.session.mark_saved(id, bytes, None).unwrap();
        assert!(!app.session.get(id).unwrap().dirty);
        assert_eq!(app.session.get(id).unwrap().undo_history(), history);
        app.session.undo(id).unwrap();
        assert_eq!(app.session.get(id).unwrap().info.fields.iter().find(|f| f.name == "name").unwrap().value.clone(), before);
        assert!(app.session.get(id).unwrap().dirty);
    }

    #[test]
    fn closing_without_saving_removes_the_draft() {
        let fake = Fake::default();
        let (mut app, _) = app_with(&fake);
        app.open_bytes("a.pdf", None, FORM.to_vec()).unwrap();
        edit(&mut app, "Ada");
        app.drafts_step();
        fake.at(2500.0);
        app.drafts_step();
        assert_eq!(fake.0.borrow().puts.len(), 1);
        app.close_tab(0);
        app.drafts_step();
        assert_eq!(fake.0.borrow().removes.len(), 1);
    }

    #[test]
    fn uncommitted_typing_is_not_drafted_until_committed() {
        let fake = Fake::default();
        let (mut app, _) = app_with(&fake);
        app.open_bytes("a.pdf", None, FORM.to_vec()).unwrap();
        edit(&mut app, "Ada");
        app.views[0].fill_text =
            Some(crate::fill_sign::TypeBox { page: 0, at: [10.0, 10.0], text: "typing".into(), size: crate::fill_sign::TEXT_SIZE, focus: false });
        app.drafts_step();
        fake.at(60_000.0);
        assert_eq!(app.drafts_step(), None);
        assert!(fake.0.borrow().puts.is_empty());
        // The page going away still gets the committed edits.
        let cell = app.drafts.as_ref().unwrap().flush_cell();
        cell.borrow_mut().flush(&fake.clone());
        assert_eq!(fake.0.borrow().puts.len(), 1);
        // Recorded on the next frame: nothing more to put.
        app.views[0].fill_text = None;
        fake.at(70_000.0);
        assert_eq!(app.drafts_step(), None);
        assert_eq!(fake.0.borrow().puts.len(), 1);
        assert!(cell.borrow().pending.is_empty());
    }

    #[test]
    fn a_flush_puts_each_pending_draft_once() {
        let fake = Fake::default();
        fake.0.borrow_mut().available = true;
        let mut flush = Flush {
            pending: vec![FlushItem { doc: DocId(1), generation: 7, key: "doc-1".into(), name: "a.pdf".into(), bytes: Arc::new(vec![1, 2]) }],
            flushed: Vec::new(),
        };
        flush.flush(&fake);
        flush.flush(&fake);
        assert_eq!(fake.0.borrow().puts, vec![("doc-1".into(), "a.pdf".into(), "pdf".into(), vec![1, 2])]);
        assert_eq!(flush.flushed, vec![(DocId(1), 7)]);
    }

    #[test]
    fn a_refused_put_is_logged_and_retried_later() {
        let fake = Fake::default();
        let (mut app, _) = app_with(&fake);
        app.open_bytes("a.pdf", None, FORM.to_vec()).unwrap();
        edit(&mut app, "Ada");
        app.drafts_step();
        fake.0.borrow_mut().refuse = true;
        fake.at(2500.0);
        app.drafts_step();
        fake.0.borrow_mut().refuse = false;
        fake.at(2600.0);
        app.drafts_step();
        assert!(fake.0.borrow().puts.is_empty());
        fake.at(5000.0);
        app.drafts_step();
        assert_eq!(fake.0.borrow().puts.len(), 1);
    }

    #[test]
    fn a_restored_draft_opens_unsaved_under_its_key() {
        let fake = Fake::default();
        let (mut app, inbox) = app_with(&fake);
        // A dead tab's key that this page would also give its own document 2.
        restore(&inbox, "doc-2", "Hợp đồng.pdf", "pdf", FORM.to_vec());
        restore(&inbox, "doc-x", "sheet.xlsx", "xlsx", b"PK".to_vec());
        app.open_draft_restores();
        assert!(inbox.lock().unwrap().is_empty());
        assert_eq!(app.views.len(), 1, "only the PDF draft opens");
        let id = app.views[0].id;
        let doc = app.session.get(id).unwrap();
        assert!(doc.dirty && doc.path.is_none());
        assert_eq!(doc.name, "Hợp đồng.pdf");
        assert!(app.has_unsaved_work(0), "the reload warning covers it");
        fake.at(60_000.0);
        app.drafts_step();
        assert!(fake.0.borrow().puts.is_empty(), "the restored bytes are already in the store");
        edit(&mut app, "Ada");
        app.drafts_step();
        fake.at(62_500.0);
        app.drafts_step();
        assert_eq!(fake.0.borrow().puts.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["doc-2"]);
        app.open_bytes("b.pdf", None, FORM.to_vec()).unwrap();
        app.drafts_step();
        let other = app.views[1].id;
        assert_eq!(other, DocId(2));
        assert_ne!(app.drafts.as_ref().unwrap().planner.key(other), Some("doc-2"));
    }

    /// `bytes` protected with an open password (AES-256).
    fn encrypted(bytes: &[u8], password: &str) -> Vec<u8> {
        let mut doc = pdfcraft_cos::Document::open(Arc::new(bytes.to_vec())).unwrap();
        doc.set_encryption(&pdfcraft_cos::NewEncryption {
            algorithm: pdfcraft_cos::Algorithm::Aes256,
            user_password: password,
            owner_password: "owner",
            permissions: -1,
            encrypt_metadata: true,
            seed: [2; 32],
        })
        .unwrap();
        pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
    }

    #[test]
    fn an_encrypted_draft_is_restored_once_its_password_is_given() {
        let fake = Fake::default();
        let (mut app, inbox) = app_with(&fake);
        restore(&inbox, "doc-5", "secret.pdf", "pdf", encrypted(FORM, "pw"));
        restore(&inbox, "doc-6", "plain.pdf", "pdf", FORM.to_vec());
        app.open_draft_restores();
        assert!(app.password_prompt.is_some());
        assert_eq!(inbox.lock().unwrap().len(), 1, "the next draft waits for the prompt");
        app.submit_password(Some("wrong".into()));
        assert!(app.password_prompt.as_ref().is_some_and(|p| p.draft_key.as_deref() == Some("doc-5")));
        app.submit_password(Some("pw".into()));
        assert!(app.password_prompt.is_none());
        let id = app.views[0].id;
        assert!(app.session.get(id).unwrap().dirty);
        assert_eq!(app.drafts.as_ref().unwrap().planner.key(id), Some("doc-5"));
        app.open_draft_restores();
        assert_eq!(app.views.len(), 2);
        assert_eq!(app.drafts.as_ref().unwrap().planner.key(app.views[1].id), Some("doc-6"));
        fake.at(60_000.0);
        app.drafts_step();
        assert!(fake.0.borrow().puts.is_empty());
    }

    #[test]
    fn a_cancelled_password_leaves_the_draft_unopened() {
        let fake = Fake::default();
        let (mut app, inbox) = app_with(&fake);
        restore(&inbox, "doc-5", "secret.pdf", "pdf", encrypted(FORM, "pw"));
        app.open_draft_restores();
        app.submit_password(None);
        assert!(app.views.is_empty());
        // The next unlocked document is an ordinary open, not the draft.
        app.open_bytes("other.pdf", None, encrypted(FORM, "pw2")).unwrap();
        app.submit_password(Some("pw2".into()));
        let id = app.views[0].id;
        assert!(!app.session.get(id).unwrap().dirty);
        app.drafts_step();
        assert_ne!(app.drafts.as_ref().unwrap().planner.key(id), Some("doc-5"));
    }
}
