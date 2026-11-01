//! Co-editing by region (ERPCons office, `erpHost` v3). A document in live mode records which
//! [`Region`]s each edit changed, so the app can lock them and send their new content, applies
//! other people's region payloads ([`Session::apply_remote`]) without touching any other region,
//! and undoes per user: undo and redo only put back the regions of the user's own step, and skip
//! a step whose region someone else changed since (as Google Docs does), so they never revert
//! another person's work. Without live mode nothing here runs.

mod codec;
pub mod region;
mod regions;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pdfcraft_cos::Document as Cos;

pub use region::{MAX_REGION_BYTES, Region};
pub use regions::MAX_PAYLOAD_BYTES;
use regions::{PageSource, View};

use crate::stable_ids::{self, IdSource, PageId, PageIds};
use crate::{DocId, Edit, EditError, Label, Scope, Session, guard};

/// Why a payload could not be made or applied.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LiveError {
    #[error("the document is not open for co-editing")]
    NotLive,
    #[error("no such document")]
    NoDocument,
    #[error("the payload comes from a newer version of the editor")]
    Version,
    #[error("the payload can't be read ({0})")]
    Malformed(String),
    #[error("the change is too large to share")]
    TooLarge,
    #[error("the change could not be applied: {0}")]
    Apply(String),
}

/// What other people hold, as the shell's lock table says: an edit changing one of these is
/// refused before it is made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blocked {
    pub regions: HashSet<Region>,
    /// Someone else holds `*`.
    pub document: bool,
    /// Someone else holds a lock: `*` can't be had.
    pub any: bool,
}

impl Blocked {
    pub fn blocks(&self, region: &Region) -> bool {
        self.document || self.regions.contains(region) || (*region == Region::Document && self.any)
    }
}

/// How a local change came about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Edit,
    Undo,
    Redo,
}

/// One step of the user's that changed `regions`: the app locks them, then sends their content.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalChange {
    /// The step's transaction id (`HistoryEntry::transaction_id`).
    pub step: u64,
    pub kind: ChangeKind,
    pub regions: Vec<Region>,
}

/// What [`Session::apply_remote`] changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub region: Region,
    /// Pages (0-based, as the document is now) whose look changed; `None` for all of them.
    pub pages: Option<Vec<usize>>,
    /// `false` when the payload had nowhere to go (its page is gone here).
    pub changed: bool,
}

#[derive(Clone, Debug)]
struct Step {
    regions: Vec<Region>,
    /// The state's epoch when the step last moved between undo and redo.
    epoch: u64,
}

/// Live mode of one document.
pub(crate) struct LiveState {
    /// Counts remote changes (and steps taken out of the middle of the history): a step taken at
    /// the current epoch can be undone by swapping whole states.
    epoch: u64,
    /// The epoch at which someone else last changed each region.
    touched: HashMap<Region, u64>,
    /// The epoch at which someone else last replaced the whole document.
    everything: u64,
    steps: HashMap<u64, Step>,
    outbox: Vec<LocalChange>,
    blocked: Blocked,
    /// The pages the room has, as far as this side knows: other pages travel with their content.
    room_pages: HashSet<PageId>,
    skipped: usize,
}

impl LiveState {
    fn new(pages: &[PageId]) -> Self {
        Self {
            epoch: 0,
            touched: HashMap::new(),
            everything: 0,
            steps: HashMap::new(),
            outbox: Vec::new(),
            blocked: Blocked::default(),
            room_pages: pages.iter().copied().collect(),
            skipped: 0,
        }
    }

    /// Steps from before live mode count as changing everything.
    fn step(&self, transaction: u64) -> Step {
        self.steps.get(&transaction).cloned().unwrap_or(Step { regions: vec![Region::Document], epoch: 0 })
    }

    /// Someone else changed one of the step's regions since it was taken.
    fn is_stale(&self, step: &Step) -> bool {
        self.everything > step.epoch
            || step.regions.iter().any(|r| *r == Region::Document && self.epoch > step.epoch)
            || step.regions.iter().any(|r| self.touched.get(r).is_some_and(|e| *e > step.epoch))
    }

