//! Phone-width layout: below [`NARROW_WIDTH`] the side panels fold away so the page keeps the
//! window, once per crossing, and the user's wide-state choice comes back when it widens again.

use crate::RightPanel;

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

    #[test]
    fn the_breakpoint_itself_is_wide() {
        let mut layout = NarrowLayout::default();
        let mut panels = open();
        layout.update(NARROW_WIDTH, &mut panels);
        assert!(!layout.is_narrow());
        assert_eq!(panels, open());
    }
}
