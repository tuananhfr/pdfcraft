//! The app's handling of host documents: opening what `open` delivered, Save and AutoSave
//! through the shell, applying its answers, and telling it about dirty and closed documents.

use std::sync::Arc;

use pdfcraft_engine::DocId;

use super::autosave::{MAX_WAKE_MS, Tick};
use super::wire::{Outgoing, Settled};
use super::{Acknowledged, After, FORMAT, PendingOpen, REASON_PARSER_FAILED, REASON_UNSUPPORTED, Record, SaveReason, SaveResult, Stop};
use crate::PdfCraftApp;
use crate::editing::SaveTarget;

impl Record {
    fn new(doc_id: String, slot: u64, version: Option<u64>) -> Self {
        Self {
            doc_id,
            slot,
            version,
            timer: Default::default(),
            stopped: None,
            sent: None,
            acked: None,
            retried: None,
            reported_dirty: false,
            replaced: false,
            queued: None,
            awaiting: None,
            signed_notice: false,
        }
    }
}

/// What one host document needs this frame, decided while its record is borrowed.
enum Action {
    None,
    Manual(After),
    Auto,
}

impl PdfCraftApp {
    /// Document `id` was opened through `erpHost`: it saves to the shell and never to `erpDrafts`.
    pub(crate) fn is_host_doc(&self, id: DocId) -> bool {
        self.online.as_ref().is_some_and(|o| o.records.contains_key(&id))
    }

    /// A save of host document `id` is in flight or waiting to be sent.
    pub(crate) fn host_save_pending(&self, id: DocId) -> bool {
        let Some(online) = self.online.as_ref() else { return false };
        online.records.get(&id).is_some_and(|r| r.queued.is_some() || online.wire.busy(&r.doc_id))
    }

    /// The server version host document `id` was opened at or last saved as.
    pub fn host_version(&self, id: DocId) -> Option<u64> {
        self.online.as_ref()?.records.get(&id)?.version
    }

    /// Open what the shell's `open` delivered, one password prompt at a time, and tell the shell
    /// how each ended (a password prompt counts as opened, as for `erpIntakeResult`).
    pub(crate) fn open_host_documents(&mut self) {
        let Some((inbox, host)) = self.online.as_ref().map(|o| (o.inbox.clone(), o.host.clone())) else { return };
        while self.password_prompt.is_none() {
            let next = inbox.lock().ok().and_then(|mut q| (!q.is_empty()).then(|| q.remove(0)));
            let Some(doc) = next else { break };
            if !doc.format.eq_ignore_ascii_case(FORMAT) {
                host.opened(&doc.doc_id, false, Some(REASON_UNSUPPORTED));
                continue;
            }
            self.startup_superseded = true;
            let title = if doc.title.trim().is_empty() { "document.pdf".to_string() } else { doc.title };
            let pending = PendingOpen { doc_id: doc.doc_id, version: doc.version };
            // Not `open_bytes`: the server's document is a PDF, never a file to convert.
            match self.try_open(&title, None, Arc::new(doc.bytes), None, true) {
                Ok(()) if self.password_prompt.is_none() => {
                    host.opened(&pending.doc_id, true, None);
                    self.finish_host_open(pending);
                }
                Ok(()) => {
                    host.opened(&pending.doc_id, true, None);
                    if let Some(prompt) = self.password_prompt.as_mut() {
                        prompt.host_doc = Some(pending);
                    }
                }
                Err(e) => {
                    // Unreadable bytes and the memory budget alike: the shell has one sentence for both.
                    host.opened(&pending.doc_id, false, Some(REASON_PARSER_FAILED));
                    self.notify_fmt("Couldn't open {name}: {e}", &[("name", &title), ("e", &e)]);
                }
            }
        }
    }

    /// The newest tab is host document `pending`. A reload of a document already open (the shell
    /// after a conflict) takes the earlier tab's place.
    pub(crate) fn finish_host_open(&mut self, pending: PendingOpen) {
        let Some(id) = self.views.last().map(|v| v.id) else { return };
        let Some(online) = self.online.as_mut() else { return };
        online.next_slot = online.next_slot.wrapping_add(1);
        let mut record = Record::new(pending.doc_id, online.next_slot, pending.version);
        let earlier = online.records.iter_mut().find(|(other, r)| **other != id && r.doc_id == record.doc_id).map(|(other, r)| {
            r.replaced = true;
            (*other, r.reported_dirty)
        });
        // The shell last heard the earlier tab's state; the next frame says the new one's.
        if let Some((_, dirty)) = earlier {
            record.reported_dirty = dirty;
        }
        online.records.insert(id, record);
        if let Some((earlier, _)) = earlier
            && let Some(index) = self.views.iter().position(|v| v.id == earlier)
        {
            self.close_tab(index);
            if let Some(from) = self.views.iter().position(|v| v.id == id) {
                self.move_tab(from, index);
            }
            self.active = self.views.iter().position(|v| v.id == id);
        }
    }

