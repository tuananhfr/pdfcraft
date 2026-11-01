//! What each region holds, as a payload, and how a payload replaces it.
//!
//! Payloads are JSON objects with `"v":1`:
//! - comment: `{"v":1,"page":<page id>,"at":<index in /Annots>,"g":[<comment>,<pop-up and what they
//!   reach>…]}`, or `{"v":1,"del":1}` once it is gone;
//! - field: `{"v":1,"f":{"V":…,"Ff":…,"w":[{"AS","AP","F","MK"} per widget]},"na":<NeedAppearances>,"g":[…]}`
//!   (absent keys are removed);
//! - `pages`: `{"v":1,"order":[<page id>…],"rot":[<degrees>…],"new":{"ids":[…],"pdf":<base64 PDF of
//!   those pages>}}`, `new` holding the pages the room may not have yet;
//! - `*`: `{"v":1,"pdf":<base64 of the whole working file>}`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use pdfcraft_cos::{Document as Cos, ObjRef, Object};
use serde_json::{Map, Value, json};

use super::LiveError;
use super::codec::{Anchors, Encoder, Targets, decode_objects, decode_value, malformed};
use super::region::Region;
use crate::stable_ids::{self, AnnotId, AnnotRef, PageId};

pub(crate) const VERSION: u64 = 1;
/// Largest payload a region sends (the room takes 1 MiB per message, envelope included).
pub const MAX_PAYLOAD_BYTES: usize = 1_000_000;
/// Widget keys a field's region carries.
const WIDGET_KEYS: [&[u8]; 4] = [b"AS", b"AP", b"F", b"MK"];

/// One state of a document as regions see it: its pages with their ids, and its comments.
pub(crate) struct View<'a> {
    pub cos: &'a Cos,
    pub pages: Vec<(ObjRef, PageId)>,
    pub comments: Vec<(AnnotRef, ObjRef)>,
}

impl<'a> View<'a> {
    /// `ids` are the page ids of `cos`, in order (`PageIds::sync`).
    pub fn new(cos: &'a Cos, ids: &[PageId]) -> Self {
        let refs = pdfcraft_annot::page_refs(cos).unwrap_or_default();
        let pages = refs.into_iter().zip(ids.iter().copied()).collect();
        Self { cos, pages, comments: stable_ids::comment_ids(cos) }
    }

    fn comment(&self, id: &AnnotId) -> Option<&(AnnotRef, ObjRef)> {
        self.comments.iter().find(|(a, _)| &a.id == id)
    }

    fn anchors(&self) -> Anchors {
        let mut annotations = HashSet::new();
        for (page, _) in &self.pages {
            annotations.extend(annots(self.cos, *page).iter().filter_map(Object::as_ref));
        }
        Anchors {
            pages: self.pages.iter().copied().collect(),
            comments: self.comments.iter().map(|(a, r)| (*r, a.id.clone())).collect(),
            annotations,
        }
    }

    fn targets(&self) -> (HashMap<PageId, ObjRef>, HashMap<AnnotId, ObjRef>) {
        (self.pages.iter().map(|(r, id)| (*id, *r)).collect(), self.comments.iter().map(|(a, r)| (a.id.clone(), *r)).collect())
    }

    /// The comment ids with their objects (the comment and its pop-up), for change detection.
    pub fn comment_objects(&self) -> HashMap<AnnotId, (ObjRef, Option<ObjRef>)> {
        self.comments.iter().map(|(a, r)| (a.id.clone(), (*r, popup_of(self.cos, *r)))).collect()
    }
}

fn annots(cos: &Cos, page: ObjRef) -> Vec<Object> {
    let obj = cos.get(page);
    let Some(a) = obj.as_dict().and_then(|d| d.get(b"Annots")) else { return Vec::new() };
    cos.resolve(a).as_array().cloned().unwrap_or_default()
}

