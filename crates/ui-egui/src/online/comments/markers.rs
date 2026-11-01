//! Comment markers on the pages: a small bubble with the thread's count beside its anchor, the
//! anchor shaded while the pointer is on the bubble, a click reported to the shell. Drawn over
//! the page like the find highlights, never part of the document.

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pdfcraft_render::DocInfo;

use crate::canvas::{DocView, PageXform};

/// One open thread's marker on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct CommentMarker {
    pub thread: String,
    pub page: usize,
    /// The anchor's quads as user-space boxes.
    pub rects: Vec<[f32; 4]>,
    pub count: u32,
}

const BUBBLE: egui::Vec2 = vec2(22.0, 16.0);
const FILL: Color32 = Color32::from_rgb(0xF5, 0xB4, 0x00);
const INK: Color32 = Color32::from_rgb(0x2B, 0x22, 0x00);

/// The bubbles of page `page` on screen, side by side where they would overlap.
fn bubbles(xf: &PageXform, info: &DocInfo, page: usize, markers: &[CommentMarker]) -> Vec<(usize, Rect, Vec<Rect>)> {
    let mut out: Vec<(usize, Rect, Vec<Rect>)> = Vec::new();
    for (i, m) in markers.iter().enumerate().filter(|(_, m)| m.page == page) {
        let shaded: Vec<Rect> = m.rects.iter().map(|r| xf.user_rect(info, page, *r)).collect();
        let Some(anchor) = shaded.iter().copied().reduce(|a, b| a.union(b)) else { continue };
        let page_rect = xf.rect;
        let x = (anchor.right() + 4.0).min(page_rect.right() - BUBBLE.x).max(page_rect.left());
        let y = (anchor.top() - BUBBLE.y / 2.0).clamp(page_rect.top(), (page_rect.bottom() - BUBBLE.y).max(page_rect.top()));
        let mut bubble = Rect::from_min_size(pos2(x, y), BUBBLE);
        while out.iter().any(|(_, b, _)| b.intersects(bubble)) {
            bubble = bubble.translate(vec2(BUBBLE.x + 2.0, 0.0));
        }
        out.push((i, bubble, shaded));
    }
    out
}

/// Draw page `page`'s markers; a click on one leaves its thread in `view.comment_clicked`.
pub(crate) fn paint_markers(ui: &egui::Ui, painter: &egui::Painter, xf: &PageXform, info: &DocInfo, page: usize, view: &mut DocView) {
    if info.pages.get(page).is_none() || view.comment_markers.is_empty() {
        return;
    }
    let font = FontId::proportional(10.5);
    for (i, bubble, shaded) in bubbles(xf, info, page, &view.comment_markers) {
        let Some(marker) = view.comment_markers.get(i) else { continue };
        let response = ui.interact(bubble, ui.id().with(("comment-marker", page, &marker.thread)), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            for r in &shaded {
                painter.rect_filled(r.expand(1.0), CornerRadius::same(2), FILL.gamma_multiply(0.3));
                painter.rect_stroke(r.expand(1.0), CornerRadius::same(2), Stroke::new(1.5, FILL), StrokeKind::Outside);
            }
        }
        painter.rect_filled(bubble, CornerRadius::same(4), FILL);
        painter.rect_stroke(bubble, CornerRadius::same(4), Stroke::new(1.0, INK.gamma_multiply(0.5)), StrokeKind::Inside);
        // The bubble's tail, towards the anchor.
        let tail = [bubble.left_bottom() + vec2(4.0, 0.0), bubble.left_bottom() + vec2(10.0, 0.0), bubble.left_bottom() + vec2(3.0, 5.0)];
        painter.add(egui::Shape::convex_polygon(tail.to_vec(), FILL, Stroke::NONE));
        let count = if marker.count > 99 { "99+".to_owned() } else { marker.count.max(1).to_string() };
        painter.text(bubble.center(), Align2::CENTER_CENTER, count, font.clone(), INK);
        if response.clicked() {
            view.comment_clicked = Some(marker.thread.clone());
        }
    }
}

#[cfg(test)]
pub(super) fn bubble_rects(xf: &PageXform, info: &DocInfo, page: usize, markers: &[CommentMarker]) -> Vec<Rect> {
    bubbles(xf, info, page, markers).into_iter().map(|(_, b, _)| b).collect()
}
