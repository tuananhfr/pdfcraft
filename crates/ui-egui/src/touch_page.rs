//! Touch on the page with the Select tool: a one-finger drag pans as the Hand tool does, a long
//! press selects the word under the finger and dragging on extends it, and in single-page view
//! a swipe turns the page. Mouse and pen input never enter these states.

use egui::{Pos2, Vec2};

/// The finger gesture in progress on the page, decided once per press.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum TouchGesture {
    #[default]
    None,
    /// A press no finger has been reported for: a mouse or pen, or a touch egui reports a
    /// frame late.
    Pressed { origin: Pos2 },
    /// A finger is down and has neither moved nor been held long enough to decide.
    Pending { origin: Pos2 },
    /// `last` is where the finger was last seen: the release itself reports no position.
    Pan { origin: Pos2, last: Pos2 },
    /// A long press. `word` (page, first glyph, last glyph) is the word it selected, which a
    /// drag extends; `None` off text, where the long press opened the context menu instead.
    Held { word: Option<(usize, usize, usize)> },
}

/// The pointer facts a gesture is decided from, read once per frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TouchInput {
    /// Where the primary button went down this frame.
    pub pressed_at: Option<Pos2>,
    /// A finger is on the screen: eframe sends `Event::Touch` along with the pointer events.
    pub touching: bool,
    /// The press is still down, or was released this frame.
    pub active: bool,
    /// The pointer has gone further from where it was pressed than a click may.
    pub moved: bool,
    pub pos: Option<Pos2>,
}

impl TouchInput {
    /// `max_click_dist` is egui's `InputOptions::max_click_dist`.
    pub fn read(i: &egui::InputState, max_click_dist: f32) -> Self {
        let p = &i.pointer;
        let moved = match (p.press_origin(), p.latest_pos()) {
            (Some(origin), Some(now)) => origin.distance(now) > max_click_dist,
            _ => false,
        };
        Self {
            pressed_at: if p.primary_pressed() { p.press_origin() } else { None },
            touching: i.any_touches(),
            active: p.any_down() || p.any_released(),
            moved,
            pos: p.latest_pos(),
        }
    }
}

impl TouchGesture {
    /// Follow this frame's pointer. The gesture keeps its state on the frame the finger lifts,
    /// so the release can still be read as a swipe.
    pub fn update(&mut self, input: TouchInput) {
        if let Some(origin) = input.pressed_at {
            *self = if input.touching { Self::Pending { origin } } else { Self::Pressed { origin } };
            return;
        }
        if !input.active {
            *self = Self::None;
            return;
        }
        if let Self::Pressed { origin } = *self
            && input.touching
        {
            *self = Self::Pending { origin };
        }
        if let Self::Pending { origin } = *self
            && input.moved
        {
            *self = Self::Pan { origin, last: input.pos.unwrap_or(origin) };
        } else if let Self::Pan { last, .. } = self
            && let Some(pos) = input.pos
        {
            *last = pos;
        }
    }

    /// egui reported a long press (held without moving) on the page.
    pub fn long_pressed(&mut self, word: Option<(usize, usize, usize)>) {
        if matches!(self, Self::Pending { .. } | Self::Pan { .. }) {
            *self = Self::Held { word };
        }
    }

    /// The long press landed on a word, which a drag now extends.
    pub fn hold_word(&mut self, word: (usize, usize, usize)) {
        if self.is_touch() {
            *self = Self::Held { word: Some(word) };
        }
    }

    pub fn is_touch(&self) -> bool {
        matches!(self, Self::Pending { .. } | Self::Pan { .. } | Self::Held { .. })
    }

    /// Where the finger went down, while it pans.
    pub fn pan_origin(&self) -> Option<Pos2> {
        match self {
            Self::Pan { origin, .. } => Some(*origin),
            _ => None,
        }
    }

    /// How far the panning finger has gone since it went down.
    pub fn pan_delta(&self) -> Option<Vec2> {
        match self {
            Self::Pan { origin, last } => Some(*last - *origin),
            _ => None,
        }
    }

    /// The word a long press selected, which a drag extends.
    pub fn held_word(&self) -> Option<(usize, usize, usize)> {
        match self {
            Self::Held { word } => *word,
            _ => None,
        }
    }
}

/// The selection from a long-pressed word (`first..=last`) to the glyph under the finger:
/// (anchor, head), so the whole word stays selected whichever way the finger goes.
pub(crate) fn extend_word(first: usize, last: usize, head: usize) -> (usize, usize) {
    if head < first { (last, head) } else { (first, head.max(last)) }
}

/// The shortest swipe that turns a page, as a share of the page view's smaller side.
const SWIPE_SHARE: f32 = 0.15;

