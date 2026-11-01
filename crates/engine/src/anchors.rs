//! Where an office Cloud comment points in a PDF (ERPCons M5, the `erpHost` comment calls). The
//! anchor is JSON the shell stores on the server with the comment; nothing of it is ever written
//! into the file, and the file's own comments are untouched.
//!
//! ```json
//! {"v":1,
//!  "region":"annot:nm:…" | "field:<full name>" | null,
//!  "pos":{"page":"<page id>" | null,"quads":[[x1,y1,x2,y2,x3,y3,x4,y4], …]},
//!  "soft":{"index":<0-based page>,"text":"<text under the quads>"},
//!  "label":"Trang 3"}
//! ```
//!
//! `region` is the co-editing region id of a selected comment or form field; a text selection has
//! none. `pos` is where the anchor is now (user-space quads on the page named by its stable id),
//! `soft` what finds it again when the region is gone or the page ids changed with a new version
//! of the file: the page's position and the text under the quads. `label` is the shell's hint for
//! people; it is made by the app and ignored when an anchor is read.

use pdfcraft_render::{PageInfo, PageRenderer, PageText, RenderRequest, RequestKind};
use serde_json::{Map, Value, json};

use crate::live::Region;
use crate::stable_ids::PageId;
use crate::{DocId, Session};

pub const ANCHOR_VERSION: u64 = 1;
/// An anchor is a small hint; anything larger is refused unread.
pub const MAX_ANCHOR_BYTES: usize = 16 * 1024;
const MAX_QUADS: usize = 64;
/// The text kept to find an anchor again: enough to tell passages apart, small on the server.
pub const MAX_TEXT_CHARS: usize = 200;
/// Pages a document can have, as far as an anchor's page index is concerned.
const MAX_INDEX: u64 = 1 << 24;

#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    /// `annot:` or `field:`; `None` for a text selection.
    pub region: Option<Region>,
    pub page: Option<PageId>,
    /// User space, as text-markup quads: top-left, top-right, bottom-left, bottom-right.
    pub quads: Vec<[f64; 8]>,
    /// 0-based page the anchor was on when made or last resolved.
    pub index: usize,
    /// The text under `quads` then, whitespace collapsed, at most [`MAX_TEXT_CHARS`] characters.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnchorError {
    #[error("the anchor comes from a newer version of the editor")]
    Version,
    #[error("the anchor can't be read ({0})")]
    Malformed(&'static str),
}

/// Whether an anchor was found again ([`Session::resolve_anchors`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// Found, as it is now.
    Open(Anchor),
    Orphaned,
}

/// Where an anchor is drawn: a page and quads on it in user space.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub page: usize,
    pub quads: Vec<[f64; 8]>,
}

impl Anchor {
    /// The anchor from the shell's JSON (any `label` is ignored).
    pub fn parse(json: &str) -> Result<Self, AnchorError> {
        if json.len() > MAX_ANCHOR_BYTES {
            return Err(AnchorError::Malformed("too large"));
        }
        let value: Value = serde_json::from_str(json).map_err(|_| AnchorError::Malformed("not JSON"))?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self, AnchorError> {
        let obj = value.as_object().ok_or(AnchorError::Malformed("not an object"))?;
        match obj.get("v").and_then(Value::as_u64) {
            Some(ANCHOR_VERSION) => {}
            Some(_) => return Err(AnchorError::Version),
            None => return Err(AnchorError::Malformed("no version")),
        }
        // Order locks and `*` name no place.
        let region = obj.get("region").and_then(Value::as_str).and_then(Region::parse).filter(|r| matches!(r, Region::Annot(_) | Region::Field(_)));
        let pos = obj.get("pos").and_then(Value::as_object);
        let page = pos.and_then(|p| p.get("page")).and_then(Value::as_str).and_then(PageId::parse);
        let quads =
            pos.and_then(|p| p.get("quads")).and_then(Value::as_array).map(|q| rounded(q.iter().filter_map(quad).collect())).unwrap_or_default();
        let soft = obj.get("soft").and_then(Value::as_object).ok_or(AnchorError::Malformed("no soft anchor"))?;
        let index = soft.get("index").and_then(Value::as_u64).filter(|i| *i < MAX_INDEX).ok_or(AnchorError::Malformed("no page index"))?;
        let text = soft.get("text").and_then(Value::as_str).map(clean_text).unwrap_or_default();
        Ok(Self { region, page, quads, index: usize::try_from(index).map_err(|_| AnchorError::Malformed("page index"))?, text })
    }

    /// The shell's JSON, with `label` for people.
    pub fn to_json(&self, label: &str) -> String {
        self.to_value(label).to_string()
    }

    pub fn to_value(&self, label: &str) -> Value {
        let mut pos = Map::new();
        pos.insert("page".into(), self.page.map_or(Value::Null, |p| Value::String(p.to_string())));
        pos.insert(
            "quads".into(),
            Value::Array(rounded(self.quads.clone()).iter().map(|q| Value::Array(q.iter().map(|v| json!(v)).collect())).collect()),
        );
        json!({
            "v": ANCHOR_VERSION,
            "region": self.region.as_ref().map_or(Value::Null, |r| Value::String(r.to_string())),
            "pos": pos,
            "soft": { "index": self.index, "text": self.text },
            "label": label,
        })
    }
}

/// Two decimals: a hundredth of a point is below anything drawn, and keeps the JSON short.
fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Quads as the JSON carries them, so an anchor reads back equal to itself.
fn rounded(quads: Vec<[f64; 8]>) -> Vec<[f64; 8]> {
    quads.into_iter().take(MAX_QUADS).map(|q| q.map(round)).collect()
}

fn quad(value: &Value) -> Option<[f64; 8]> {
    let a = value.as_array().filter(|a| a.len() == 8)?;
    let mut q = [0.0; 8];
    for (slot, v) in q.iter_mut().zip(a) {
        *slot = v.as_f64().filter(|f| f.is_finite() && f.abs() < 1e7)?;
    }
    Some(q)
}

/// Whitespace collapsed to single spaces, trimmed, cut to [`MAX_TEXT_CHARS`] characters.
pub fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_TEXT_CHARS).collect()
}