    pub(crate) fn record(&mut self, step: u64, kind: ChangeKind, regions: Vec<Region>) {
        self.steps.insert(step, Step { regions: regions.clone(), epoch: self.epoch });
        if !regions.is_empty() {
            self.outbox.push(LocalChange { step, kind, regions });
        }
    }
}

/// Which regions an edit can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Comments,
    Fields,
    Pages,
    Document,
    /// Every kind (other people's changes).
    All,
}

fn class_of(edit: &Edit, xfa: bool) -> Class {
    match edit {
        Edit::Batch { edits, .. } => {
            let mut classes = edits.iter().map(|e| class_of(e, xfa));
            let first = classes.next().unwrap_or(Class::Document);
            if classes.all(|c| c == first) { first } else { Class::Document }
        }
        Edit::RotatePages { .. }
        | Edit::DeletePages { .. }
        | Edit::MovePages { .. }
        | Edit::InsertBlankPage { .. }
        | Edit::DuplicatePages { .. }
        | Edit::InsertPagesFrom { .. } => Class::Pages,
        // An XFA form also keeps its values in a document-wide data packet.
        _ => match crate::scope_of(edit) {
            Scope::Comments => Class::Comments,
            Scope::Form if !xfa => Class::Fields,
            _ => Class::Document,
        },
    }
}

/// Objects whose state differs between `prev` and `next` (one a later state of the other).
fn changed_objects(prev: &Cos, next: &Cos) -> HashSet<u32> {
    let mut out = HashSet::new();
    for n in prev.modified_objects().into_iter().chain(next.modified_objects()) {
        let same = matches!((prev.try_get(n), next.try_get(n)), (Ok(a), Ok(b)) if Arc::ptr_eq(&a, &b));
        if !same {
            out.insert(n);
        }
    }
    out
}

/// The regions of `class` whose content differs between two states. `None` when a change can
/// only travel as `*` (a field added or removed, an id the room can't carry) and `sendable` is
/// asked for.
fn changed_regions(prev: &View, next: &View, class: Class, sendable: bool) -> Result<Option<Vec<Region>>, LiveError> {
    let touched = changed_objects(prev.cos, next.cos);
    let mut out: Vec<Region> = Vec::new();
    if matches!(class, Class::Comments | Class::All) {
        let (a, b) = (prev.comment_objects(), next.comment_objects());
        let ids: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).cloned().collect();
        for id in ids {
            let differs = match (a.get(&id), b.get(&id)) {
                (Some(x), Some(y)) => {
                    let hit = |(r, p): &(pdfcraft_cos::ObjRef, Option<pdfcraft_cos::ObjRef>)| {
                        touched.contains(&r.num) || p.is_some_and(|p| touched.contains(&p.num))
                    };
                    (x != y || hit(x) || hit(y)) && regions::comment_payload(prev, &id)? != regions::comment_payload(next, &id)?
                }
                _ => true,
            };
            if differs {
                out.push(Region::Annot(id));
            }
        }
    }
    if matches!(class, Class::Fields | Class::All) {
        let (a, b) = (regions::field_objects(prev.cos), regions::field_objects(next.cos));
        if sendable && (a.len() != b.len() || a.keys().any(|k| !b.contains_key(k))) {
            return Ok(None);
        }
        let mut names: Vec<&String> = a.keys().filter(|k| b.contains_key(*k)).collect();
        names.sort();
        for name in names {
            let (x, y) = (a.get(name), b.get(name));
            let hit = x != y || x.into_iter().chain(y).flatten().any(|r| touched.contains(&r.num));
            if hit && regions::field_payload(prev, name)? != regions::field_payload(next, name)? {
                out.push(Region::Field(name.clone()));
            }
        }
    }
    if matches!(class, Class::Pages | Class::All) && regions::page_state(prev) != regions::page_state(next) {
        out.push(Region::Pages);
    }
    if sendable && out.iter().any(|r| !r.is_sendable()) {
        return Ok(None);
    }
    Ok(Some(out))
}

fn too_large(e: LiveError) -> EditError {
    match e {
        LiveError::TooLarge => EditError::LiveTooLarge,
        other => EditError::Invalid(other.to_string()),
    }
}

