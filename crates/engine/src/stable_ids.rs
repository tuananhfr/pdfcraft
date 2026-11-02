//! Ids that stay put while several people edit one document (ERPCons co-editing): the `/NM` of
//! comments, a per-session id for each page, and form fields by their full name.
//!
//! Every id the session creates carries its [`ClientPrefix`], so two sessions creating at once
//! never pick the same one. Ids already in the file are never rewritten.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pdfcraft_cos::{Document as Cos, ObjRef, Object, PdfString, XrefEntry};

use crate::{DocId, Edit, Session};

/// 24 random bits naming one editing session (six lowercase hex digits on the wire). The office
/// shell hands one in with `erpHost.setClientId`; otherwise the session picks its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClientPrefix(u32);

impl ClientPrefix {
    const MASK: u32 = 0x00ff_ffff;

    /// Exactly six lowercase hex digits; anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return None;
        }
        u32::from_str_radix(s, 16).ok().map(Self)
    }

    /// A fresh prefix from the system's randomness (Web Crypto in the browser).
    pub fn random() -> Self {
        let mut bytes = [0u8; 4];
        if getrandom::fill(&mut bytes).is_err() {
            // No randomness source: a hash keyed by the OS (fixed on some targets) plus the clock
            // still beats a constant.
            use std::hash::{BuildHasher, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(clock_nanos());
            let [a, b, c, d, ..] = h.finish().to_le_bytes();
            bytes = [a, b, c, d];
        }
        Self(u32::from_le_bytes(bytes) & Self::MASK)
    }

    pub fn bits(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ClientPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06x}", self.0)
    }
}

fn clock_nanos() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    return std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    #[cfg(target_arch = "wasm32")]
    return (js_sys::Date::now().max(0.0) * 1e6) as u64;
}

/// The session's prefix and its counter of created ids, shared by every edit of the session.
#[derive(Clone, Debug)]
pub(crate) struct IdSource {
    prefix: ClientPrefix,
    seq: Arc<AtomicU64>,
}

impl Default for IdSource {
    fn default() -> Self {
        Self { prefix: ClientPrefix::random(), seq: Arc::default() }
    }
}

impl IdSource {
    pub(crate) fn prefix(&self) -> ClientPrefix {
        self.prefix
    }

    /// Never the same number twice in a session, whatever the prefix.
    pub(crate) fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    /// A comment's `/NM`: a UUID (version 4 layout) whose first 48 bits are the prefix and the
    /// session counter, so it is unique by construction; `a` and `b` fill the rest.
    pub(crate) fn annotation_name(&self, a: u64, b: u64) -> String {
        let seq = self.next_seq() & 0xff_ffff;
        format!(
            "{}{:02x}-{:04x}-4{:03x}-{:04x}-{:012x}",
            self.prefix,
            seq >> 16,
            seq & 0xffff,
            a & 0xfff,
            0x8000 | (b >> 48) & 0x3fff,
            b & 0xffff_ffff_ffff
        )
    }
}

/// A page's id for this session. Pages of the file as opened (or last saved) are named by their
/// object, so every session that opened the same version agrees; pages created here carry the
/// prefix. Kept through insert, delete, reorder, undo and redo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageId {
    File { num: u32, generation: u16 },
    New { prefix: ClientPrefix, seq: u64 },
}

impl PageId {
    /// The [`Display`](fmt::Display) form back: `f<num>.<generation>` or `n<prefix>.<seq hex>`.
    /// Only that form: ids are lock keys, so `f012.0` must not name the page `f12.0` names.
    pub fn parse(s: &str) -> Option<Self> {
        Self::parse_any(s).filter(|id| id.to_string() == s)
    }

    fn parse_any(s: &str) -> Option<Self> {
        let (head, tail) = s.get(1..)?.split_once('.')?;
        match s.as_bytes().first()? {
            b'f' if is_decimal(head) && is_decimal(tail) => Some(Self::File { num: head.parse().ok()?, generation: tail.parse().ok()? }),
            b'n' if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => {
                Some(Self::New { prefix: ClientPrefix::parse(head)?, seq: u64::from_str_radix(tail, 16).ok()? })
            }
            _ => None,
        }
    }
}