/// Text layouts differ in the spaces they reconstruct; the letters decide.
fn same_text(a: &str, b: &str) -> bool {
    a.chars().filter(|c| !c.is_whitespace()).eq(b.chars().filter(|c| !c.is_whitespace()))
}

/// A user-space rectangle as a quad (top-left, top-right, bottom-left, bottom-right, unrotated).
pub fn rect_quad(r: [f64; 4]) -> [f64; 8] {
    let (x0, y0, x1, y1) = (r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3]));
    [x0, y1, x1, y1, x0, y0, x1, y0]
}

/// The bounding box of quads in user space; `None` for none.
pub fn quads_bounds(quads: &[[f64; 8]]) -> Option<[f64; 4]> {
    let mut points = quads.iter().flat_map(|q| q.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)));
    let (x, y) = points.next()?;
    Some(points.fold([x, y, x, y], |b, (x, y)| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]))
}

/// A quad's box in view space (the text layer's space), widened by half a point.
fn view_box(page: &PageInfo, q: &[f64; 8]) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for &[px, py] in q.as_chunks::<2>().0 {
        let [x, y] = page.user_to_view(px as f32, py as f32);
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    [b[0] - 0.5, b[1] - 0.5, b[2] + 0.5, b[3] + 0.5]
}

/// The text of the glyphs whose centre is under `quads` ([`clean_text`]): what an anchor keeps
/// and checks. The app calls it on its own text layer, the engine on a fresh one.
pub fn text_under(text: &PageText, page: &PageInfo, quads: &[[f64; 8]]) -> String {
    let boxes: Vec<[f32; 4]> = quads.iter().map(|q| view_box(page, q)).collect();
    let mut out = String::new();
    let mut last_line = None;
    for (i, g) in text.glyphs.iter().enumerate() {
        let (cx, cy) = ((g.rect[0] + g.rect[2]) / 2.0, (g.rect[1] + g.rect[3]) / 2.0);
        if !boxes.iter().any(|b| cx >= b[0] && cx <= b[2] && cy >= b[1] && cy <= b[3]) {
            continue;
        }
        let line = text.line_of.get(i).copied();
        if !out.is_empty() && (line != last_line || text.space_before.get(i).copied().unwrap_or(false)) {
            out.push(' ');
        }
        out.push_str(&g.text);
        last_line = line;
        if out.len() > MAX_TEXT_CHARS * 8 {
            break;
        }
    }
    clean_text(&out)
}

