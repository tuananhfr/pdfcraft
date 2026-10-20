//! How the tab strip and the mode bar shed controls at phone width: from measured widths, the
//! lowest-priority control gives way first, so nothing overlaps or falls off the edge.

/// Measured widths (points) of the mode bar's parts, each including its own spacing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ModeBarWidths {
    pub menu: f32,
    pub menu_icon: f32,
    /// The divider after the menu, with its margins.
    pub divider: f32,
    pub tabs: f32,
    /// The current mode alone, as a button that opens the list of modes.
    pub picker: f32,
    pub search: f32,
    pub search_icon: f32,
    /// One icon button (Print, Save, Properties) with its gap.
    pub icon: f32,
}

/// What the mode bar shows. [`ModeBarFit::FULL`] is the desktop bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModeBarFit {
    pub menu_icon: bool,
    pub search_icon: bool,
    pub mode_picker: bool,
    /// Print and Properties are also in Menu ▸ File; Save stays.
    pub print: bool,
    pub properties: bool,
}

impl ModeBarFit {
    pub const FULL: Self = Self { menu_icon: false, search_icon: false, mode_picker: false, print: true, properties: true };

    pub fn width(&self, w: &ModeBarWidths) -> f32 {
        let menu = if self.menu_icon { w.menu_icon } else { w.menu };
        let tabs = if self.mode_picker { w.picker } else { w.tabs };
        let search = if self.search_icon { w.search_icon } else { w.search };
        let icons = 1 + usize::from(self.print) + usize::from(self.properties);
        menu + w.divider + tabs + search + icons as f32 * w.icon
    }
}

/// The fullest mode bar that fits in `available`; past the last step it shows what it can.
pub(crate) fn fit_mode_bar(available: f32, w: &ModeBarWidths) -> ModeBarFit {
    let steps: [fn(&mut ModeBarFit); 5] =
        [|f| f.search_icon = true, |f| f.menu_icon = true, |f| f.mode_picker = true, |f| f.print = false, |f| f.properties = false];
    let mut fit = ModeBarFit::FULL;
    for step in steps {
        if fit.width(w) <= available {
            break;
        }
        step(&mut fit);
    }
    fit
}

/// What the tab strip shows to the right of the document tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StripFit {
    pub open_icon: bool,
    /// The theme button is also in Menu ▸ View ▸ Display theme.
    pub theme: bool,
    /// Keyboard shortcuts are also in Menu ▸ Help and on F1.
    pub help: bool,
}

impl StripFit {
    pub const FULL: Self = Self { open_icon: false, theme: true, help: true };
}

/// The fullest strip whose controls leave the tabs `tabs_min` points in `available`.
/// `open` and `open_icon` are the Open button's widths with its gaps; `button` is one icon
/// button with its gap.
pub(crate) fn fit_tab_strip(available: f32, tabs_min: f32, open: f32, open_icon: f32, button: f32) -> StripFit {
    let steps: [fn(&mut StripFit); 3] = [|f| f.open_icon = true, |f| f.theme = false, |f| f.help = false];
    let width = |f: &StripFit| {
        let buttons = usize::from(f.theme) + usize::from(f.help);
        tabs_min + if f.open_icon { open_icon } else { open } + buttons as f32 * button
    };
    let mut fit = StripFit::FULL;
    for step in steps {
        if width(&fit) <= available {
            break;
        }
        step(&mut fit);
    }
    fit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths() -> ModeBarWidths {
        ModeBarWidths { menu: 75.0, menu_icon: 34.0, divider: 18.0, tabs: 440.0, picker: 140.0, search: 268.0, search_icon: 36.0, icon: 36.0 }
    }

    #[test]
    fn a_wide_bar_keeps_everything() {
        assert_eq!(fit_mode_bar(1346.0, &widths()), ModeBarFit::FULL);
    }

    #[test]
    fn controls_give_way_in_priority_order() {
        let w = widths();
        let search_only = ModeBarFit { search_icon: true, ..ModeBarFit::FULL };
        assert_eq!(fit_mode_bar(search_only.width(&w), &w), search_only);
        // A phone: the modes fold into a picker, Save and the search stay.
        let phone = fit_mode_bar(370.0, &w);
        assert!(phone.search_icon && phone.menu_icon && phone.mode_picker, "{phone:?}");
        assert!(phone.width(&w) <= 370.0);
        // Narrower still: Print goes before Properties.
        let tight = fit_mode_bar(300.0, &w);
        assert!(!tight.print && tight.properties, "{tight:?}");
        assert!(tight.width(&w) <= 300.0);
    }

    #[test]
    fn nothing_fits_still_keeps_the_essentials() {
        let tiny = fit_mode_bar(10.0, &widths());
        assert!(tiny.menu_icon && tiny.mode_picker && tiny.search_icon && !tiny.print && !tiny.properties);
    }

    #[test]
    fn the_strip_sheds_open_text_then_theme_then_help() {
        assert_eq!(fit_tab_strip(340.0, 180.0, 72.0, 40.0, 32.0), StripFit::FULL);
        assert_eq!(fit_tab_strip(300.0, 180.0, 72.0, 40.0, 32.0), StripFit { open_icon: true, ..StripFit::FULL });
        assert_eq!(fit_tab_strip(80.0, 0.0, 72.0, 40.0, 32.0), StripFit { open_icon: true, theme: false, help: true });
        assert_eq!(fit_tab_strip(45.0, 0.0, 72.0, 40.0, 32.0), StripFit { open_icon: true, theme: false, help: false });
    }
}
