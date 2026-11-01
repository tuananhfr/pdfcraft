//! One frame of co-editing: the shell's calls, lock answers, the user's changes, letting regions
//! go, and the selection.

use std::collections::HashSet;

use pdfcraft_engine::live::{Applied, Blocked, LiveError, LocalChange, Region};
use pdfcraft_engine::{AnnotId, DocId};

use super::{ConflictPrompt, FOCUS_IDLE_MS, Held, IDLE_END_MS, Live, LiveEvent, SELECTION_MS};
use crate::PdfCraftApp;

impl Live {
    /// Ask the shell for `region`'s lock, unless it is held or asked for already.
    pub(super) fn ask(&mut self, region: &str) {
        if self.held.contains_key(region) || !self.asking.insert(region.to_owned()) {
            return;
        }
        let answers = self.answers.clone();
        let owned = region.to_owned();
        self.host.region_will_edit(
            region,
            Box::new(move |granted| {
                if let Ok(mut q) = answers.lock() {
                    q.push((owned, granted));
                }
            }),
        );
    }

    /// This page's member id, when the shell's presence says which one it is.
    fn me(&self) -> Option<&str> {
        self.members.iter().find(|m| m.me).map(|m| m.id.as_str())
    }

    fn blocked(&self) -> Blocked {
        let mut blocked = Blocked { any: !self.others.is_empty(), ..Blocked::default() };
        for region in self.others.keys() {
            match Region::parse(region) {
                Some(Region::Document) => blocked.document = true,
                Some(r) => {
                    blocked.regions.insert(r);
                }
                None => {}
            }
        }
        blocked
    }

    fn busy(&self) -> HashSet<String> {
        self.waiting.iter().flat_map(|c| c.regions.iter().map(Region::to_string)).collect()
    }
}

impl PdfCraftApp {
    /// Co-editing for this frame (web, `erpHost` v3 only).
    pub(crate) fn live_tick(&mut self, ctx: &egui::Context) {
        if self.live_step() {
            ctx.request_repaint_after(std::time::Duration::from_millis(SELECTION_MS as u64));
        }
        self.conflict_prompt(ctx);
    }

    /// One step; `true` while something waits on time (a lock to let go, a selection to send).
    pub(crate) fn live_step(&mut self) -> bool {
        let Some(id) = self.bind_live_doc() else { return false };
        let Some(now) = self.online.as_ref().map(|o| o.host.now_ms()) else { return false };
        self.live_events(id);
        self.live_answers(id, now);
        self.live_outgoing(id, now);
        self.live_release(id, now);
        let selection_pending = self.live_selection(id, now);
        self.refresh_live_marks();
        selection_pending || self.online.as_ref().and_then(|o| o.live.as_ref()).is_some_and(|l| !l.held.is_empty() || !l.waiting.is_empty())
    }

    /// The live room is about the host document opened last: live mode follows it.
    fn bind_live_doc(&mut self) -> Option<DocId> {
        let online = self.online.as_ref()?;
        let doc = online.records.iter().filter(|(id, _)| self.views.iter().any(|v| v.id == **id)).max_by_key(|(_, r)| r.slot).map(|(id, _)| *id);
        let live = online.live.as_ref()?;
        if live.doc == doc {
            return doc;
        }
        let old = live.doc;
        if let Some(old) = old {
            let _ = self.session.set_live(old, false);
        }
        if let Some(new) = doc {
            let _ = self.session.set_live(new, true);
        }
        let live = self.online.as_mut()?.live.as_mut()?;
        live.reset();
        live.doc = doc;
        doc
    }

    fn live_events(&mut self, id: DocId) {
        let Some(live) = self.online.as_ref().and_then(|o| o.live.as_ref()) else { return };
        let events = live.inbox.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for event in events {
            match event {
                LiveEvent::ApplyRemote { region, payload, .. } => self.apply_remote_region(id, &region, &payload),
                LiveEvent::LockDenied(region) => self.live_denied(id, &region),
                other => {
                    let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
                    live.revision = live.revision.wrapping_add(1);
                    match other {
                        LiveEvent::Presence(members) => live.members = members,
                        LiveEvent::Locks(locks) => {
                            let me = live.me().map(str::to_owned);
                            live.others = locks
                                .into_iter()
                                .filter(|l| !l.mine && me.as_deref() != Some(l.holder.as_str()) && !live.held.contains_key(&l.region))
                                .map(|l| (l.region.clone(), l))
                                .collect();
                            let blocked = live.blocked();
                            self.session.set_live_blocked(id, blocked);
                        }
                        LiveEvent::Cursors(cursors) => {
                            let me = live.me().map(str::to_owned);
                            live.cursors = cursors.into_iter().filter(|c| me.as_deref() != Some(c.author.as_str())).collect();
                        }
                        LiveEvent::Leader(leader) => live.leader = leader,
                        LiveEvent::Conflict { region, reply } => live.conflicts.push(ConflictPrompt { region, reply }),
                        LiveEvent::ApplyRemote { .. } | LiveEvent::LockDenied(_) => {}
                    }
                }
            }
        }
    }