/// The regions an edit from `prev` to `next` changes, or why it is refused: one of them is
/// held by someone else, or its content is too large to send.
pub(crate) fn regions_of_edit(
    state: &LiveState,
    page_ids: &mut PageIds,
    ids: &IdSource,
    (prev, next): (&Cos, &Cos),
    edit: &Edit,
    xfa: bool,
    file_len: usize,
) -> Result<Vec<Region>, EditError> {
    let class = class_of(edit, xfa);
    let found = if class == Class::Document {
        None
    } else {
        let prev_ids = page_ids.sync(prev, ids);
        let next_ids = page_ids.sync(next, ids);
        changed_regions(&View::new(prev, &prev_ids), &View::new(next, &next_ids), class, true).map_err(too_large)?
    };
    let regions = found.unwrap_or_else(|| vec![Region::Document]);
    if let Some(r) = regions.iter().find(|r| state.blocked.blocks(r)) {
        return Err(EditError::RegionLocked(r.to_string()));
    }
    if regions.contains(&Region::Document) && !regions::document_fits(file_len) {
        return Err(EditError::LiveTooLarge);
    }
    Ok(regions)
}

/// Scope of the view data a change to `regions` affects.
fn scope_of_regions(regions: &[Region]) -> Scope {
    if regions.iter().all(|r| matches!(r, Region::Annot(_))) {
        Scope::Comments
    } else if regions.iter().all(|r| matches!(r, Region::Field(_))) {
        Scope::Form
    } else {
        Scope::Full
    }
}

impl Session {
    /// Turn live mode on or off for document `id` (the office shell offers `erpHost` v3).
    pub fn set_live(&mut self, id: DocId, on: bool) -> Result<(), EditError> {
        let ids = self.ids.clone();
        let doc = self.doc_mut(id)?;
        if !on {
            doc.live = None;
            return Ok(());
        }
        if doc.live.is_some() {
            return Ok(());
        }
        let mut named = false;
        let pages = match doc.editor.as_mut() {
            Some(editor) => {
                // Not an edit of the user's (no undo step, not unsaved): every member names alike.
                named = stable_ids::name_comments_by_object(&mut editor.cos);
                for snapshot in editor.undo.iter_mut().chain(editor.redo.iter_mut()) {
                    stable_ids::name_comments_by_object(&mut snapshot.1);
                }
                doc.page_ids.sync(&editor.cos, &ids)
            }
            None => Vec::new(),
        };
        doc.live = Some(LiveState::new(&pages));
        if named {
            Self::refresh_scoped(doc, Scope::Comments)?;
        }
        Ok(())
    }

    pub fn is_live(&self, id: DocId) -> bool {
        self.get(id).is_some_and(|d| d.live.is_some())
    }

    /// What other people hold now (the shell's `showLocks`).
    pub fn set_live_blocked(&mut self, id: DocId, blocked: Blocked) {
        if let Ok(doc) = self.doc_mut(id)
            && let Some(state) = doc.live.as_mut()
        {
            state.blocked = blocked;
        }
    }

