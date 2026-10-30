//! `doc.preserveView` of `erpHost` v2: a newer version of a document already open takes its tab's
//! place, and the reader stays on the page they were reading at the zoom they had, as far as the
//! new version still has that page.

use pdfcraft_engine::DocId;

use crate::PdfCraftApp;
use crate::canvas::Fit;

impl PdfCraftApp {
    /// Give the view of document `to` the page, zoom and page display of the view of `from`.
    pub(super) fn keep_view(&mut self, from: DocId, to: DocId) {
        let Some(old) = self.views.iter().find(|v| v.id == from) else { return };
        let (page, fit, zoom) = (old.current, old.fit, old.zoom);
        let (layout, continuous, rotation, cover) = (old.layout, old.continuous, old.rotation, old.cover);
        let Some(view) = self.views.iter_mut().find(|v| v.id == to) else { return };
        view.layout = layout;
        view.continuous = continuous;
        view.rotation = rotation;
        view.cover = cover;
        if fit == Fit::None {
            view.set_zoom(zoom);
        } else {
            view.set_fit(fit);
        }
        // Clamped to the new version's pages.
        view.go_to_page(page);
        // Coming from the first page was not the reader's own move.
        view.back.clear();
        // A phone keeps the zoom the reader picked instead of fitting the width again.
        self.narrow.adopt(from, to);
    }
}