fn set_annots(cos: &mut Cos, page: ObjRef, list: Vec<Object>) -> Result<(), LiveError> {
    let existing = cos.get(page).as_dict().and_then(|d| d.reference(b"Annots"));
    match existing {
        Some(r) if cos.get(r).as_array().is_some() => cos.set(r, Object::Array(list)),
        _ => cos
            .update_dict(page, |d| {
                if list.is_empty() {
                    d.remove(b"Annots");
                } else {
                    d.set(b"Annots".to_vec(), Object::Array(list));
                }
            })
            .map_err(|e| LiveError::Apply(e.to_string()))?,
    }
    Ok(())
}

fn popup_of(cos: &Cos, annot: ObjRef) -> Option<ObjRef> {
    cos.get(annot).as_dict().and_then(|d| d.reference(b"Popup"))
}

fn rotation(cos: &Cos, page: ObjRef) -> i64 {
    let mut node = Some(page);
    // The page tree is shallow; the bound only stops a cycle.
    for _ in 0..64 {
        let Some(r) = node else { break };
        let obj = cos.get(r);
        let Some(d) = obj.as_dict() else { break };
        if let Some(rot) = d.get(b"Rotate").and_then(|o| cos.resolve(o).as_int()) {
            return rot.rem_euclid(360);
        }
        node = d.reference(b"Parent");
    }
    0
}

fn header() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("v".into(), Value::from(VERSION));
    m
}

/// The payload's object, refusing one from a newer version.
pub(crate) fn parse(payload: &str) -> Result<Map<String, Value>, LiveError> {
    let Value::Object(m) = serde_json::from_str::<Value>(payload).map_err(|_| malformed("JSON"))? else {
        return Err(malformed("a JSON object"));
    };
    match m.get("v").and_then(Value::as_u64) {
        Some(VERSION) => Ok(m),
        Some(_) => Err(LiveError::Version),
        None => Err(malformed("the version")),
    }
}

fn finish(m: Map<String, Value>) -> Result<String, LiveError> {
    let s = Value::Object(m).to_string();
    if s.len() > MAX_PAYLOAD_BYTES {
        return Err(LiveError::TooLarge);
    }
    Ok(s)
}

// ── comments ────────────────────────────────────────────────────────────────────────────────

pub(crate) fn comment_payload(view: &View, id: &AnnotId) -> Result<String, LiveError> {
    let mut m = header();
    let Some((at, r)) = view.comment(id) else {
        m.insert("del".into(), Value::from(1));
        return finish(m);
    };
    let page = view.pages.get(at.page).map(|p| p.1).ok_or_else(|| malformed("the comment's page"))?;
    let anchors = view.anchors();
    let mut enc = Encoder::new(view.cos, &anchors);
    enc.own(*r)?;
    if let Some(p) = popup_of(view.cos, *r) {
        enc.own(p)?;
    }
    m.insert("page".into(), Value::String(page.to_string()));
    m.insert("at".into(), Value::from(at.index));
    m.insert("g".into(), Value::Array(enc.finish()?));
    finish(m)
}