    /// The user's changes since the last call, in order.
    pub fn take_live_changes(&mut self, id: DocId) -> Vec<LocalChange> {
        match self.doc_mut(id) {
            Ok(doc) => doc.live.as_mut().map(|s| std::mem::take(&mut s.outbox)).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// How many of the user's steps the last undo or redo skipped because someone else changed
    /// their region since.
    pub fn take_live_skipped(&mut self, id: DocId) -> usize {
        match self.doc_mut(id) {
            Ok(doc) => doc.live.as_mut().map(|s| std::mem::take(&mut s.skipped)).unwrap_or_default(),
            Err(_) => 0,
        }
    }

    /// The current content of `region`, to send. Sending `pages` tells the room about the pages
    /// it carries.
    pub fn live_payload(&mut self, id: DocId, region: &Region) -> Result<String, LiveError> {
        let ids = self.ids.clone();
        let doc = self.doc_mut(id).map_err(|_| LiveError::NoDocument)?;
        let (Some(editor), Some(state)) = (doc.editor.as_ref(), doc.live.as_mut()) else { return Err(LiveError::NotLive) };
        let page_ids = doc.page_ids.sync(&editor.cos, &ids);
        let view = View::new(&editor.cos, &page_ids);
        let payload = guard(|| regions::payload(&view, region, &state.room_pages, &doc.bytes)).map_err(LiveError::Apply)??;
        if *region == Region::Pages {
            state.room_pages = page_ids.into_iter().collect();
        }
        Ok(payload)
    }

    /// Replace `region` of document `id` by someone else's `payload`. Every other region stays as
    /// it was; the user's undo steps on the regions it changed are skipped from now on. Applies
    /// in view and comment mode too: readers follow the edits live.
    pub fn apply_remote(&mut self, id: DocId, region: &str, payload: &str) -> Result<Applied, LiveError> {
        // No editor sends more; reading a larger one would only cost memory.
        if payload.len() > regions::MAX_PAYLOAD_BYTES {
            return Err(LiveError::TooLarge);
        }
        let region = Region::parse(region).ok_or_else(|| codec::malformed("the region"))?;
        let payload = regions::parse(payload)?;
        let ids = self.ids.clone();
        let doc = self.doc_mut(id).map_err(|_| LiveError::NoDocument)?;
        if doc.live.is_none() {
            return Err(LiveError::NotLive);
        }
        let Some(editor) = doc.editor.as_ref() else { return Err(LiveError::NotLive) };
        let prev = editor.cos.clone();
        let prev_ids = doc.page_ids.sync(&prev, &ids);
        let render_password = editor.keys.render.clone();
        let mut next = prev.clone();
        let applied = guard(|| -> Result<bool, LiveError> {
            match &region {
                Region::Annot(annot) => regions::apply_comment(&mut next, &prev_ids, annot, &payload),
                Region::Field(name) => regions::apply_field(&mut next, &prev_ids, name, &payload),
                Region::Pages => regions::apply_pages(&mut next, &prev_ids, &payload, PageSource::Payload).map(|()| true),
                Region::Document => {
                    let bytes = regions::document_bytes(&payload)?;
                    next = Cos::open_with_password(Arc::new(bytes), render_password.as_deref()).map_err(|e| LiveError::Apply(e.to_string()))?;
                    Ok(true)
                }
            }
        })
        .map_err(LiveError::Apply)??;
        if !applied {
            return Ok(Applied { region, pages: Some(Vec::new()), changed: false });
        }
        // The room checks only who holds the region id; what the payload wrote is checked here.
        if let Some(writable) = regions::writable_objects(&View::new(&prev, &prev_ids), &region) {
            let added = prev.next_object_number();
            let outside = |n: u32| {
                if n >= added || writable.objects.contains(&n) {
                    return false;
                }
                // A freed object reads as a fresh null each time: compare values, not pointers.
                let (Ok(a), Ok(b)) = (prev.try_get(n), next.try_get(n)) else { return true };
                match (a.as_dict(), b.as_dict()) {
                    (Some(x), Some(y)) if writable.annots_of.contains(&n) => {
                        let (mut x, mut y) = (x.clone(), y.clone());
                        x.remove(b"Annots");
                        y.remove(b"Annots");
                        x != y
                    }
                    _ => a != b,
                }
            };
            if changed_objects(&prev, &next).into_iter().any(outside) {
                return Err(codec::malformed("a change outside its region"));
            }
        }
        if region == Region::Document {
            doc.page_ids.clear();
        }
        let next_ids = doc.page_ids.sync(&next, &ids);
        let touched = if region == Region::Document {
            Vec::new()
        } else {
            guard(|| changed_regions(&View::new(&prev, &prev_ids), &View::new(&next, &next_ids), Class::All, false))
                .map_err(LiveError::Apply)??
                .unwrap_or_default()
        };
        let pages = pages_of(&region, &prev, &next, &next_ids, &touched);
        let scope = scope_of_regions(std::slice::from_ref(&region));
        if let Some(editor) = doc.editor.as_mut() {
            editor.cos = next;
        }
        let refreshed = guard(|| if scope == Scope::Full { Self::refresh(doc) } else { Self::refresh_scoped(doc, scope) })
            .unwrap_or_else(|m| Err(EditError::Reopen(m)));
        if let Err(e) = refreshed {
            if let Some(editor) = doc.editor.as_mut() {
                editor.cos = prev;
            }
            doc.page_ids.clear();
            if !matches!(guard(|| Self::refresh(doc)), Ok(Ok(()))) {
                Self::show_released(doc);
            }
            return Err(LiveError::Apply(e.to_string()));
        }
        if let Some(state) = doc.live.as_mut() {
            state.epoch += 1;
            let epoch = state.epoch;
            if region == Region::Document {
                state.everything = epoch;
                state.room_pages = next_ids.iter().copied().collect();
            }
            if region == Region::Pages {
                state.room_pages.extend(next_ids.iter().copied());
            }
            for r in touched.into_iter().chain(std::iter::once(region.clone())) {
                state.touched.insert(r, epoch);
            }
        }
        doc.dirty = true;
        doc.generation += 1;
        Ok(Applied { region, pages, changed: true })
    }

    /// Take back a change the room refused (its lock was denied): its regions go back to how they
    /// were before it, and the step leaves the history. Other people's changes since stay.
    pub fn revert_live_change(&mut self, id: DocId, change: &LocalChange) -> Result<(), EditError> {
        let ids = self.ids.clone();
        let doc = self.doc_mut(id)?;
        let Some(state) = doc.live.as_mut() else { return Ok(()) };
        state.outbox.retain(|c| !(c.step == change.step && c.kind == change.kind));
        let step = state.step(change.step);
        let epoch = state.epoch;
        // What someone else changed since is theirs now: only the rest goes back.
        let fresh: Vec<Region> =
            step.regions.iter().filter(|r| !state.is_stale(&Step { regions: vec![(*r).clone()], epoch: step.epoch })).cloned().collect();
        let editor = doc.editor.as_mut().ok_or(EditError::NothingToUndo)?;
        // An edit or a redo sits on the undo stack (its snapshot is the state before it); an undo
        // on the redo stack (its snapshot is the state after the step).
        let on_undo = change.kind != ChangeKind::Undo;
        let stack = if on_undo { &mut editor.undo } else { &mut editor.redo };
        let Some(at) = stack.iter().rposition(|s| s.4.transaction_id == change.step) else { return Ok(()) };
        let is_top = at + 1 == stack.len();
        let swapped_back = is_top && step.epoch == epoch;
        let (before, before_keys) = (editor.cos.clone(), editor.keys.clone());
        if swapped_back {
            if let Some(s) = stack.get(at) {
                editor.cos = s.1.clone();
                editor.keys = s.2.clone();
            }
        } else if let Some(s) = stack.get(at)
            && !fresh.is_empty()
        {
            let mut next = editor.cos.clone();
            copy_regions(&mut next, &s.1, &fresh, &mut doc.page_ids, &ids).map_err(too_large)?;
            editor.cos = next;
        }
        let snapshot = stack.remove(at);
        // An undone undo goes back to the undo stack: the step is still the user's.
        if !on_undo {
            editor.undo.push((snapshot.0, before, before_keys, snapshot.3, snapshot.4));
        }
        if let Some(state) = doc.live.as_mut() {
            if !swapped_back {
                state.epoch += 1;
            }
            if on_undo {
                state.steps.remove(&change.step);
            } else if let Some(s) = state.steps.get_mut(&change.step) {
                s.epoch = state.epoch;
            }
        }
        Self::adopt_keys(doc);
        Self::refresh_scoped(doc, scope_of_regions(&step.regions)).inspect_err(|_| Self::show_released(doc))?;
        doc.generation += 1;
        Ok(())
    }

    /// Undo or redo in live mode: skip the user's steps someone else changed since, swap whole
    /// states while nobody else changed anything since the step, and otherwise put back only the
    /// step's regions.
    pub(crate) fn live_history(&mut self, id: DocId, undo: bool) -> Result<Label, EditError> {
        let ids = self.ids.clone();
        let doc = self.doc_mut(id)?;
        doc.check_not_view_only()?;
        doc.edit_notices.clear();
        let Some(state) = doc.live.as_mut() else { return Err(EditError::NothingToUndo) };
        let nothing = || if undo { EditError::NothingToUndo } else { EditError::NothingToRedo };
        let editor = doc.editor.as_mut().ok_or_else(nothing)?;
        let (from, to) = if undo { (&mut editor.undo, &mut editor.redo) } else { (&mut editor.redo, &mut editor.undo) };
        let step = loop {
            let top = from.last().ok_or_else(nothing)?;
            let step = state.step(top.4.transaction_id);
            if state.is_stale(&step) {
                let tid = top.4.transaction_id;
                from.pop();
                state.steps.remove(&tid);
                state.skipped += 1;
                continue;
            }
            if let Some(r) = step.regions.iter().find(|r| state.blocked.blocks(r)) {
                return Err(EditError::RegionLocked(r.to_string()));
            }
            break step;
        };
        let (label, snapshot, keys, scope, entry) = from.pop().ok_or_else(nothing)?;
        let current_keys = editor.keys.clone();
        let (next, next_keys, scope) = if step.epoch == state.epoch {
            (snapshot, keys, scope)
        } else {
            let mut next = editor.cos.clone();
            if let Err(e) = copy_regions(&mut next, &snapshot, &step.regions, &mut doc.page_ids, &ids) {
                from.push((label, snapshot, keys, scope, entry));
                return Err(too_large(e));
            }
            (next, current_keys.clone(), scope_of_regions(&step.regions))
        };
        let current = std::mem::replace(&mut editor.cos, next);
        editor.keys = next_keys;
        to.push((label.clone(), current, current_keys, scope, entry));
        editor.trim_history(crate::memory::HISTORY_BYTES);
        let kind = if undo { ChangeKind::Undo } else { ChangeKind::Redo };
        state.record(entry.transaction_id, kind, step.regions.clone());
        Self::adopt_keys(doc);
        Self::refresh_scoped(doc, scope).inspect_err(|_| Self::show_released(doc))?;
        doc.dirty = true;
        doc.generation += 1;
        Ok(label)
    }
}

/// Put `regions` of `source` (an earlier or later state of the same document) into `target`.
fn copy_regions(target: &mut Cos, source: &Cos, regions: &[Region], page_ids: &mut PageIds, ids: &IdSource) -> Result<(), LiveError> {
    let source_ids = page_ids.sync(source, ids);
    let source_view = View::new(source, &source_ids);
    let mut known = HashSet::new();
    known.extend(source_ids.iter().copied());
    for region in regions {
        let target_ids = page_ids.sync(target, ids);
        let payload = regions::parse(&regions::payload(&source_view, region, &known, &[])?)?;
        match region {
            Region::Annot(id) => {
                regions::apply_comment(target, &target_ids, id, &payload)?;
            }
            Region::Field(name) => {
                regions::apply_field(target, &target_ids, name, &payload)?;
            }
            Region::Pages => regions::apply_pages(target, &target_ids, &payload, PageSource::State(source, &source_ids))?,
            // Only reached with no remote change since the step, which swaps whole states.
            Region::Document => return Err(LiveError::Apply("the whole document can't be put back over other people's changes".into())),
        }
    }
    Ok(())
}

/// The pages whose look a remote change to `region` changed (`None`: all of them).
fn pages_of(region: &Region, prev: &Cos, next: &Cos, next_ids: &[PageId], touched: &[Region]) -> Option<Vec<usize>> {
    if touched.iter().chain(std::iter::once(region)).any(|r| matches!(r, Region::Pages | Region::Document)) {
        return None;
    }
    let mut pages = Vec::new();
    for state in [prev, next] {
        let comments = stable_ids::comment_ids(state);
        let fields = pdfcraft_forms::fields(state);
        for r in touched.iter().chain(std::iter::once(region)) {
            match r {
                Region::Annot(id) => pages.extend(comments.iter().filter(|(a, _)| &a.id == id).map(|(a, _)| a.page)),
                Region::Field(name) => pages.extend(fields.iter().filter(|f| &f.name == name).flat_map(|f| f.widgets.iter().filter_map(|w| w.page))),
                _ => {}
            }
        }
    }
    pages.retain(|p| *p < next_ids.len());
    pages.sort_unstable();
    pages.dedup();
    Some(pages)
}
