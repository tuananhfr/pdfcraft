//! Phone-width layout: below [`NARROW_WIDTH`] the side panels fold away and each document fits
//! the page width, once per crossing, and the user's wide-state choices come back when the
//! window widens again.

use pdfcraft_engine::DocId;

use crate::RightPanel;
use crate::canvas::{DocView, Fit};

/// Window content width (points) below which the layout compacts. A tablet in portrait
/// (768+) stays wide.
pub const NARROW_WIDTH: f32 = 600.0;

/// Which side panels are open: the Tools pane on the left and the right (inspector) panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Panels {
    pub left_open: bool,
    pub right: Option<RightPanel>,
}

/// Edge-triggered compaction. Nothing happens while the width stays on one side of the
/// breakpoint, so a panel the user opens on a phone stays open.
#[derive(Clone, Debug, Default)]
pub(crate) struct NarrowLayout {
    narrow: bool,
    /// The wide-state choices folded away on entering narrow. Each is dropped once the user
    /// opens that panel while narrow: from then on their choice is the current one.
    saved_left: Option<bool>,
    saved_right: Option<Option<RightPanel>>,
    /// The documents fitted to the narrow window, each with the zoom it had in the wide one.
    wide_zooms: Vec<WideZoom>,
}

/// A document's zoom in the wide window: a fit mode, or `zoom` with [`Fit::None`].
#[derive(Clone, Copy, Debug)]
struct WideZoom {
    id: DocId,
    fit: Fit,
    zoom: f32,
}

impl NarrowLayout {
    pub fn is_narrow(&self) -> bool {
        self.narrow
    }

    /// Follow the window's content width; compacts or restores `panels` only on a crossing.
    pub fn update(&mut self, width: f32, panels: &mut Panels) {
        // A window not laid out yet (zero size) says nothing about the device.
        if !(width.is_finite() && width > 0.0) {
            return;
        }
        let narrow = width < NARROW_WIDTH;
        match (self.narrow, narrow) {
            (false, true) => {
                self.saved_left = Some(panels.left_open);
                self.saved_right = Some(panels.right);
                *panels = Panels { left_open: false, right: None };
            }
            (true, true) => {
                if panels.left_open {
                    self.saved_left = None;
                }
                if panels.right.is_some() {
                    self.saved_right = None;
                }
            }
            (true, false) => {
                *panels = self.wide_choice(*panels);
                self.saved_left = None;
                self.saved_right = None;
            }
            (false, false) => {}
        }
        self.narrow = narrow;
    }

    /// Fit each document to the page width once per narrow spell (those opened while narrow
    /// too), so a zoom the user picks afterwards stands; once the window is wide again, give
    /// every document the zoom it had there. Call after [`Self::update`].
    pub fn fit_documents(&mut self, views: &mut [DocView]) {
        if self.narrow {
            self.wide_zooms.retain(|w| views.iter().any(|v| v.id == w.id));
            for view in views.iter_mut() {
                if !self.wide_zooms.iter().any(|w| w.id == view.id) {
                    self.wide_zooms.push(WideZoom { id: view.id, fit: view.fit, zoom: view.zoom });
                    // `set_fit` scrolls to the page's top; a view already fitting stays put.
                    if view.fit != Fit::Width {
                        view.set_fit(Fit::Width);
                    }
                }
            }
        } else {
            for wide in self.wide_zooms.drain(..) {
                let Some(view) = views.iter_mut().find(|v| v.id == wide.id) else { continue };
                if wide.fit == Fit::None {
                    view.set_zoom(wide.zoom);
                } else if view.fit != wide.fit {
                    view.set_fit(wide.fit);
                }
            }
        }
    }