/// Replace comment `id` by `payload`, creating it (with its object number, for a comment of the
/// file) or deleting it. `false` when its page is not here.
pub(crate) fn apply_comment(cos: &mut Cos, ids: &[PageId], id: &AnnotId, payload: &Map<String, Value>) -> Result<bool, LiveError> {
    let view = View::new(cos, ids);
    let existing = view.comment(id).map(|(a, r)| (view.pages.get(a.page).map(|p| p.0), *r));
    let (pages, comments) = view.targets();
    if payload.contains_key("del") {
        if let Some((Some(page), r)) = existing {
            let popup = popup_of(cos, r);
            let list: Vec<Object> =
                annots(cos, page).into_iter().filter(|o| o.as_ref() != Some(r) && (popup.is_none() || o.as_ref() != popup)).collect();
            set_annots(cos, page, list)?;
        }
        return Ok(true);
    }
    let page_id = payload.get("page").and_then(Value::as_str).and_then(PageId::parse).ok_or_else(|| malformed("the page"))?;
    let Some(page) = pages.get(&page_id).copied() else { return Ok(false) };
    let g = payload.get("g").and_then(Value::as_array).ok_or_else(|| malformed("the objects"))?;
    let root = match (existing, id) {
        (Some((_, r)), _) => Some(r),
        // A comment of the file comes back under its own number, so its id stays the same.
        (None, AnnotId::Object { num, generation }) => {
            let r = ObjRef::new(*num, *generation);
            if !reusable_comment_object(&view, r) {
                return Err(malformed("the comment's object"));
            }
            Some(r)
        }
        (None, AnnotId::Name(_)) => None,
    };
    let old_popup = root.and_then(|r| popup_of(cos, r));
    let mut preset = HashMap::new();
    if let Some(r) = root {
        preset.insert(0, r);
    }
    // The pop-up keeps its object too.
    if let (Some(p), Some(Value::Object(d))) = (old_popup, g.first().and_then(|o| o.get("d")).and_then(|d| d.get("Popup")))
        && let Some(i) = d.get("r").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok())
        && i != 0
    {
        preset.insert(i, p);
    }
    let refs = decode_objects(cos, g, &preset, &Targets { pages: &pages, comments: &comments })?;
    let root = *refs.first().ok_or_else(|| malformed("the comment"))?;
    let popup = popup_of(cos, root);
    if let Some((Some(old_page), _)) = existing
        && old_page != page
    {
        let list =
            annots(cos, old_page).into_iter().filter(|o| o.as_ref() != Some(root) && (old_popup.is_none() || o.as_ref() != old_popup)).collect();
        set_annots(cos, old_page, list)?;
    }
    let mut list: Vec<Object> =
        annots(cos, page).into_iter().filter(|o| old_popup.is_none() || old_popup == popup || o.as_ref() != old_popup).collect();
    if !list.iter().any(|o| o.as_ref() == Some(root)) {
        let at = payload.get("at").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).unwrap_or(usize::MAX).min(list.len());
        list.insert(at, Object::Ref(root));
    }
    if let Some(p) = popup
        && !list.iter().any(|o| o.as_ref() == Some(p))
    {
        let after = list.iter().position(|o| o.as_ref() == Some(root)).map_or(list.len(), |i| i + 1);
        list.insert(after, Object::Ref(p));
    }
    set_annots(cos, page, list)?;
    Ok(true)
}

/// Whether a comment named by object number may be written at `r` while no page lists it: a free
/// number below the next one, or a comment's own object that no page lists any more. Anything
/// else (the catalogue, a page, a widget, another comment) belongs to another region.
fn reusable_comment_object(view: &View, r: ObjRef) -> bool {
    let cos = view.cos;
    if r.num == 0 || r.num >= cos.next_object_number() {
        return false;
    }
    match &*cos.get(r) {
        Object::Null => true,
        Object::Dict(d) => {
            // `is_comment` assumes an annotation; a font or an image has a `/Subtype` too.
            cos.generation(r.num) == r.generation
                && d.name(b"Type").is_none_or(|t| t == b"Annot")
                && d.contains(b"Rect")
                && stable_ids::is_comment(d)
                && !view.pages.iter().any(|(page, _)| annots(cos, *page).iter().any(|o| o.as_ref() == Some(r)))
        }
        _ => false,
    }
}

/// The existing objects a payload for `region` may change; `None` for regions that may change
/// any (`pages`, `*`). Objects it adds are always allowed.
#[derive(Default)]
pub(crate) struct Writable {
    pub objects: HashSet<u32>,
    /// Pages whose `/Annots` alone may change: a comment moves between pages with its pop-up.
    pub annots_of: HashSet<u32>,
}

