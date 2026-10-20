//! How the tab strip and the mode bar shed controls when the window is too narrow for them (a
//! phone, or a tablet in a long language): from measured widths, the lowest-priority control
//! gives way first, so nothing overlaps or falls off the edge.

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
    /// The search box at its narrowest, before it becomes an icon.
    pub search_short: f32,
    pub search_icon: f32,
    /// One icon button (Print, Save, Properties) with its gap.
    pub icon: f32,
}

/// What the mode bar shows. [`ModeBarFit::FULL`] is the desktop bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModeBarFit {
    /// The search box shrinks towards its narrowest: see [`ModeBarFit::search_box_width`].
    pub search_short: bool,
    pub menu_icon: bool,
    pub search_icon: bool,
    pub mode_picker: bool,
    /// Print and Properties are also in Menu ▸ File; Save stays.
    pub print: bool,
    pub properties: bool,
}

impl ModeBarFit {
    pub const FULL: Self = Self { search_short: false, menu_icon: false, search_icon: false, mode_picker: false, print: true, properties: true };

    pub fn width(&self, w: &ModeBarWidths) -> f32 {
        let menu = if self.menu_icon { w.menu_icon } else { w.menu };
        let tabs = if self.mode_picker { w.picker } else { w.tabs };
        let search = match (self.search_icon, self.search_short) {
            (true, _) => w.search_icon,
            (false, true) => w.search_short,
            (false, false) => w.search,
        };
        let icons = 1 + usize::from(self.print) + usize::from(self.properties);
        menu + w.divider + tabs + search + icons as f32 * w.icon
    }

    /// How much wider than its narrowest a shrunk search box can be in `available`: the box
    /// takes the room the rest of the bar leaves, up to its full width.
    pub fn search_box_growth(&self, available: f32, w: &ModeBarWidths) -> f32 {
        if self.search_short && !self.search_icon { (available - self.width(w)).clamp(0.0, (w.search - w.search_short).max(0.0)) } else { 0.0 }
    }
}

/// The fullest mode bar that fits in `available`; past the last step it shows what it can.
pub(crate) fn fit_mode_bar(available: f32, w: &ModeBarWidths) -> ModeBarFit {
    let steps: [fn(&mut ModeBarFit); 6] = [
        |f| f.search_short = true,
        |f| f.search_icon = true,
        |f| f.menu_icon = true,
        |f| f.mode_picker = true,
        |f| f.print = false,
        |f| f.properties = false,
    ];
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
        ModeBarWidths {
            menu: 75.0,
            menu_icon: 34.0,
            divider: 18.0,
            tabs: 440.0,
            picker: 140.0,
            search: 268.0,
            search_short: 168.0,
            search_icon: 36.0,
            icon: 36.0,
        }
    }

    #[test]
    fn a_wide_bar_keeps_everything() {
        assert_eq!(fit_mode_bar(1346.0, &widths()), ModeBarFit::FULL);
    }

    #[test]
    fn controls_give_way_in_priority_order() {
        let w = widths();
        let short = ModeBarFit { search_short: true, ..ModeBarFit::FULL };
        assert_eq!(fit_mode_bar(short.width(&w), &w), short, "the search box gives way first");
        let search_only = ModeBarFit { search_short: true, search_icon: true, ..ModeBarFit::FULL };
        assert_eq!(fit_mode_bar(search_only.width(&w), &w), search_only);
        // A tablet in portrait (768 wide) keeps the menu and every mode.
        let tablet = fit_mode_bar(748.0, &w);
        assert!(!tablet.menu_icon && !tablet.mode_picker && tablet.print && tablet.properties, "{tablet:?}");
        assert!(tablet.width(&w) <= 748.0);
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
    fn a_shrunk_search_box_takes_the_room_left() {
        let w = widths();
        let full = ModeBarFit::FULL.width(&w);
        let fit = fit_mode_bar(full - 40.0, &w);
        assert!(fit.search_short && !fit.search_icon);
        assert_eq!(fit.search_box_growth(full - 40.0, &w), 60.0, "40 short of full: 100 - 40 above the narrowest");
        assert_eq!(fit.width(&w) + fit.search_box_growth(full - 40.0, &w), full - 40.0, "it fills the bar exactly");
        assert_eq!(ModeBarFit::FULL.search_box_growth(full, &w), 0.0);
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