    /// Someone else's region: applied where the user's selection and page stay put.
    fn apply_remote_region(&mut self, id: DocId, region: &str, payload: &str) {
        let selected = self.selected_comment(id);
        let page = self.views.iter().find(|v| v.id == id).map(|v| v.current);
        let page_id = page.and_then(|p| self.session.page_ids(id).get(p).copied());
        // One this side can't apply is dropped, leaving the document as it was here.
        let Ok(applied) = self.session.apply_remote(id, region, payload) else { return };
        self.refresh_after_remote(id, &applied);
        let comment = selected.as_ref().and_then(|a| self.session.find_annotation(id, a));
        let now_at = page_id.and_then(|p| self.session.page_index(id, p));
        for view in self.views.iter_mut().filter(|v| v.id == id) {
            if selected.is_some() {
                view.comments.selected = comment;
            }
            if let Some(at) = now_at
                && Some(at) != page
            {
                view.go_to_page(at);
            }
        }
    }

    /// Redraw what a remote change touched.
    pub(crate) fn refresh_after_remote(&mut self, id: DocId, applied: &Applied) {
        let Some(doc) = self.session.get(id) else { return };
        for view in self.views.iter_mut().filter(|v| v.id == id) {
            match &applied.pages {
                Some(pages) => pages.iter().for_each(|p| view.page_changed(*p)),
                None => view.document_changed(&doc.info),
            }
        }
    }

    /// The id of the comment selected in document `id`'s view.
    fn selected_comment(&self, id: DocId) -> Option<AnnotId> {
        let (page, index) = self.views.iter().find(|v| v.id == id)?.comments.selected?;
        self.session.annotation_ids(id).into_iter().find(|a| a.page == page && a.index == index).map(|a| a.id)
    }

    fn live_answers(&mut self, id: DocId, now: f64) {
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
        let answers = live.answers.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        let mut denied = Vec::new();
        for (region, granted) in answers {
            live.asking.remove(&region);
            if granted {
                live.held.insert(region, Held { last_ms: now });
            } else {
                denied.push(region);
            }
        }
        for region in denied {
            self.live_denied(id, &region);
        }
    }

    /// The room refused `region` (or took it back): the user's steps on it waiting to be sent
    /// are taken back, and the user hears who holds it.
    pub(crate) fn live_denied(&mut self, id: DocId, region: &str) {
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
        live.held.remove(region);
        live.asking.remove(region);
        let (doomed, kept): (Vec<LocalChange>, Vec<LocalChange>) =
            std::mem::take(&mut live.waiting).into_iter().partition(|c| c.regions.iter().any(|r| r.to_string() == region));
        live.waiting = kept;
        for change in doomed.iter().rev() {
            let _ = self.session.revert_live_change(id, change);
        }
        if !doomed.is_empty()
            && let Some(doc) = self.session.get(id)
        {
            for view in self.views.iter_mut().filter(|v| v.id == id) {
                view.document_changed(&doc.info);
            }
        }
        // The inline editor stays open with what was typed: committing it is refused while someone
        // else holds the region, and the editor reopens with the draft.
        self.live_lock_notice(region);
    }

    /// "<name> is editing this part." for a region someone else holds.
    pub(crate) fn live_lock_notice(&mut self, region: &str) {
        let name = self
            .online
            .as_ref()
            .and_then(|o| o.live.as_ref())
            .and_then(|l| l.others.get(region).or_else(|| l.others.get("*")).and_then(|lock| l.name_of(&lock.holder, lock.name.as_ref())));
        match name {
            Some(name) => self.notify_fmt("{name} is editing this part.", &[("name", &name)]),
            None => self.notify_tr("Someone else is editing this part."),
        }
    }