pub(crate) fn writable_objects(view: &View, region: &Region) -> Option<Writable> {
    let cos = view.cos;
    let mut out = Writable::default();
    match region {
        Region::Annot(id) => {
            for (page, _) in &view.pages {
                out.annots_of.insert(page.num);
                if let Some(r) = cos.get(*page).as_dict().and_then(|d| d.reference(b"Annots")) {
                    out.objects.insert(r.num);
                }
            }
            let own = match (view.comment(id), id) {
                (Some((_, r)), _) => Some(*r),
                (None, AnnotId::Object { num, generation }) => Some(ObjRef::new(*num, *generation)).filter(|r| reusable_comment_object(view, *r)),
                (None, AnnotId::Name(_)) => None,
            };
            if let Some(r) = own {
                out.objects.insert(r.num);
                out.objects.extend(popup_of(cos, r).map(|p| p.num));
            }
        }
        Region::Field(name) => {
            out.objects.extend(field_objects(cos).get(name).into_iter().flatten().map(|r| r.num));
            // NeedAppearances, in the catalogue's /AcroForm.
            if let Some(root) = cos.root() {
                out.objects.insert(root.num);
                out.objects.extend(cos.get(root).as_dict().and_then(|d| d.reference(b"AcroForm")).map(|r| r.num));
            }
        }
        Region::Pages | Region::Document => return None,
    }
    Some(out)
}

// ── form fields ─────────────────────────────────────────────────────────────────────────────

/// Every field by full name, with its objects (the field and its widgets).
pub(crate) fn field_objects(cos: &Cos) -> HashMap<String, Vec<ObjRef>> {
    pdfcraft_forms::fields(cos).into_iter().map(|f| (f.name, std::iter::once(f.obj).chain(f.widgets.iter().map(|w| w.obj)).collect())).collect()
}

pub(crate) fn field_payload(view: &View, name: &str) -> Result<String, LiveError> {
    let fields = pdfcraft_forms::fields(view.cos);
    let field = fields.iter().find(|f| f.name == name).ok_or_else(|| LiveError::Apply("no such field".into()))?;
    let anchors = view.anchors();
    let mut enc = Encoder::new(view.cos, &anchors);
    let mut f = Map::new();
    let obj = view.cos.get(field.obj);
    if let Some(d) = obj.as_dict() {
        for key in ["V", "Ff"] {
            if let Some(v) = d.get(key.as_bytes()) {
                f.insert(key.into(), enc.value(v)?);
            }
        }
    }
    let mut widgets = Vec::new();
    for w in &field.widgets {
        let obj = view.cos.get(w.obj);
        let mut wm = Map::new();
        if let Some(d) = obj.as_dict() {
            for key in WIDGET_KEYS {
                if let Some(v) = d.get(key) {
                    wm.insert(String::from_utf8_lossy(key).into_owned(), enc.value(v)?);
                }
            }
        }
        widgets.push(Value::Object(wm));
    }
    f.insert("w".into(), Value::Array(widgets));
    let mut m = header();
    m.insert("f".into(), Value::Object(f));
    m.insert("na".into(), Value::Bool(pdfcraft_forms::need_appearances(view.cos)));
    m.insert("g".into(), Value::Array(enc.finish()?));
    finish(m)
}

/// `false` when the field is not here.
pub(crate) fn apply_field(cos: &mut Cos, ids: &[PageId], name: &str, payload: &Map<String, Value>) -> Result<bool, LiveError> {
    let fields = pdfcraft_forms::fields(cos);
    let Some(field) = fields.iter().find(|f| f.name == name) else { return Ok(false) };
    let f = payload.get("f").and_then(Value::as_object).ok_or_else(|| malformed("the field"))?;
    let g = payload.get("g").and_then(Value::as_array).ok_or_else(|| malformed("the objects"))?;
    let view = View::new(cos, ids);
    let (pages, comments) = view.targets();
    let targets = Targets { pages: &pages, comments: &comments };
    let refs = decode_objects(cos, g, &HashMap::new(), &targets)?;
    let set = |cos: &mut Cos, r: ObjRef, keys: &[&[u8]], from: &Map<String, Value>| -> Result<(), LiveError> {
        let mut values = Vec::new();
        for key in keys {
            let v = match from.get(&*String::from_utf8_lossy(key)) {
                Some(v) => Some(decode_value(v, &refs, &targets, 0)?),
                None => None,
            };
            values.push((key.to_vec(), v));
        }
        cos.update_dict(r, |d| {
            for (key, v) in values {
                match v {
                    Some(v) => d.set(key, v),
                    None => {
                        d.remove(&key);
                    }
                }
            }
        })
        .map_err(|e| LiveError::Apply(e.to_string()))
    };
    set(cos, field.obj, &[b"V", b"Ff"], f)?;
    let widgets = f.get("w").and_then(Value::as_array).ok_or_else(|| malformed("the widgets"))?;
    for (w, v) in field.widgets.iter().zip(widgets) {
        let wm = v.as_object().ok_or_else(|| malformed("a widget"))?;
        set(cos, w.obj, &WIDGET_KEYS, wm)?;
    }
    if payload.get("na").and_then(Value::as_bool) == Some(true) && !pdfcraft_forms::need_appearances(cos) {
        set_need_appearances(cos)?;
    }
    Ok(true)
}

