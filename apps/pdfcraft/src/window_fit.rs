//! The window opens maximized, and Restore gives a window that fits the screen.
//!
//! eframe only clamps the start size to the largest monitor's full size, ignoring the taskbar and
//! the title bar, so on a 1366×768 screen or at 150 % scaling the default window ran off the screen.
//! eframe also saves the size the window has when the app closes, so a window closed maximized came
//! back with a whole-screen restore size. egui knows the monitor only once the window exists, so the
//! restore size is set on the first frame (before eframe shows the window), then the window is
//! maximized. A window the user left un-maximized is restored as it was.

use egui::{Pos2, Vec2, ViewportCommand, ViewportInfo, vec2};

pub const DEFAULT_INNER_SIZE: Vec2 = vec2(1440.0, 920.0);
pub const MIN_INNER_SIZE: Vec2 = vec2(820.0, 520.0);
/// Kept free at the bottom of the screen: Windows 11's taskbar height. egui can't see the real one.
const TASKBAR: f32 = 48.0;
/// Kept free between the restored window and the screen edges.
const GAP: f32 = 16.0;
/// Title bar and borders of an un-maximized window, for when they can't be measured: a maximized
/// window reports different ones.
const FALLBACK_FRAME: Vec2 = vec2(16.0, 40.0);

/// Outer position and inner size of the restored window on a `monitor` whose top-left corner is
/// `origin`. `frame` is the outer size minus the inner size.
pub fn restored_rect(origin: Pos2, monitor: Vec2, frame: Vec2) -> (Pos2, Vec2) {
    let usable = vec2(monitor.x, monitor.y - TASKBAR);
    let inner = (usable - frame - Vec2::splat(2.0 * GAP)).min(DEFAULT_INNER_SIZE).max(MIN_INNER_SIZE);
    let outer = inner + frame;
    (origin + ((usable - outer) / 2.0).max(Vec2::ZERO), inner)
}

#[derive(Debug, PartialEq)]
enum Decision {
    /// The monitor isn't known yet.
    Wait,
    Keep,
    Fit(Pos2, Vec2),
}

/// A window restored un-maximized keeps its saved geometry. Otherwise it is fitted on its monitor:
/// a maximized window's client area starts at the monitor's left edge, and its frame sticks out by
/// the same border on every side, which gives the monitor's top edge. A new window is on the primary
/// monitor, at the origin.
fn decide(viewport: &ViewportInfo, saved_window: bool) -> Decision {
    let Some(monitor) = viewport.monitor_size else {
        return Decision::Wait;
    };
    let maximized = viewport.maximized == Some(true);
    if saved_window && !maximized {
        return Decision::Keep;
    }
    let (origin, frame) = match (viewport.outer_rect, viewport.inner_rect) {
        (Some(outer), Some(inner)) if maximized => (Pos2::new(inner.min.x, outer.min.y + (inner.min.x - outer.min.x)), FALLBACK_FRAME),
        (Some(outer), Some(inner)) => (Pos2::ZERO, outer.size() - inner.size()),
        _ => (Pos2::ZERO, FALLBACK_FRAME),
    };
    let (position, size) = restored_rect(origin, monitor, frame);
    Decision::Fit(position, size)
}

pub struct WindowFit {
    pending: bool,
    saved_window: bool,
}

impl WindowFit {
    /// `saved_window`: eframe restored a window geometry saved by an earlier run.
    pub fn new(saved_window: bool) -> Self {
        Self { pending: true, saved_window }
    }

    /// Sets the restore rect and maximizes, once.
    pub fn apply(&mut self, ctx: &egui::Context) {
        if !self.pending {
            return;
        }
        match ctx.input(|input| decide(input.viewport(), self.saved_window)) {
            Decision::Wait => {}
            Decision::Keep => self.pending = false,
            Decision::Fit(position, size) => {
                self.pending = false;
                // On a maximized window winit's InnerSize ends with SW_RESTORE, which brings back the
                // saved whole-screen size: un-maximize first. A no-op for a window that isn't maximized.
                ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2};

    const FRAME: Vec2 = vec2(16.0, 39.0);

    fn fits(monitor: Vec2) {
        let (position, inner) = restored_rect(Pos2::ZERO, monitor, FRAME);
        let outer_max = position + inner + FRAME;
        assert!(position.x >= 0.0 && position.y >= 0.0, "{monitor:?}: {position:?}");
        assert!(outer_max.x <= monitor.x && outer_max.y <= monitor.y - TASKBAR, "{monitor:?}: ends at {outer_max:?}");
        assert!(inner.x <= DEFAULT_INNER_SIZE.x && inner.y <= DEFAULT_INNER_SIZE.y);
    }

    fn viewport(outer: Rect, inner: Rect, maximized: bool) -> ViewportInfo {
        ViewportInfo {
            monitor_size: Some(vec2(1920.0, 1080.0)),
            outer_rect: Some(outer),
            inner_rect: Some(inner),
            maximized: Some(maximized),
            ..Default::default()
        }
    }

    #[test]
    fn restored_window_fits_small_and_scaled_screens() {
        // 1920×1080 at 150 %, 1366×768 at 100 %, 1920×1080 at 125 %.
        for monitor in [vec2(1280.0, 720.0), vec2(1366.0, 768.0), vec2(1536.0, 864.0)] {
            fits(monitor);
        }
        assert_eq!(restored_rect(Pos2::ZERO, vec2(1280.0, 720.0), FRAME), (pos2(16.0, 16.0), vec2(1232.0, 601.0)));
    }

    #[test]
    fn large_screen_keeps_the_default_size_centred() {
        fits(vec2(1920.0, 1080.0));
        let (position, inner) = restored_rect(Pos2::ZERO, vec2(1920.0, 1080.0), FRAME);
        assert_eq!(inner, DEFAULT_INNER_SIZE);
        assert_eq!(position, pos2((1920.0 - 1456.0) / 2.0, (1032.0 - 959.0) / 2.0));
    }

    #[test]
    fn tiny_screen_never_goes_below_the_minimum_size() {
        let (position, inner) = restored_rect(Pos2::ZERO, vec2(600.0, 400.0), FRAME);
        assert_eq!(inner, MIN_INNER_SIZE);
        assert_eq!(position, Pos2::ZERO);
    }

    #[test]
    fn saved_un_maximized_window_is_kept() {
        let outer = Rect::from_min_size(pos2(100.0, 80.0), vec2(1016.0, 739.0));
        let inner = Rect::from_min_size(pos2(108.0, 111.0), vec2(1000.0, 700.0));
        assert_eq!(decide(&viewport(outer, inner, false), true), Decision::Keep);
        assert!(matches!(decide(&viewport(outer, inner, false), false), Decision::Fit(..)));
        assert_eq!(decide(&ViewportInfo::default(), false), Decision::Wait);
    }

    #[test]
    fn saved_maximized_window_is_fitted_on_its_own_monitor() {
        // Maximized on a second 1920×1080 monitor to the right of the primary one.
        let outer = Rect::from_min_max(pos2(1912.0, -8.0), pos2(3848.0, 1040.0));
        let inner = Rect::from_min_max(pos2(1920.0, 23.0), pos2(3840.0, 1032.0));
        assert_eq!(
            decide(&viewport(outer, inner, true), true),
            Decision::Fit(pos2(1920.0 + (1920.0 - 1456.0) / 2.0, (1032.0 - 960.0) / 2.0), DEFAULT_INNER_SIZE)
        );
    }
}