/// Whether a finished pan of `delta` turns the page in single-page view: `Some(true)` for the
/// next page (finger moved left or up). Only along an axis the page doesn't pan in, so
/// panning a zoomed page never turns it.
pub(crate) fn swipe_turn(delta: Vec2, view: Vec2, pans_x: bool, pans_y: bool) -> Option<bool> {
    let min = SWIPE_SHARE * view.x.min(view.y);
    let (along, pans) = if delta.x.abs() >= delta.y.abs() { (delta.x, pans_x) } else { (delta.y, pans_y) };
    (!pans && along.abs() >= min && min > 0.0).then_some(along < 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn press(at: Pos2, touching: bool) -> TouchInput {
        TouchInput { pressed_at: Some(at), touching, active: true, moved: false, pos: Some(at) }
    }

    const HELD: TouchInput = TouchInput { pressed_at: None, touching: true, active: true, moved: false, pos: None };
    const MOVED: TouchInput = TouchInput { pressed_at: None, touching: true, active: true, moved: true, pos: Some(pos2(40.0, 10.0)) };
    const LIFTED: TouchInput = TouchInput { pressed_at: None, touching: false, active: true, moved: true, pos: None };
    const IDLE: TouchInput = TouchInput { pressed_at: None, touching: false, active: false, moved: false, pos: None };

    #[test]
    fn a_finger_that_moves_pans_until_the_frame_after_it_lifts() {
        let mut g = TouchGesture::default();
        g.update(press(pos2(10.0, 10.0), true));
        assert_eq!(g, TouchGesture::Pending { origin: pos2(10.0, 10.0) });
        g.update(HELD);
        assert_eq!(g.pan_origin(), None, "a still finger may become a tap or a long press");
        g.update(MOVED);
        assert_eq!(g.pan_delta(), Some(vec2(30.0, 0.0)));
        g.update(TouchInput { pos: Some(pos2(-50.0, 12.0)), ..MOVED });
        g.update(LIFTED);
        assert!(g.pan_origin().is_some(), "the release frame still reads as the pan, for swipes");
        assert_eq!(g.pan_delta(), Some(vec2(-60.0, 2.0)), "the last position seen, not the release's");
        g.update(IDLE);
        assert_eq!(g, TouchGesture::None);
    }

    #[test]
    fn mouse_presses_never_become_touch_gestures() {
        let mut g = TouchGesture::default();
        g.update(press(pos2(1.0, 1.0), false));
        g.update(TouchInput { touching: false, ..MOVED });
        assert!(!g.is_touch());
        assert_eq!(g.pan_origin(), None);
        g.long_pressed(Some((0, 1, 2)));
        assert!(!g.is_touch());
        g.update(IDLE);
        assert_eq!(g, TouchGesture::None);
    }

    #[test]
    fn a_finger_reported_a_frame_after_its_press_still_pans() {
        let mut g = TouchGesture::default();
        g.update(press(pos2(10.0, 10.0), false));
        g.update(MOVED);
        assert_eq!(g.pan_delta(), Some(vec2(30.0, 0.0)));
    }

    #[test]
    fn a_long_press_holds_and_moving_on_does_not_pan() {
        let mut g = TouchGesture::default();
        g.update(press(pos2(0.0, 0.0), true));
        g.update(HELD);
        g.long_pressed(Some((2, 5, 9)));
        g.update(MOVED);
        assert_eq!(g.held_word(), Some((2, 5, 9)));
        assert_eq!(g.pan_origin(), None);
        // A new press starts over.
        g.update(press(pos2(3.0, 3.0), true));
        assert_eq!(g, TouchGesture::Pending { origin: pos2(3.0, 3.0) });
    }

    #[test]
    fn extending_a_word_keeps_it_whole_both_ways() {
        assert_eq!(extend_word(5, 9, 20), (5, 20));
        assert_eq!(extend_word(5, 9, 7), (5, 9));
        assert_eq!(extend_word(5, 9, 1), (9, 1));
    }

    #[test]
    fn swipes_turn_only_along_an_axis_that_does_not_pan() {
        let view = vec2(390.0, 700.0);
        assert_eq!(swipe_turn(vec2(-120.0, 10.0), view, false, true), Some(true), "left: next page");
        assert_eq!(swipe_turn(vec2(120.0, -10.0), view, false, true), Some(false), "right: previous page");
        assert_eq!(swipe_turn(vec2(-120.0, 0.0), view, true, true), None, "a zoomed page pans instead");
        assert_eq!(swipe_turn(vec2(-20.0, 0.0), view, false, false), None, "too short");
        assert_eq!(swipe_turn(vec2(5.0, -200.0), view, false, false), Some(true), "up on a page that fits: next");
        assert_eq!(swipe_turn(vec2(5.0, -200.0), view, false, true), None, "a tall page scrolls");
        assert_eq!(swipe_turn(vec2(-200.0, 0.0), Vec2::ZERO, false, false), None);
    }
}