fn set_need_appearances(cos: &mut Cos) -> Result<(), LiveError> {
    let Some(root) = cos.root() else { return Ok(()) };
    let Some(af) = cos.get(root).as_dict().and_then(|d| d.get(b"AcroForm").cloned()) else { return Ok(()) };
    let r = match af {
        Object::Ref(r) => r,
        _ => root,
    };
    let inline = r == root;
    cos.update_dict(r, |d| {
        if inline {
            if let Some(Object::Dict(af)) = d.get_mut(b"AcroForm") {
                af.set(b"NeedAppearances".to_vec(), Object::Bool(true));
            }
        } else {
            d.set(b"NeedAppearances".to_vec(), Object::Bool(true));
        }
    })
    .map_err(|e| LiveError::Apply(e.to_string()))
}

// ── pages ───────────────────────────────────────────────────────────────────────────────────

/// The page order and rotations, for change detection.
pub(crate) fn page_state(view: &View) -> Vec<(PageId, i64)> {
    view.pages.iter().map(|(r, id)| (*id, rotation(view.cos, *r))).collect()
}

/// `known`: the pages the room has; the others travel with their content.
pub(crate) fn pages_payload(view: &View, known: &HashSet<PageId>) -> Result<String, LiveError> {
    let mut m = header();
    let state = page_state(view);
    m.insert("order".into(), Value::Array(state.iter().map(|(id, _)| Value::String(id.to_string())).collect()));
    m.insert("rot".into(), Value::Array(state.iter().map(|(_, r)| Value::from(*r)).collect()));
    let new: Vec<(usize, PageId)> = state.iter().enumerate().filter(|(_, (id, _))| !known.contains(id)).map(|(i, (id, _))| (i, *id)).collect();
    if !new.is_empty() {
        let indices: Vec<usize> = new.iter().map(|(i, _)| *i).collect();
        let mut out = pdfcraft_organize::extract_pages(view.cos, &indices).map_err(|e| LiveError::Apply(e.to_string()))?;
        // The receiver names the new pages by these ids; they are stored in the pages too.
        for (page, (_, id)) in pdfcraft_annot::page_refs(&out).unwrap_or_default().into_iter().zip(&new) {
            stable_ids::store_page_id(&mut out, page, *id);
        }
        let bytes = pdfcraft_cos::write_full(&out, &pdfcraft_cos::SaveOptions::default()).map_err(|e| LiveError::Apply(e.to_string()))?;
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(LiveError::TooLarge);
        }
        m.insert("new".into(), json!({ "ids": new.iter().map(|(_, id)| id.to_string()).collect::<Vec<_>>(), "pdf": B64.encode(&bytes) }));
    }
    finish(m)
}

/// Where pages missing here come from: a payload's PDF, or an earlier state of this document
/// (undo) with its page ids.
pub(crate) enum PageSource<'a> {
    Payload,
    State(&'a Cos, &'a [PageId]),
}

