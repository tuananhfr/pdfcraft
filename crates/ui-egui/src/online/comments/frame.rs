//! One frame of the comment calls: the shell's events, the selection's anchor, the markers and
//! the clicks on them.

use pdfcraft_engine::DocId;
use pdfcraft_engine::anchors::{self, Anchor, Resolution};
use pdfcraft_engine::live::Region;

use super::{AnchorStatus, CommentEvent, CommentMarker, ResolvedThread, Selected, Thread};
use crate::PdfCraftApp;

/// `label` of an anchor: the page as people count it.
fn label(index: usize) -> String {
    crate::i18n::fmt(tl!("Page {n}"), &[("n", &index.saturating_add(1).to_string())])
}

fn anchor_json(anchor: &Anchor) -> String {
    anchor.to_json(&label(anchor.index))
}

impl PdfCraftApp {
    /// The comment calls for this frame (web only, when the shell offers them).
    pub(crate) fn comments_tick(&mut self) {
        if self.online.as_ref().is_some_and(|o| o.comments.is_some()) {
            self.comments_step();
        }
    }

    pub(crate) fn comments_step(&mut self) {
        let doc = self.newest_host_doc();
        let events = self
            .online
            .as_ref()
            .and_then(|o| o.comments.as_ref())
            .and_then(|c| c.inbox.lock().ok().map(|mut q| std::mem::take(&mut *q)))
            .unwrap_or_default();
        for event in events {
            match event {
                CommentEvent::Show(threads) => {
                    let Some(comments) = self.online.as_mut().and_then(|o| o.comments.as_mut()) else { return };
                    comments.threads = threads
                        .into_iter()
                        .map(|t| {
                            let anchor = Anchor::parse(&t.anchor).ok();
                            (t, anchor)
                        })
                        .collect();
                    comments.revision = comments.revision.wrapping_add(1);
                }
                CommentEvent::Focus(anchor) => {
                    if let (Some(id), Ok(anchor)) = (doc, Anchor::parse(&anchor)) {
                        self.focus_anchor(id, &anchor);
                    }
                }
                CommentEvent::Resolve { threads, reply } => reply(doc.map(|id| self.resolve_threads(id, &threads)).unwrap_or_default()),
            }
        }
        self.forward_marker_clicks();
        let Some(id) = doc else {
            self.clear_comment_markers(None);
            if let Some(c) = self.online.as_mut().and_then(|o| o.comments.as_mut())
                && let Ok(mut s) = c.selection.lock()
            {
                *s = None;
            }
            return;
        };
        self.refresh_selection_anchor(id);
        self.refresh_comment_markers(id);
    }

    /// What the selection in document `id` is, as far as anchors go: a selected comment, the
    /// field being filled, else selected text.
    fn selected_for_anchor(&self, id: DocId) -> Option<Selected> {
        let view = self.views.iter().find(|v| v.id == id)?;
        if let Some((page, index)) = view.comments.selected {
            return Some(Selected::Annotation { page, index });
        }
        if let Some(f) = view.forms.focus.as_ref() {
            return Some(Selected::Field { name: f.name.clone(), widget: f.widget });
        }
        let info = &self.session.get(id)?.info;
        let (page, quads) = view.selection_quads(info)?;
        Some(Selected::Text { page, quads })
    }

    fn refresh_selection_anchor(&mut self, id: DocId) {
        let generation = self.session.get(id).map(|d| d.edit_generation()).unwrap_or(0);
        let key = Some((id, generation, self.selected_for_anchor(id)));
        if self.online.as_ref().and_then(|o| o.comments.as_ref()).is_none_or(|c| c.selection_key == key) {
            return;
        }
        let anchor = match key.as_ref().and_then(|k| k.2.clone()) {
            Some(Selected::Annotation { page, index }) => {
                let layer = self.views.iter().find(|v| v.id == id).and_then(|v| v.page_text(page));
                self.session.anchor_of_annotation(id, page, index, layer.as_deref())
            }
            Some(Selected::Field { name, widget }) => self.session.anchor_of_field(id, &name, widget),
            Some(Selected::Text { page, quads }) => {
                let text = self.views.iter().find(|v| v.id == id).and_then(|v| v.page_text(page));
                let info = self.session.get(id).and_then(|d| d.info.pages.get(page).cloned());
                let under = text.zip(info).map(|(t, p)| anchors::text_under(&t, &p, &quads)).unwrap_or_default();
                self.session.anchor_of_text(id, page, quads, &under)
            }
            None => None,
        };
        let Some(comments) = self.online.as_mut().and_then(|o| o.comments.as_mut()) else { return };
        comments.selection_key = key;
        if let Ok(mut s) = comments.selection.lock() {
            *s = anchor.as_ref().map(anchor_json);
        }
    }