    /// The panels as the user last chose them at a wide width: what settings persist, so a
    /// phone-width session doesn't overwrite the desktop layout.
    pub fn wide_choice(&self, current: Panels) -> Panels {
        Panels { left_open: self.saved_left.unwrap_or(current.left_open), right: self.saved_right.unwrap_or(current.right) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: f32 = 1366.0;
    const PHONE: f32 = 390.0;

    fn open() -> Panels {
        Panels { left_open: true, right: Some(RightPanel::Bookmarks) }
    }

    #[test]
    fn wide_to_narrow_compacts_once() {
        let mut layout = NarrowLayout::default();
        let mut panels = open();
        layout.update(WIDE, &mut panels);
        assert_eq!(panels, open(), "wide leaves the panels alone");
        layout.update(PHONE, &mut panels);
        assert!(layout.is_narrow());
        assert_eq!(panels, Panels { left_open: false, right: None });
        assert_eq!(layout.wide_choice(panels), open(), "the wide choice is what persists");
    }

    #[test]
    fn a_panel_opened_while_narrow_stays_open() {
        let mut layout = NarrowLayout::default();
        let mut panels = open();
        layout.update(WIDE, &mut panels);
        layout.update(PHONE, &mut panels);
        panels.right = Some(RightPanel::Comments);
        for width in [PHONE, 420.0, 599.0] {
            layout.update(width, &mut panels);
            assert_eq!(panels.right, Some(RightPanel::Comments), "never fought while the width stays narrow ({width})");
        }
        layout.update(WIDE, &mut panels);
        assert_eq!(panels, Panels { left_open: true, right: Some(RightPanel::Comments) }, "the user's narrow choice holds; Tools comes back");
    }

    #[test]
    fn narrow_to_wide_restores_the_wide_choice() {
        let mut layout = NarrowLayout::default();
        let mut panels = Panels { left_open: false, right: Some(RightPanel::Pages) };
        layout.update(WIDE, &mut panels);
        layout.update(PHONE, &mut panels);
        assert_eq!(panels, Panels { left_open: false, right: None });
        layout.update(WIDE, &mut panels);
        assert_eq!(panels, Panels { left_open: false, right: Some(RightPanel::Pages) });
        // A second crossing compacts again.
        layout.update(PHONE, &mut panels);
        assert_eq!(panels.right, None);
    }

    #[test]
    fn opened_then_closed_while_narrow_stays_closed_when_wide() {
        let mut layout = NarrowLayout::default();
        let mut panels = open();
        layout.update(WIDE, &mut panels);
        layout.update(PHONE, &mut panels);
        panels.right = Some(RightPanel::Search);
        layout.update(PHONE, &mut panels);
        panels.right = None;
        layout.update(PHONE, &mut panels);
        layout.update(WIDE, &mut panels);
        assert_eq!(panels.right, None, "closing it on the phone was the user's latest choice");
        assert!(panels.left_open);
    }

    #[test]
    fn launching_narrow_compacts_and_an_unsized_window_is_ignored() {
        let mut layout = NarrowLayout::default();
        let mut panels = open();
        layout.update(0.0, &mut panels);
        layout.update(f32::NAN, &mut panels);
        assert_eq!(panels, open());
        layout.update(PHONE, &mut panels);
        assert_eq!(panels, Panels { left_open: false, right: None });
        assert_eq!(layout.wide_choice(panels), open());
    }

    fn document(id: u64) -> DocView {
        let page = pdfcraft_render::PageInfo { width: 612.0, height: 792.0, label: String::new(), crop: [0.0, 0.0, 612.0, 792.0], rotation: 0 };
        let info = pdfcraft_render::DocInfo { pages: vec![page; 3], ..Default::default() };
        DocView::new(DocId(id), &info, crate::canvas::ViewDefaults::default())
    }

    fn zoom_of(view: &DocView) -> (Fit, f32) {
        (view.fit, if view.fit == Fit::None { view.zoom } else { 0.0 })
    }

    /// Crosses to `width` the way a frame does.
    fn resize(layout: &mut NarrowLayout, width: f32, views: &mut [DocView]) {
        let mut panels = Panels { left_open: false, right: None };
        layout.update(width, &mut panels);
        layout.fit_documents(views);
    }

    #[test]
    fn narrowing_fits_every_document_to_the_width_and_widening_gives_each_zoom_back() {
        let mut layout = NarrowLayout::default();
        let mut views = vec![document(1), document(2), document(3)];
        views[0].set_zoom(1.25);
        views[1].set_fit(Fit::Page);
        views[2].set_fit(Fit::Height);
        resize(&mut layout, WIDE, &mut views);
        let wide: Vec<_> = views.iter().map(zoom_of).collect();
        assert_eq!(wide, [(Fit::None, 1.25), (Fit::Page, 0.0), (Fit::Height, 0.0)], "a wide window changes nothing");
        resize(&mut layout, PHONE, &mut views);
        assert!(views.iter().all(|v| v.fit == Fit::Width), "every open document fits the width");
        resize(&mut layout, WIDE, &mut views);
        assert_eq!(views.iter().map(zoom_of).collect::<Vec<_>>(), wide, "each gets its own zoom back");
        // The next narrow spell fits them again.
        resize(&mut layout, PHONE, &mut views);
        assert!(views.iter().all(|v| v.fit == Fit::Width));
    }

    #[test]
    fn a_zoom_picked_while_narrow_stands_until_the_window_widens() {
        let mut layout = NarrowLayout::default();
        let mut views = vec![document(1), document(2)];
        views[0].set_zoom(0.8);
        resize(&mut layout, WIDE, &mut views);
        resize(&mut layout, PHONE, &mut views);
        views[0].set_zoom(2.0);
        views[1].set_fit(Fit::Page);
        for width in [PHONE, 420.0, 599.0] {
            resize(&mut layout, width, &mut views);
            assert_eq!(zoom_of(&views[0]), (Fit::None, 2.0), "never fought while narrow ({width})");
            assert_eq!(views[1].fit, Fit::Page);
        }
        resize(&mut layout, WIDE, &mut views);
        assert_eq!(zoom_of(&views[0]), (Fit::None, 0.8), "widening restores the wide zoom");
        assert_eq!(views[1].fit, crate::canvas::ViewDefaults::default().fit);
    }

    #[test]
    fn a_document_opened_while_narrow_fits_and_widens_to_its_opening_zoom() {
        let mut layout = NarrowLayout::default();
        let mut views = vec![document(1)];
        resize(&mut layout, PHONE, &mut views);
        let mut opened = document(2);
        opened.set_zoom(1.5);
        views.push(opened);
        resize(&mut layout, PHONE, &mut views);
        assert_eq!(views[1].fit, Fit::Width);
        // A closed document is forgotten; the ids left are still restored.
        views.remove(0);
        resize(&mut layout, PHONE, &mut views);
        assert_eq!(layout.wide_zooms.len(), 1);
        resize(&mut layout, WIDE, &mut views);
        assert_eq!(zoom_of(&views[0]), (Fit::None, 1.5));
        assert!(layout.wide_zooms.is_empty());
    }

    #[test]
    fn the_breakpoint_itself_is_wide() {
        let mut layout = NarrowLayout::default();
        let mut panels = open();
        layout.update(NARROW_WIDTH, &mut panels);
        assert!(!layout.is_narrow());
        assert_eq!(panels, open());
    }
}