/// Bring the pages of `cos` (ids `ids`) to the order and rotations of `payload`: delete the pages
/// it lacks, insert the ones missing here, reorder, rotate. Comments and fields on deleted pages
/// go with them, as Delete pages does.
pub(crate) fn apply_pages(cos: &mut Cos, ids: &[PageId], payload: &Map<String, Value>, source: PageSource) -> Result<(), LiveError> {
    let order: Vec<PageId> = payload
        .get("order")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("the page order"))?
        .iter()
        .map(|v| v.as_str().and_then(PageId::parse).ok_or_else(|| malformed("a page id")))
        .collect::<Result<_, _>>()?;
    let rot: Vec<i64> = payload.get("rot").and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_i64().unwrap_or(0)).collect()).unwrap_or_default();
    if order.is_empty() {
        return Err(malformed("an empty page order"));
    }
    let organize = |e: pdfcraft_organize::OrganizeError| LiveError::Apply(e.to_string());
    let mut by_ref: HashMap<ObjRef, PageId> = pdfcraft_annot::page_refs(cos).unwrap_or_default().into_iter().zip(ids.iter().copied()).collect();
    let wanted: HashSet<PageId> = order.iter().copied().collect();
    let current = |cos: &Cos, by_ref: &HashMap<ObjRef, PageId>| -> Vec<Option<PageId>> {
        pdfcraft_annot::page_refs(cos).unwrap_or_default().into_iter().map(|r| by_ref.get(&r).copied()).collect()
    };
    // Insert first: deleting every page is refused, and the order may share no page with ours.
    let have: HashSet<PageId> = current(cos, &by_ref).into_iter().flatten().collect();
    let missing: Vec<PageId> = order.iter().copied().filter(|id| !have.contains(id)).collect();
    if !missing.is_empty() {
        let (src, src_ids): (Cos, Vec<PageId>) = match source {
            PageSource::Payload => {
                let new = payload.get("new").and_then(Value::as_object);
                let src_ids = new
                    .and_then(|n| n.get("ids"))
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().and_then(PageId::parse)).collect())
                    .unwrap_or_default();
                match new.and_then(|n| n.get("pdf")).and_then(Value::as_str) {
                    Some(pdf) => {
                        let bytes = B64.decode(pdf).map_err(|_| malformed("the pages' PDF"))?;
                        (Cos::open(Arc::new(bytes)).map_err(|e| LiveError::Apply(e.to_string()))?, src_ids)
                    }
                    None => (Cos::new_empty(), Vec::new()),
                }
            }
            PageSource::State(src, src_ids) => (src.clone(), src_ids.to_vec()),
        };
        let take: Vec<(usize, PageId)> = missing.iter().filter_map(|id| src_ids.iter().position(|s| s == id).map(|i| (i, *id))).collect();
        if !take.is_empty() {
            let at = by_ref.len();
            let indices: Vec<usize> = take.iter().map(|(i, _)| *i).collect();
            let outcome =
                pdfcraft_organize::import_pages_with(cos, &src, &indices, at, pdfcraft_organize::FieldConflicts::Merge).map_err(organize)?;
            for (page, (_, id)) in outcome.pages.iter().zip(&take) {
                stable_ids::store_page_id(cos, *page, *id);
                by_ref.insert(*page, *id);
            }
        }
    }
    let doomed: Vec<usize> =
        current(cos, &by_ref).iter().enumerate().filter(|(_, id)| id.is_none_or(|id| !wanted.contains(&id))).map(|(i, _)| i).collect();
    if !doomed.is_empty() {
        pdfcraft_organize::delete_pages(cos, &doomed).map_err(organize)?;
    }
    // Ids this side can't place (a page whose content never came) are left out.
    let target: Vec<PageId> = {
        let present: HashSet<PageId> = current(cos, &by_ref).into_iter().flatten().collect();
        order.iter().copied().filter(|id| present.contains(id)).collect()
    };
    reorder(cos, &by_ref, &target)?;
    for (i, page) in pdfcraft_annot::page_refs(cos).unwrap_or_default().into_iter().enumerate() {
        let Some(id) = by_ref.get(&page) else { continue };
        let Some(k) = order.iter().position(|o| o == id) else { continue };
        let want = rot.get(k).copied().unwrap_or(0).rem_euclid(360);
        let delta = (want - rotation(cos, page)).rem_euclid(360);
        if delta != 0 && delta % 90 == 0 {
            pdfcraft_organize::rotate_pages(cos, &[i], delta).map_err(organize)?;
        }
    }
    Ok(())
}