    /// The password prompt for host document `pending` was cancelled: nothing of it is open.
    pub(crate) fn host_open_cancelled(&self, pending: &PendingOpen) {
        if let Some(online) = self.online.as_ref() {
            online.host.closed(&pending.doc_id);
        }
    }

    /// Document `id`'s tab is closing: the shell hears it, unless a reload replaced it.
    pub(crate) fn forget_host_doc(&mut self, id: DocId) {
        let Some(online) = self.online.as_mut() else { return };
        if let Some(record) = online.records.remove(&id)
            && !record.replaced
        {
            online.host.closed(&record.doc_id);
        }
    }

    /// Save (Ctrl+S, "Lưu", a save prompt's Save) of host document `id`: to the shell, never a
    /// picker, after a save already in flight (rule 3). Returns `true` only when there was nothing
    /// to send, and `after` has run; otherwise `after` runs once the shell answers `saved`.
    pub(crate) fn online_save(&mut self, id: DocId, after: After) -> bool {
        let Some(online) = self.online.as_mut() else { return false };
        let Some(record) = online.records.get_mut(&id) else { return false };
        if online.wire.busy(&record.doc_id) {
            record.queued = Some(after);
            return false;
        }
        self.send_manual(id, after)
    }

    fn send_manual(&mut self, id: DocId, after: After) -> bool {
        if !self.flatten_fill_sign_for_save(id) {
            return false;
        }
        if !self.signature_write_confirmed(id) {
            // What the confirmation covered has changed since (flattening, or a queued save).
            self.hold_breaking_save(id, SaveTarget::InPlace, after);
            return false;
        }
        let Some(doc) = self.session.get(id) else { return false };
        if !doc.dirty {
            after(self);
            return true;
        }
        self.send_save(id, SaveReason::Manual, Some(after));
        false
    }