fn is_decimal(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

impl fmt::Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File { num, generation } => write!(f, "f{num}.{generation}"),
            Self::New { prefix, seq } => write!(f, "n{prefix}.{seq:x}"),
        }
    }
}

/// Page ids by page object. Entries outlive their pages so undoing a delete brings the same id back.
#[derive(Default)]
pub(crate) struct PageIds {
    by_ref: HashMap<ObjRef, PageId>,
}

impl PageIds {
    /// The id of each page of `cos`, in order, naming pages seen for the first time: by the id a
    /// co-editing session stored in the page ([`PAGE_ID_KEY`]) unless another page has it.
    pub(crate) fn sync(&mut self, cos: &Cos, ids: &IdSource) -> Vec<PageId> {
        let refs = pdfcraft_annot::page_refs(cos).unwrap_or_default();
        let mut used: HashSet<PageId> = refs.iter().filter_map(|r| self.by_ref.get(r).copied()).collect();
        refs.iter()
            .map(|r| {
                if let Some(id) = self.by_ref.get(r) {
                    return *id;
                }
                let id = match stored_page_id(cos, *r).filter(|id| !used.contains(id)) {
                    Some(id) => id,
                    None if in_file(cos, *r) => PageId::File { num: r.num, generation: r.generation },
                    None => PageId::New { prefix: ids.prefix(), seq: ids.next_seq() },
                };
                used.insert(id);
                self.by_ref.insert(*r, id);
                id
            })
            .collect()
    }

    /// Forget every page, as after the working file was replaced by another client's.
    pub(crate) fn clear(&mut self) {
        self.by_ref.clear();
    }

    /// Objects numbered from `first_free` on were never in the state an edit started from: a later
    /// edit may number new objects the same way, so their ids (from an undone, now discarded
    /// branch) must not pass to them.
    pub(crate) fn forget_from(&mut self, first_free: u32) {
        self.by_ref.retain(|r, _| r.num < first_free);
    }

    /// After a save that may have renumbered the objects: the pages keep their order, so they
    /// keep their ids in that order.
    fn carry_over(&mut self, before: &[PageId], cos: &Cos) {
        let refs = pdfcraft_annot::page_refs(cos).unwrap_or_default();
        if refs.len() != before.len() {
            return;
        }
        // Stale numbers would name other objects after a renumbering.
        let renumbered = refs.iter().zip(before).any(|(r, id)| self.by_ref.get(r) != Some(id));
        if renumbered {
            self.by_ref.clear();
        }
        self.by_ref.extend(refs.into_iter().zip(before.iter().copied()));
    }
}

/// A page's id as a co-editing session stored it, so the room's ids survive the leader's save:
/// a client that opens that version reads them instead of naming the pages by object.
pub(crate) const PAGE_ID_KEY: &[u8] = b"ERPConsPageId";

fn stored_page_id(cos: &Cos, page: ObjRef) -> Option<PageId> {
    let obj = cos.get(page);
    let value = obj.as_dict()?.get(PAGE_ID_KEY)?;
    PageId::parse(&cos.resolve(value).as_string()?.to_text())
}

/// Store `id` in a page (co-editing only).
pub(crate) fn store_page_id(cos: &mut Cos, page: ObjRef, id: PageId) {
    let _ = cos.update_dict(page, |d| d.set(PAGE_ID_KEY.to_vec(), PdfString::text(&id.to_string())));
}

/// Name with a fresh id, stored in the page, each page `next` created (numbered from
/// `first_free`): a copied page must not keep the id stored in its original.
pub(crate) fn stamp_new_pages(next: &mut Cos, first_free: u32, ids: &IdSource) {
    for page in pdfcraft_annot::page_refs(next).unwrap_or_default().into_iter().filter(|r| r.num >= first_free) {
        store_page_id(next, page, PageId::New { prefix: ids.prefix(), seq: ids.next_seq() });
    }
}