    /// Place the markers again when the threads or the document changed.
    fn refresh_comment_markers(&mut self, id: DocId) {
        let generation = self.session.get(id).map(|d| d.edit_generation()).unwrap_or(0);
        let Some(comments) = self.online.as_ref().and_then(|o| o.comments.as_ref()) else { return };
        let key = Some((id, generation, comments.revision));
        if comments.markers_key == key {
            return;
        }
        let threads: Vec<(String, u32, Anchor)> =
            comments.threads.iter().filter(|(t, _)| t.status == "open").filter_map(|(t, a)| Some((t.id.clone(), t.count, a.clone()?))).collect();
        let mut markers = Vec::new();
        for (thread, count, anchor) in threads {
            let Some(place) = self.session.anchor_place(id, &anchor) else { continue };
            let rects = place.quads.iter().filter_map(|q| anchors::quads_bounds(std::slice::from_ref(q))).map(|b| b.map(|v| v as f32)).collect();
            markers.push(CommentMarker { thread, page: place.page, rects, count });
        }
        if let Some(c) = self.online.as_mut().and_then(|o| o.comments.as_mut()) {
            c.markers_key = key;
        }
        for view in self.views.iter_mut().filter(|v| v.id == id) {
            view.comment_markers = markers.clone();
        }
        self.clear_comment_markers(Some(id));
    }

    /// Markers belong to the document the threads are about only.
    fn clear_comment_markers(&mut self, keep: Option<DocId>) {
        for view in self.views.iter_mut().filter(|v| Some(v.id) != keep) {
            view.comment_markers.clear();
        }
    }

    fn forward_marker_clicks(&mut self) {
        let clicked: Vec<String> = self.views.iter_mut().filter_map(|v| v.comment_clicked.take()).collect();
        let Some(comments) = self.online.as_ref().and_then(|o| o.comments.as_ref()) else { return };
        for thread in clicked {
            comments.host.comment_clicked(&thread);
        }
    }

    /// Show document `id` at `anchor`: its page scrolled to it, outlined for a moment, its comment
    /// selected when it is one.
    fn focus_anchor(&mut self, id: DocId, anchor: &Anchor) {
        let Some(place) = self.session.anchor_place(id, anchor) else { return };
        let Some(bounds) = anchors::quads_bounds(&place.quads).map(|b| b.map(|v| v as f32)) else { return };
        let selected = match &anchor.region {
            Some(Region::Annot(a)) => self.session.find_annotation(id, a),
            _ => None,
        };
        let Some(info) = self.session.get(id).map(|d| d.info.clone()) else { return };
        if let Some(index) = self.views.iter().position(|v| v.id == id) {
            self.active = Some(index);
        }
        for view in self.views.iter_mut().filter(|v| v.id == id) {
            view.reveal(place.page, bounds, &info);
            if selected.is_some() {
                view.comments.selected = selected;
            }
        }
    }

    /// `resolveAnchors`: each thread's anchor found again in document `id`, in order; an orphaned
    /// one keeps its anchor. A thread whose anchor this editor can't read (a newer editor's) is
    /// left out: nothing here can tell whether it still points somewhere.
    fn resolve_threads(&mut self, id: DocId, threads: &[Thread]) -> Vec<ResolvedThread> {
        let readable: Vec<(&Thread, Anchor)> = threads.iter().filter_map(|t| Some((t, Anchor::parse(&t.anchor).ok()?))).collect();
        let anchors: Vec<Anchor> = readable.iter().map(|(_, a)| a.clone()).collect();
        let resolved = self.session.resolve_anchors(id, &anchors);
        readable
            .into_iter()
            .zip(resolved)
            .map(|((t, _), r)| match r {
                Resolution::Open(anchor) => ResolvedThread { id: t.id.clone(), anchor: anchor_json(&anchor), status: AnchorStatus::Open },
                Resolution::Orphaned => ResolvedThread { id: t.id.clone(), anchor: t.anchor.clone(), status: AnchorStatus::Orphaned },
            })
            .collect()
    }
}