/// Where `needle` is on the page, ignoring whitespace: one quad per line of the match nearest to
/// `near` (a user-space point).
fn find_text(text: &PageText, page: &PageInfo, needle: &str, near: Option<[f64; 2]>) -> Option<Vec<[f64; 8]>> {
    let needle: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if needle.is_empty() {
        return None;
    }
    let hay: Vec<(char, usize)> =
        text.glyphs.iter().enumerate().flat_map(|(i, g)| g.text.chars().filter(|c| !c.is_whitespace()).map(move |c| (c, i))).collect();
    let mut best: Option<(f64, Vec<[f64; 8]>)> = None;
    for start in 0..hay.len().saturating_sub(needle.len().saturating_sub(1)) {
        let Some(window) = hay.get(start..start.saturating_add(needle.len())) else { break };
        if !window.iter().map(|(c, _)| *c).eq(needle.iter().copied()) {
            continue;
        }
        let (Some(first), Some(last)) = (window.first().map(|w| w.1), window.last().map(|w| w.1)) else { continue };
        let quads = line_quads(text, page, first, last);
        let distance = match (near, quads_bounds(&quads)) {
            (Some([x, y]), Some(b)) => (x - (b[0] + b[2]) / 2.0).hypot(y - (b[1] + b[3]) / 2.0),
            _ => 0.0,
        };
        if best.as_ref().is_none_or(|(d, _)| distance < *d) {
            best = Some((distance, quads));
        }
    }
    best.map(|(_, q)| q)
}

/// Glyphs `first..=last` as one quad per line.
fn line_quads(text: &PageText, page: &PageInfo, first: usize, last: usize) -> Vec<[f64; 8]> {
    let mut lines: Vec<(Option<u32>, [f32; 4])> = Vec::new();
    for i in first..=last {
        let Some(g) = text.glyphs.get(i) else { break };
        let line = text.line_of.get(i).copied();
        match lines.last_mut() {
            Some((l, r)) if *l == line => *r = [r[0].min(g.rect[0]), r[1].min(g.rect[1]), r[2].max(g.rect[2]), r[3].max(g.rect[3])],
            _ => lines.push((line, g.rect)),
        }
    }
    lines.into_iter().map(|(_, r)| page.view_rect_to_quad(r)).collect()
}

/// The text layer of each page asked for, read once per call.
struct Texts {
    renderer: Option<PageRenderer>,
    pages: Vec<(usize, Option<std::sync::Arc<PageText>>)>,
}

impl Texts {
    fn get(&mut self, page: usize) -> Option<std::sync::Arc<PageText>> {
        if let Some((_, t)) = self.pages.iter().find(|(p, _)| *p == page) {
            return t.clone();
        }
        let text =
            self.renderer.as_mut().and_then(|r| r.render(RenderRequest { page, kind: RequestKind::Text, scale: 1.0, ..Default::default() }).text);
        self.pages.push((page, text.clone()));
        text
    }
}

impl Session {
    /// The anchor of the comment at `index` in page `page`'s `/Annots`: its region, its quads (a
    /// text markup's, else its rectangle) and, for a text markup, the text it marks, read from
    /// `layer` (the page's text layer the app has) or else from the page.
    pub fn anchor_of_annotation(&mut self, id: DocId, page: usize, index: usize, layer: Option<&PageText>) -> Option<Anchor> {
        let annot = self.annotation_ids(id).into_iter().find(|a| a.page == page && a.index == index)?.id;
        let region = Region::Annot(annot);
        let place = self.region_place(id, &region)?;
        let markup = self.get(id)?.info.annotations.iter().any(|a| a.page == page && a.index == index && !a.quads.is_empty());
        let text = if markup {
            let fresh = if layer.is_none() { self.texts(id).get(place.page) } else { None };
            layer.or(fresh.as_deref()).zip(self.get(id)?.info.pages.get(place.page)).map(|(t, p)| text_under(t, p, &place.quads)).unwrap_or_default()
        } else {
            String::new()
        };
        self.anchor_at(id, Some(region), place, text)
    }

    /// The anchor of form field `name`, at its widget `widget` (or its first one on a page).
    pub fn anchor_of_field(&mut self, id: DocId, name: &str, widget: usize) -> Option<Anchor> {
        let field = self.get(id)?.form.iter().find(|f| f.name == name)?.clone();
        let w = field.widgets.get(widget).filter(|w| w.page.is_some()).or_else(|| field.widgets.iter().find(|w| w.page.is_some()))?;
        let place = Place { page: w.page?, quads: vec![rect_quad(w.rect)] };
        self.anchor_at(id, Some(Region::Field(field.name.clone())), place, String::new())
    }

    /// The anchor of selected text: `quads` on page `page`, `text` read from under them with
    /// [`text_under`].
    pub fn anchor_of_text(&mut self, id: DocId, page: usize, quads: Vec<[f64; 8]>, text: &str) -> Option<Anchor> {
        if quads.is_empty() {
            return None;
        }
        self.anchor_at(id, None, Place { page, quads }, clean_text(text))
    }

    fn anchor_at(&mut self, id: DocId, region: Option<Region>, place: Place, text: String) -> Option<Anchor> {
        if place.page >= self.get(id)?.info.pages.len() {
            return None;
        }
        let page = self.page_ids(id).get(place.page).copied();
        Some(Anchor { region, page, quads: rounded(place.quads), index: place.page, text })
    }