/// Whether `r` is an object of the file as opened or last saved (not created since).
fn in_file(cos: &Cos, r: ObjRef) -> bool {
    match cos.xref_entry(r.num) {
        Some(XrefEntry::InFile { generation, .. }) => generation == r.generation,
        Some(XrefEntry::InStream { .. }) => r.generation == 0,
        _ => false,
    }
}

/// A comment's id: its `/NM`, or for a comment of the file without one (or sharing it with
/// another comment) its object, which every session that opened the same version agrees on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AnnotId {
    Name(String),
    Object { num: u32, generation: u16 },
}

impl AnnotId {
    /// The [`Display`](fmt::Display) form back: `nm:<name>` or `obj:<num>.<generation>`. Only
    /// that form: ids are lock keys, so `obj:012.0` must not name the comment `obj:12.0` names.
    pub fn parse(s: &str) -> Option<Self> {
        Self::parse_any(s).filter(|id| id.to_string() == s)
    }

    fn parse_any(s: &str) -> Option<Self> {
        if let Some(name) = s.strip_prefix("nm:") {
            return (!name.is_empty()).then(|| Self::Name(name.to_owned()));
        }
        let (num, generation) = s.strip_prefix("obj:")?.split_once('.')?;
        if !is_decimal(num) || !is_decimal(generation) {
            return None;
        }
        Some(Self::Object { num: num.parse().ok()?, generation: generation.parse().ok()? })
    }
}

impl fmt::Display for AnnotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(n) => write!(f, "nm:{n}"),
            Self::Object { num, generation } => write!(f, "obj:{num}.{generation}"),
        }
    }
}

/// A comment and its id: 0-based page, index in the page's `/Annots`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnotRef {
    pub page: usize,
    pub index: usize,
    pub id: AnnotId,
}

fn name_of(cos: &Cos, d: &pdfcraft_cos::Dict) -> Option<String> {
    d.get(b"NM").and_then(|n| cos.resolve(n).as_string().map(PdfString::to_text)).filter(|n| !n.is_empty())
}

pub(crate) fn is_comment(d: &pdfcraft_cos::Dict) -> bool {
    d.name(b"Subtype").is_some_and(|s| pdfcraft_annot::is_comment_subtype(&String::from_utf8_lossy(s)))
}

/// Every comment of `cos` (by reference; inline dictionaries have no id) with its page and index.
fn comments(cos: &Cos) -> Vec<(usize, usize, ObjRef, Option<String>)> {
    let mut out = Vec::new();
    for (page, p) in pdfcraft_annot::page_refs(cos).unwrap_or_default().into_iter().enumerate() {
        let annots = cos.get(p).as_dict().and_then(|d| d.get(b"Annots").cloned()).map(|a| cos.resolve(&a));
        for (index, entry) in annots.as_deref().and_then(Object::as_array).into_iter().flatten().enumerate() {
            let Some(r) = entry.as_ref() else { continue };
            let obj = cos.get(r);
            let Some(d) = obj.as_dict().filter(|d| is_comment(d)) else { continue };
            out.push((page, index, r, name_of(cos, d)));
        }
    }
    out
}

/// Every comment of `cos` with its id and object, in page order.
pub(crate) fn comment_ids(cos: &Cos) -> Vec<(AnnotRef, ObjRef)> {
    let all = comments(cos);
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for name in all.iter().filter_map(|c| c.3.as_deref()) {
        *uses.entry(name).or_default() += 1;
    }
    all.iter()
        .map(|(page, index, r, name)| {
            let id = match name {
                Some(n) if uses.get(n.as_str()) == Some(&1) => AnnotId::Name(n.clone()),
                _ => AnnotId::Object { num: r.num, generation: r.generation },
            };
            (AnnotRef { page: *page, index: *index, id }, *r)
        })
        .collect()
}