/// Move pages until their order is `target`, keeping the longest run already in order in place.
fn reorder(cos: &mut Cos, by_ref: &HashMap<ObjRef, PageId>, target: &[PageId]) -> Result<(), LiveError> {
    let now: Vec<Option<PageId>> = pdfcraft_annot::page_refs(cos).unwrap_or_default().into_iter().map(|r| by_ref.get(&r).copied()).collect();
    if now.iter().copied().eq(target.iter().map(|t| Some(*t))) {
        return Ok(());
    }
    let rank: HashMap<PageId, usize> = target.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let ranks: Vec<usize> = now.iter().filter_map(|id| id.and_then(|id| rank.get(&id).copied())).collect();
    let keep: HashSet<usize> = longest_increasing(&ranks).into_iter().collect();
    for (k, id) in target.iter().enumerate() {
        if keep.contains(&k) {
            continue;
        }
        let refs = pdfcraft_annot::page_refs(cos).unwrap_or_default();
        let Some(from) = refs.iter().position(|r| by_ref.get(r) == Some(id)) else { continue };
        // Where it goes: right after the page that precedes it in the target, as it stands now.
        let to = match k.checked_sub(1).and_then(|p| target.get(p)) {
            Some(prev) => refs.iter().position(|r| by_ref.get(r) == Some(prev)).map_or(0, |p| if p < from { p + 1 } else { p }),
            None => 0,
        };
        if to != from {
            pdfcraft_organize::move_pages(cos, &[from], to).map_err(|e| LiveError::Apply(e.to_string()))?;
        }
    }
    Ok(())
}

/// The values of the longest strictly increasing subsequence of `xs`.
fn longest_increasing(xs: &[usize]) -> Vec<usize> {
    let n = xs.len();
    let mut len = vec![1usize; n];
    let mut prev: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        for j in 0..i {
            if xs.get(j) < xs.get(i) && len.get(j).map(|l| l + 1) > len.get(i).copied() {
                len[i] = len[j] + 1;
                prev[i] = Some(j);
            }
        }
    }
    let mut best = (0..n).max_by_key(|i| len.get(*i).copied().unwrap_or(0));
    let mut out = Vec::new();
    while let Some(i) = best {
        if let Some(x) = xs.get(i) {
            out.push(*x);
        }
        best = prev.get(i).copied().flatten();
    }
    out.reverse();
    out
}

// ── the whole document ──────────────────────────────────────────────────────────────────────

/// Whether a working file of `len` bytes can travel as `*`.
pub(crate) fn document_fits(len: usize) -> bool {
    // Base64 grows the file by a third.
    len.saturating_mul(4) / 3 <= MAX_PAYLOAD_BYTES.saturating_sub(64)
}

pub(crate) fn document_payload(bytes: &[u8]) -> Result<String, LiveError> {
    if !document_fits(bytes.len()) {
        return Err(LiveError::TooLarge);
    }
    let mut m = header();
    m.insert("pdf".into(), Value::String(B64.encode(bytes)));
    finish(m)
}

pub(crate) fn document_bytes(payload: &Map<String, Value>) -> Result<Vec<u8>, LiveError> {
    payload.get("pdf").and_then(Value::as_str).and_then(|s| B64.decode(s).ok()).ok_or_else(|| malformed("the document"))
}

/// The region's payload in `view`; `known` as for [`pages_payload`], `bytes` the working file.
pub(crate) fn payload(view: &View, region: &Region, known: &HashSet<PageId>, bytes: &[u8]) -> Result<String, LiveError> {
    match region {
        Region::Annot(id) => comment_payload(view, id),
        Region::Field(name) => field_payload(view, name),
        Region::Pages => pages_payload(view, known),
        Region::Document => document_payload(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::longest_increasing;

    #[test]
    fn longest_run_kept() {
        assert_eq!(longest_increasing(&[1, 2, 3, 0]), vec![1, 2, 3]);
        assert_eq!(longest_increasing(&[3, 0, 1, 2]), vec![0, 1, 2]);
        assert_eq!(longest_increasing(&[]), Vec::<usize>::new());
    }
}