    /// Where a comment or a field is now; `None` when it is gone.
    fn region_place(&self, id: DocId, region: &Region) -> Option<Place> {
        let doc = self.get(id)?;
        match region {
            Region::Annot(annot) => {
                let (page, index) = self.find_annotation(id, annot)?;
                let a = doc.info.annotations.iter().find(|a| a.page == page && a.index == index)?;
                let quads =
                    if a.quads.is_empty() { vec![rect_quad(a.rect.map(f64::from))] } else { a.quads.iter().map(|q| q.map(f64::from)).collect() };
                Some(Place { page, quads })
            }
            Region::Field(name) => {
                let w = doc.form.iter().find(|f| &f.name == name)?.widgets.iter().find(|w| w.page.is_some())?;
                Some(Place { page: w.page?, quads: vec![rect_quad(w.rect)] })
            }
            Region::Pages | Region::Document => None,
        }
    }

    /// Where to draw `anchor` now, without reading any text: its region's place, else its quads
    /// on the page with its id, else on the page at its index.
    pub fn anchor_place(&mut self, id: DocId, anchor: &Anchor) -> Option<Place> {
        if let Some(region) = &anchor.region {
            return self.region_place(id, region);
        }
        let count = self.get(id)?.info.pages.len();
        let page = anchor.page.and_then(|p| self.page_index(id, p)).or((anchor.index < count).then_some(anchor.index))?;
        (!anchor.quads.is_empty()).then(|| Place { page, quads: anchor.quads.clone() })
    }

    /// Find each anchor again in the document as it is now (a new version opened, someone else's
    /// change applied). A region that still exists keeps the anchor, at the region's place. Else
    /// the text under the quads must still be on the page named by the anchor's id or at its
    /// index (moved within that page if need be), and the anchor loses its region. Else it is
    /// orphaned. Changes nothing in the document.
    pub fn resolve_anchors(&mut self, id: DocId, anchors: &[Anchor]) -> Vec<Resolution> {
        let mut texts = self.texts(id);
        anchors.iter().map(|a| self.resolve_anchor(id, a, &mut texts)).collect()
    }

    fn resolve_anchor(&mut self, id: DocId, anchor: &Anchor, texts: &mut Texts) -> Resolution {
        let Some(count) = self.get(id).map(|d| d.info.pages.len()) else { return Resolution::Orphaned };
        if let Some(region) = &anchor.region
            && let Some(place) = self.region_place(id, region)
        {
            let page = self.page_ids(id).get(place.page).copied();
            return Resolution::Open(Anchor {
                region: Some(region.clone()),
                page,
                quads: rounded(place.quads),
                index: place.page,
                text: anchor.text.clone(),
            });
        }
        let by_id = anchor.page.and_then(|p| self.page_index(id, p));
        let by_index = (anchor.index < count).then_some(anchor.index);
        if anchor.text.is_empty() {
            // Nothing to check a page against: only a selection's own page keeps it.
            return match (&anchor.region, by_id.or(by_index.filter(|_| anchor.page.is_none()))) {
                (None, Some(page)) if !anchor.quads.is_empty() => self.reanchored(id, page, anchor.quads.clone(), anchor),
                _ => Resolution::Orphaned,
            };
        }
        let near = quads_bounds(&anchor.quads).map(|b| [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]);
        let mut candidates = Vec::new();
        candidates.extend(by_id);
        candidates.extend(by_index.filter(|p| Some(*p) != by_id));
        for page in candidates {
            let (Some(text), Some(info)) = (texts.get(page), self.get(id).and_then(|d| d.info.pages.get(page)).cloned()) else { continue };
            if !anchor.quads.is_empty() && same_text(&text_under(&text, &info, &anchor.quads), &anchor.text) {
                return self.reanchored(id, page, anchor.quads.clone(), anchor);
            }
            if let Some(quads) = find_text(&text, &info, &anchor.text, near) {
                return self.reanchored(id, page, quads, anchor);
            }
        }
        Resolution::Orphaned
    }

    fn reanchored(&mut self, id: DocId, page: usize, quads: Vec<[f64; 8]>, anchor: &Anchor) -> Resolution {
        let page_id = self.page_ids(id).get(page).copied();
        Resolution::Open(Anchor { region: None, page: page_id, quads: rounded(quads), index: page, text: anchor.text.clone() })
    }

    fn texts(&self, id: DocId) -> Texts {
        let renderer = self.get(id).map(|d| PageRenderer::new(d.display.clone(), d.config.clone()));
        Texts { renderer, pages: Vec::new() }
    }
}