    fn live_outgoing(&mut self, id: DocId, now: f64) {
        let changes = self.session.take_live_changes(id);
        let intent = self.live_intent(id);
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
        live.waiting.extend(changes);
        let fresh: Vec<String> = intent.iter().filter(|r| !live.intent.contains(*r)).cloned().collect();
        live.intent = intent;
        for region in live.busy().into_iter().chain(fresh) {
            live.ask(&region);
        }
        // A shell may answer at once.
        self.live_answers(id, now);
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
        let (ready, waiting): (Vec<LocalChange>, Vec<LocalChange>) =
            std::mem::take(&mut live.waiting).into_iter().partition(|c| c.regions.iter().all(|r| live.held.contains_key(&r.to_string())));
        live.waiting = waiting;
        let mut sent: HashSet<Region> = HashSet::new();
        let mut too_large = false;
        for change in ready {
            let mut payloads = Vec::new();
            let mut refused = false;
            for region in change.regions.iter().filter(|r| !sent.contains(*r)) {
                match self.session.live_payload(id, region) {
                    Ok(p) => payloads.push((region.clone(), p)),
                    Err(LiveError::TooLarge) => {
                        refused = true;
                        break;
                    }
                    // Nothing to send for it (the document went read-only meanwhile).
                    Err(_) => {}
                }
            }
            if refused {
                too_large = true;
                let _ = self.session.revert_live_change(id, &change);
                continue;
            }
            let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
            for (region, payload) in payloads {
                let key = region.to_string();
                live.host.region_changed(&key, &payload);
                live.remember_sent(&key, &payload);
                if let Some(h) = live.held.get_mut(&key) {
                    h.last_ms = now;
                }
                sent.insert(region);
            }
        }
        if too_large {
            if let Some(doc) = self.session.get(id) {
                for view in self.views.iter_mut().filter(|v| v.id == id) {
                    view.document_changed(&doc.info);
                }
            }
            self.notify_tr("This change is too large to share while editing together.");
        }
    }

    /// The regions the user is about to change: the field being filled, the comment being edited
    /// or replied to. Their locks are asked for once per focus, before the first keystroke.
    fn live_intent(&self, id: DocId) -> HashSet<String> {
        let mut out = HashSet::new();
        let Some(view) = self.views.iter().find(|v| v.id == id) else { return out };
        if let Some(f) = view.forms.focus.as_ref() {
            out.insert(Region::Field(f.name.clone()).to_string());
        }
        let comments = &view.comments;
        let editing = comments.editing.as_ref().map(|(p, i, _)| (*p, *i)).or(comments.selected.filter(|_| !comments.reply.is_empty()));
        if let Some((page, index)) = editing
            && let Some(a) = self.session.annotation_ids(id).into_iter().find(|a| a.page == page && a.index == index)
        {
            out.insert(Region::Annot(a.id).to_string());
        }
        out
    }

    /// The regions the user is in: the selected comment, the field being filled.
    fn live_focus(&self, id: DocId) -> HashSet<String> {
        let mut out = HashSet::new();
        if let Some(a) = self.selected_comment(id) {
            out.insert(Region::Annot(a).to_string());
        }
        if let Some(f) = self.views.iter().find(|v| v.id == id).and_then(|v| v.forms.focus.as_ref()) {
            out.insert(Region::Field(f.name.clone()).to_string());
        }
        out
    }

    /// Let go of the regions the user left: idle past [`IDLE_END_MS`], or past
    /// [`FOCUS_IDLE_MS`] while still selected.
    fn live_release(&mut self, id: DocId, now: f64) {
        let held = self.online.as_ref().and_then(|o| o.live.as_ref()).is_some_and(|l| !l.held.is_empty());
        if !held {
            return;
        }
        let focus = self.live_focus(id);
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
        let busy = live.busy();
        let ended: Vec<String> = live
            .held
            .iter()
            .filter(|(r, h)| !busy.contains(*r) && now - h.last_ms > if focus.contains(*r) { FOCUS_IDLE_MS } else { IDLE_END_MS })
            .map(|(r, _)| r.clone())
            .collect();
        for region in ended {
            live.held.remove(&region);
            live.host.region_did_end_edit(&region);
        }
    }

    /// Report where the user is: the selected comment or field, else the page shown (`page:<id>`).
    /// `true` while a change waits for the throttle.
    fn live_selection(&mut self, id: DocId, now: f64) -> bool {
        let focus = self.live_focus(id);
        let region = match focus.iter().min() {
            Some(r) => r.clone(),
            None => {
                let Some(page) = self.views.iter().find(|v| v.id == id).map(|v| v.current) else { return false };
                let Some(pid) = self.session.page_ids(id).get(page).copied() else { return false };
                format!("page:{pid}")
            }
        };
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return false };
        let selection = (region, String::new());
        if live.selection.as_ref() == Some(&selection) {
            return false;
        }
        if now - live.selection_sent_ms < SELECTION_MS {
            return true;
        }
        live.host.selection_changed(&selection.0, &selection.1);
        live.selection = Some(selection);
        live.selection_sent_ms = now;
        false
    }
}
