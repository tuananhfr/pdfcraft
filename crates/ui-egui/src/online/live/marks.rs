//! Others on the pages: a region someone else holds gets an outline in their colour with their
//! name; someone's selected comment or field a thinner one; the page someone is on a name tag.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Stroke, StrokeKind, pos2, vec2};
use std::collections::HashMap;

use pdfcraft_engine::live::Region;
use pdfcraft_engine::{AnnotId, DocId, PageId};
use pdfcraft_render::DocInfo;

use super::Live;
use crate::PdfCraftApp;
use crate::canvas::PageXform;

/// Others' presence on one page, as the canvas draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub page: usize,
    /// In user space; `None` for a name tag at the page's corner.
    pub rect: Option<[f32; 4]>,
    pub colour: Color32,
    pub name: String,
    /// Someone holds it (a lock), rather than has it selected.
    pub lock: bool,
}

/// A page, and the rectangle on it in user space (`None`: the page itself).
type Place = (usize, Option<[f32; 4]>);

/// `#rrggbb`, or a neutral grey.
fn colour(hex: Option<&String>) -> Color32 {
    let parsed = hex.and_then(|h| h.strip_prefix('#')).filter(|h| h.len() == 6).and_then(|h| u32::from_str_radix(h, 16).ok());
    match parsed {
        Some(v) => Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
        None => Color32::from_gray(120),
    }
}

impl Live {
    /// The marks of the regions others hold and select in document `id`.
    fn build_marks(&self, regions: &dyn Fn(&str) -> Vec<Place>) -> Vec<Mark> {
        let mut out = Vec::new();
        let mut others: Vec<&super::LockView> = self.others.values().collect();
        others.sort_by(|a, b| a.region.cmp(&b.region));
        for lock in others {
            let name = self.name_of(&lock.holder, lock.name.as_ref()).unwrap_or_default();
            let colour = colour(self.colour_of(&lock.holder, lock.colour.as_ref()).as_ref());
            for (page, rect) in regions(&lock.region) {
                out.push(Mark { page, rect, colour, name: name.clone(), lock: true });
            }
        }
        for cursor in &self.cursors {
            let name = self.name_of(&cursor.author, cursor.name.as_ref()).unwrap_or_default();
            let colour = colour(self.colour_of(&cursor.author, cursor.colour.as_ref()).as_ref());
            for (page, rect) in regions(&cursor.region) {
                // A lock already outlines it.
                if out.iter().any(|m| m.lock && m.page == page && m.rect == rect && m.rect.is_some()) {
                    continue;
                }
                out.push(Mark { page, rect, colour, name: name.clone(), lock: false });
            }
        }
        out
    }
}

/// Where comments, fields and pages are in one document, read once for every region's mark.
struct Lookup {
    pages: HashMap<PageId, usize>,
    annots: HashMap<AnnotId, (usize, usize)>,
    rects: HashMap<(usize, usize), Vec<[f32; 4]>>,
    fields: HashMap<String, Vec<Place>>,
}

impl Lookup {
    fn new(app: &mut PdfCraftApp, id: DocId) -> Self {
        let mut pages = HashMap::new();
        for (i, p) in app.session.page_ids(id).into_iter().enumerate() {
            pages.entry(p).or_insert(i);
        }
        let mut annots = HashMap::new();
        for a in app.session.annotation_ids(id) {
            annots.entry(a.id).or_insert((a.page, a.index));
        }
        let mut rects: HashMap<(usize, usize), Vec<[f32; 4]>> = HashMap::new();
        let mut fields: HashMap<String, Vec<Place>> = HashMap::new();
        if let Some(doc) = app.session.get(id) {
            for a in &doc.info.annotations {
                rects.entry((a.page, a.index)).or_default().push(a.rect);
            }
            for f in doc.form.iter() {
                fields
                    .entry(f.name.clone())
                    .or_default()
                    .extend(f.widgets.iter().filter_map(|w| w.page.map(|p| (p, Some(w.rect.map(|v| v as f32))))));
            }
        }
        Self { pages, annots, rects, fields }
    }

    /// Where region `region` is on the pages: comments and fields by their rectangles,
    /// `page:<id>` (a cursor) as the page's name tag.
    fn places(&self, region: &str) -> Vec<Place> {
        if let Some(page) = region.strip_prefix("page:").and_then(PageId::parse) {
            return self.pages.get(&page).map(|p| vec![(*p, None)]).unwrap_or_default();
        }
        match Region::parse(region) {
            Some(Region::Annot(annot)) => {
                let Some(&(page, index)) = self.annots.get(&annot) else { return Vec::new() };
                self.rects.get(&(page, index)).into_iter().flatten().map(|r| (page, Some(*r))).collect()
            }
            Some(Region::Field(name)) => self.fields.get(&name).cloned().unwrap_or_default(),
            _ => Vec::new(),
        }
    }
}

impl PdfCraftApp {
    /// Draw the marks again when the locks, the cursors or the document changed.
    pub(crate) fn refresh_live_marks(&mut self) {
        let Some(live) = self.online.as_ref().and_then(|o| o.live.as_ref()) else { return };
        let Some(id) = live.doc else { return };
        let generation = self.session.get(id).map(|d| d.edit_generation()).unwrap_or(0);
        let key = (id, live.revision, generation);
        if live.marks_key == Some(key) {
            return;
        }
        let regions: Vec<String> = live.others.keys().cloned().chain(live.cursors.iter().map(|c| c.region.clone())).collect();
        let places: HashMap<String, Vec<Place>> = if regions.is_empty() {
            HashMap::new()
        } else {
            let lookup = Lookup::new(self, id);
            regions
                .into_iter()
                .map(|r| {
                    let at = lookup.places(&r);
                    (r, at)
                })
                .collect()
        };
        let Some(live) = self.online.as_mut().and_then(|o| o.live.as_mut()) else { return };
        let marks = live.build_marks(&|r| places.get(r).cloned().unwrap_or_default());
        live.marks_key = Some(key);
        for view in self.views.iter_mut().filter(|v| v.id == id) {
            view.live_marks = marks.clone();
        }
    }
}

/// Paint the marks of page `page`.
pub(crate) fn paint_page(painter: &egui::Painter, xf: &PageXform, info: &DocInfo, page: usize, marks: &[Mark]) {
    if info.pages.get(page).is_none() {
        return;
    }
    let font = FontId::proportional(11.0);
    for mark in marks.iter().filter(|m| m.page == page) {
        let anchor = match mark.rect {
            Some(r) => {
                let rect = xf.user_rect(info, page, r).expand(3.0);
                let width = if mark.lock { 2.0 } else { 1.0 };
                painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(width, mark.colour), StrokeKind::Outside);
                rect.left_top()
            }
            None => xf.rect.left_top() + vec2(4.0, 4.0),
        };
        if mark.name.is_empty() {
            continue;
        }
        let galley = painter.layout_no_wrap(mark.name.clone(), font.clone(), Color32::WHITE);
        let size = galley.size() + vec2(8.0, 4.0);
        let top = if mark.rect.is_some() { anchor.y - size.y } else { anchor.y };
        let tag = Rect::from_min_size(pos2(anchor.x, top), size);
        painter.rect_filled(tag, CornerRadius::same(3), mark.colour);
        painter.text(tag.center(), Align2::CENTER_CENTER, &mark.name, font.clone(), Color32::WHITE);
    }
}