    /// Send document `id` as it is now. `after` waits for its `saved`.
    fn send_save(&mut self, id: DocId, reason: SaveReason, after: Option<After>) {
        let bytes = self.session.save_copy_bytes(id);
        let Some(doc) = self.session.get(id) else { return };
        let (name, generation) = (doc.name.clone(), doc.edit_generation());
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                // Not again until the document changes.
                if let Some(record) = self.online.as_mut().and_then(|o| o.records.get_mut(&id)) {
                    record.sent = Some(generation);
                }
                self.save_failed(id, "Couldn't save {name}: {e}", &[("name", &name), ("e", &e.to_string())]);
                return;
            }
        };
        let Some(online) = self.online.as_mut() else { return };
        let Some(record) = online.records.get_mut(&id) else { return };
        record.sent = Some(generation);
        record.timer.sent();
        if let Some(after) = after {
            record.awaiting = Some((generation, after));
        }
        let out = Outgoing { doc_id: record.doc_id.clone(), slot: record.slot, revision: generation, bytes, reason };
        online.wire.dispatch(out);
    }

    /// Apply the shell's answers, then report, send and plan for each host document (every frame).
    pub(crate) fn online_tick(&mut self, ctx: &egui::Context) {
        if let Some(ms) = self.online_step() {
            // `as` saturates (NaN gives 0), so this never panics.
            ctx.request_repaint_after(std::time::Duration::from_millis(ms.clamp(0.0, MAX_WAKE_MS) as u64));
        }
    }

    /// One tick; returns how soon the next one is needed.
    pub(crate) fn online_step(&mut self) -> Option<f64> {
        let online = self.online.as_ref()?;
        let now = online.host.now_ms();
        let acks = online.acks.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for settled in online.wire.take_settled() {
            self.apply_answer(settled);
        }
        for ack in acks {
            self.apply_acknowledged(ack);
        }
        let ids: Vec<DocId> = self.views.iter().map(|v| v.id).filter(|id| self.is_host_doc(*id)).collect();
        let mut wake: Option<f64> = None;
        let mut flushable = Vec::new();
        for id in ids {
            if let Some(ms) = self.step_host_doc(id, now, &mut flushable) {
                wake = Some(wake.map_or(ms, |w| w.min(ms)));
            }
        }
        if let Some(online) = self.online.as_ref() {
            online.wire.set_flushable(flushable);
        }
        wake.map(|w| w.min(MAX_WAKE_MS))
    }

    fn apply_answer(&mut self, settled: Settled) {
        let Some(online) = self.online.as_mut() else { return };
        let Some((&id, record)) = online.records.iter_mut().find(|(_, r)| r.slot == settled.slot) else { return };
        if record.sent.is_none_or(|sent| sent < settled.revision) {
            record.sent = Some(settled.revision);
        }
        let awaiting = record.awaiting.take_if(|(revision, _)| *revision == settled.revision);
        match settled.result {
            SaveResult::Saved { version } => self.stored(id, settled.revision, version, Some(settled.bytes), awaiting),
            // The shell shows the conflict and decides; nothing is sent again by itself (rule 4).
            SaveResult::Conflict { .. } => record.stopped = Some(Stop::Conflict),
            // The shell keeps the bytes, retries and shows the state (rule 5); `acknowledged` may follow.
            SaveResult::Offline | SaveResult::Failed { retryable: true, .. } => record.retried = Some((settled.revision, settled.bytes)),
            SaveResult::Failed { retryable: false, message } => {
                record.stopped = Some(Stop::Failed);
                let message = if message.trim().is_empty() { tl!("Save failed").to_string() } else { message };
                self.save_failed_message(id, message);
            }
        }
    }

    /// The shell's own retry stored `ack.revision`: a `saved` of that revision. Only revisions this
    /// opening left to the shell to retry count; anything else changes nothing.
    fn apply_acknowledged(&mut self, ack: Acknowledged) {
        let Some(online) = self.online.as_mut() else { return };
        let Some((&id, record)) = online.records.iter_mut().find(|(_, r)| r.doc_id == ack.doc_id && !r.replaced) else { return };
        let bytes = match &record.retried {
            Some((retried, bytes)) if *retried == ack.revision => Some(bytes.clone()),
            Some((retried, _)) if ack.revision < *retried => None,
            _ => return,
        };
        self.stored(id, ack.revision, ack.version, bytes, None);
    }

    /// The shell stored `revision` of document `id`. It is marked saved only if it is still that
    /// revision (rule 1); `after` runs then.
    fn stored(&mut self, id: DocId, revision: u64, version: Option<u64>, bytes: Option<Arc<Vec<u8>>>, awaiting: Option<(u64, After)>) {
        let Some(record) = self.online.as_mut().and_then(|o| o.records.get_mut(&id)) else { return };
        if record.acked.is_some_and(|acked| revision < acked) {
            return;
        }
        record.acked = Some(revision);
        if version.is_some() {
            record.version = version;
        }
        record.stopped = None;
        if record.retried.as_ref().is_some_and(|(retried, _)| *retried <= revision) {
            record.retried = None;
        }
        // Edits made since that revision stay unsaved.
        if self.session.get(id).is_none_or(|d| d.edit_generation() != revision) {
            return;
        }
        let Some(bytes) = bytes else { return };
        if let Err(e) = self.session.mark_saved(id, bytes, None) {
            self.save_failed(id, "Saved, but reopening failed: {e}", &[("e", &e.to_string())]);
            return;
        }
        if let Some(doc) = self.session.get(id)
            && let Some(view) = self.views.iter_mut().find(|v| v.id == id)
        {
            view.document_changed(&doc.info);
        }
        if let Some((_, after)) = awaiting {
            after(self);
        }
    }

    fn step_host_doc(&mut self, id: DocId, now: f64, flushable: &mut Vec<Outgoing>) -> Option<f64> {
        let index = self.views.iter().position(|v| v.id == id)?;
        let unsaved = self.has_unsaved_work(index);
        let typing = self.has_uncommitted_input(index);
        let confirmed = self.signature_write_confirmed(id);
        let doc = self.session.get(id)?;
        let (generation, changed, signed, bytes) = (doc.edit_generation(), doc.dirty, doc.is_signed(), doc.bytes.clone());
        let online = self.online.as_mut()?;
        let record = online.records.get_mut(&id)?;
        if unsaved != record.reported_dirty {
            record.reported_dirty = unsaved;
            online.host.dirty(&record.doc_id, unsaved);
        }
        let busy = online.wire.busy(&record.doc_id);
        // A signature is never invalidated without asking: only a confirmed Save sends a signed
        // document, and flush only what such a Save confirmed. A revision left to the shell's retry
        // is handed over again: flush covers everything not acknowledged.
        if (!signed || confirmed) && changed && record.stopped.is_none() && record.acked != Some(generation) {
            flushable.push(Outgoing { doc_id: record.doc_id.clone(), slot: record.slot, revision: generation, bytes, reason: SaveReason::Flush });
        }
        let notice = signed && changed && !std::mem::replace(&mut record.signed_notice, true);
        let unsent = changed && !signed && record.stopped.is_none() && record.sent != Some(generation);
        let mut wake = None;
        let action = match (busy, record.queued.take()) {
            (false, Some(after)) => Action::Manual(after),
            (true, queued) => {
                record.queued = queued;
                record.timer.tick(generation, unsent, typing, now);
                Action::None
            }
            (false, None) => match record.timer.tick(generation, unsent, typing, now) {
                Tick::Send => Action::Auto,
                Tick::Wait(ms) => {
                    wake = Some(ms);
                    Action::None
                }
                Tick::Idle => Action::None,
            },
        };
        if notice {
            self.notify_tr("AutoSave is off for signed documents. Choose Save to save your changes.");
        }
        match action {
            Action::Manual(after) => {
                self.send_manual(id, after);
            }
            Action::Auto => self.send_save(id, SaveReason::Auto, None),
            Action::None => {}
        }
        wake
    }
}