/// Co-editing: give each comment without a `/NM`, or sharing one with an earlier comment, the
/// name `obj-<num>-<gen>` of its object, which every session that opened the same version
/// derives alike. A comment then keeps its id when a page is deleted and brought back, which
/// copies it to another object. `true` when something was named.
pub(crate) fn name_comments_by_object(cos: &mut Cos) -> bool {
    let all = comments(cos);
    let mut taken: HashSet<String> = all.iter().filter_map(|c| c.3.clone()).collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut named = false;
    for (_, _, r, name) in all {
        if let Some(name) = name
            && seen.insert(name)
        {
            continue;
        }
        let fresh = format!("obj-{}-{}", r.num, r.generation);
        if taken.insert(fresh.clone()) {
            named |= cos.update_dict(r, |d| d.set(b"NM".to_vec(), PdfString::text(&fresh))).is_ok();
        }
    }
    named
}

/// Edits that copy existing comments (pages duplicated or inserted, comment data imported).
pub(crate) fn copies_comments(edit: &Edit) -> bool {
    match edit {
        Edit::DuplicatePages { .. } | Edit::InsertPagesFrom { .. } | Edit::ImportData { .. } => true,
        Edit::Batch { edits, .. } => edits.iter().any(copies_comments),
        _ => false,
    }
}

/// Give each comment `next` created (numbered from `first_free`) without a `/NM`, or with one
/// another comment already has, a fresh `/NM`. Comments that were there keep theirs.
pub(crate) fn name_new_comments(next: &mut Cos, first_free: u32, mut fresh: impl FnMut() -> String) {
    let all = comments(next);
    let (created, kept): (Vec<_>, Vec<_>) = all.into_iter().partition(|c| c.2.num >= first_free);
    if created.is_empty() {
        return;
    }
    let mut taken: HashSet<String> = kept.into_iter().filter_map(|c| c.3).collect();
    for (_, _, r, name) in created {
        if let Some(name) = name.filter(|n| !taken.contains(n)) {
            taken.insert(name);
            continue;
        }
        let name = fresh();
        let _ = next.update_dict(r, |d| d.set(b"NM".to_vec(), PdfString::text(&name)));
        taken.insert(name);
    }
}

impl Session {
    /// Mix `prefix` into every id created from now on (`erpHost.setClientId`). Ids created
    /// earlier keep theirs.
    pub fn set_client_prefix(&mut self, prefix: ClientPrefix) {
        self.ids.prefix = prefix;
    }

    pub fn client_prefix(&self) -> ClientPrefix {
        self.ids.prefix
    }

    /// The id of each page of document `id`, in order (empty when it can't be edited).
    pub fn page_ids(&mut self, id: DocId) -> Vec<PageId> {
        let ids = &self.ids;
        let Some(doc) = self.docs.iter_mut().find(|d| d.id == id) else { return Vec::new() };
        let Some(editor) = doc.editor.as_ref() else { return Vec::new() };
        doc.page_ids.sync(&editor.cos, ids)
    }

    /// Where the page named `page` is now, if it still exists.
    pub fn page_index(&mut self, id: DocId, page: PageId) -> Option<usize> {
        self.page_ids(id).iter().position(|p| *p == page)
    }

    /// After a save of document `id` reopened its working file; `before` is [`Self::page_ids`]
    /// from just before it, which also named the pages created since the last save.
    pub(crate) fn page_ids_after_save(&mut self, id: DocId, before: &[PageId]) {
        let Some(doc) = self.docs.iter_mut().find(|d| d.id == id) else { return };
        let Some(editor) = doc.editor.as_ref() else { return };
        doc.page_ids.carry_over(before, &editor.cos);
    }

    /// Every comment of document `id` with its id, in page order.
    pub fn annotation_ids(&self, id: DocId) -> Vec<AnnotRef> {
        let Some(cos) = self.get(id).and_then(|d| d.editor.as_ref()).map(|e| &e.cos) else { return Vec::new() };
        comment_ids(cos).into_iter().map(|(a, _)| a).collect()
    }

    /// Where the comment named `annot` is now: (page, index in its `/Annots`).
    pub fn find_annotation(&self, id: DocId, annot: &AnnotId) -> Option<(usize, usize)> {
        self.annotation_ids(id).into_iter().find(|a| &a.id == annot).map(|a| (a.page, a.index))
    }
}
