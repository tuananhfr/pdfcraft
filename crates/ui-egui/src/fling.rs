//! Scroll inertia for a one-finger pan of the page: a quick release keeps the page moving and
//! slows it to a stop, as touch screens do. The model and its constants match GridCraft's, so
//! the editors feel the same under a finger.

use egui::Vec2;

/// Fling speeds, in points per second.
const FLING_MIN_SPEED: f32 = 250.0;
const FLING_STOP_SPEED: f32 = 20.0;
/// Fraction of the fling speed lost per second.
const FLING_DECAY: f32 = 4.0;

/// The page's motion after the finger lifted: its velocity in screen points per second, in the
/// direction the finger moved (the sign `Ui::scroll_with_delta` takes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Fling {
    velocity: Option<Vec2>,
    /// Two fingers zoomed during this gesture: the finger left on the page lifts at the
    /// pinch's speed, which isn't a pan.
    pinched: bool,
}

impl Fling {
    /// Starts a fling from the finger's velocity as it lifted, if it was quick enough.
    pub fn release(&mut self, velocity: Vec2) {
        let quick = velocity.is_finite() && velocity.length() >= FLING_MIN_SPEED;
        self.velocity = (quick && !self.pinched).then_some(velocity);
    }

    /// A new press: it stops the page and starts a gesture that may fling again.
    pub fn press(&mut self) {
        *self = Self::default();
    }

    pub fn pinch(&mut self) {
        *self = Self { velocity: None, pinched: true };
    }

    pub fn stop(&mut self) {
        self.velocity = None;
    }

    pub fn flinging(&self) -> bool {
        self.velocity.is_some()
    }

    /// The scroll delta for the next `dt` seconds, slowing the fling down. `offset` and
    /// `max_offset` are the scroll position and its limit: an axis that reaches its edge
    /// stops there, and the fling ends once neither axis moves.
    pub fn step(&mut self, dt: f32, offset: Vec2, max_offset: Vec2) -> Vec2 {
        let Some(v) = self.velocity else { return Vec2::ZERO };
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.1) } else { 0.0 };
        let (dx, vx) = clamp_to_edge(v.x * dt, v.x, offset.x, max_offset.x);
        let (dy, vy) = clamp_to_edge(v.y * dt, v.y, offset.y, max_offset.y);
        let (delta, v) = (Vec2::new(dx, dy), Vec2::new(vx, vy));
        let next = v * (1.0 - FLING_DECAY * dt).max(0.0);
        self.velocity = (next.length() >= FLING_STOP_SPEED).then_some(next);
        delta
    }
}

/// One axis of a fling step: the delta cut at the content's edge, and the velocity, zero once
/// the edge is reached.
fn clamp_to_edge(delta: f32, velocity: f32, offset: f32, max_offset: f32) -> (f32, f32) {
    // A positive delta moves the content down/right, so the offset falls by it.
    let room = if delta > 0.0 { offset } else { max_offset - offset };
    let room = if room.is_finite() { room.max(0.0) } else { 0.0 };
    if delta.abs() >= room { (room.copysign(delta), 0.0) } else { (delta, velocity) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    const FRAME: f32 = 1.0 / 60.0;
    const ROOM: Vec2 = Vec2::splat(1.0e6);

    /// Runs a fling to its end from the middle of a long document: total distance and frames.
    fn run_out(f: &mut Fling) -> (Vec2, usize) {
        let (mut total, mut frames) = (Vec2::ZERO, 0);
        while f.flinging() && frames < 1000 {
            total += f.step(FRAME, ROOM * 0.5 - total, ROOM);
            frames += 1;
        }
        (total, frames)
    }

    #[test]
    fn a_quick_release_keeps_scrolling_and_slows_to_a_stop() {
        let mut f = Fling::default();
        f.release(vec2(0.0, -1200.0));
        assert!(f.flinging());
        let first = f.step(FRAME, ROOM * 0.5, ROOM);
        assert!(first.y < 0.0 && first.x == 0.0, "it goes the way the finger went: {first:?}");
        let second = f.step(FRAME, ROOM * 0.5, ROOM);
        assert!(second.y < 0.0 && second.y.abs() < first.y.abs(), "and slows down");
        let (total, frames) = run_out(&mut f);
        assert!(!f.flinging() && frames < 1000, "it stops on its own");
        assert!(total.y < -100.0, "a page-length glide, not a nudge: {total:?}");
    }

    #[test]
    fn a_slow_release_does_not_fling() {
        let mut f = Fling::default();
        f.release(vec2(100.0, 120.0));
        assert!(!f.flinging());
        assert_eq!(f.step(FRAME, ROOM * 0.5, ROOM), Vec2::ZERO);
        for nonsense in [vec2(f32::NAN, 0.0), vec2(f32::INFINITY, 0.0)] {
            f.release(nonsense);
            assert!(!f.flinging(), "{nonsense:?}");
        }
    }

    #[test]
    fn a_new_touch_stops_it() {
        let mut f = Fling::default();
        f.release(vec2(900.0, 0.0));
        f.step(FRAME, ROOM * 0.5, ROOM);
        f.press();
        assert!(!f.flinging());
        assert_eq!(f.step(FRAME, ROOM * 0.5, ROOM), Vec2::ZERO);
    }

    #[test]
    fn a_pinch_does_not_fling_but_the_next_pan_does() {
        let mut f = Fling::default();
        f.pinch();
        f.release(vec2(0.0, 2000.0));
        assert!(!f.flinging());
        f.press();
        f.release(vec2(0.0, 2000.0));
        assert!(f.flinging());
    }

    #[test]
    fn it_stops_at_the_content_edge() {
        let mut f = Fling::default();
        // Flung towards the end, 10 points from it.
        f.release(vec2(0.0, -3000.0));
        let max = vec2(0.0, 500.0);
        let d = f.step(FRAME, vec2(0.0, 490.0), max);
        assert_eq!(d, vec2(0.0, -10.0), "it goes as far as the edge, not past it");
        assert!(!f.flinging(), "nothing left to move");
        // Flung diagonally into the top: the sideways glide goes on alone.
        f.release(vec2(1500.0, 1500.0));
        let d = f.step(FRAME, vec2(300.0, 0.0), vec2(600.0, 500.0));
        assert_eq!(d.y, 0.0);
        assert!(d.x > 0.0 && f.flinging());
        let d = f.step(FRAME, vec2(300.0 - d.x, 0.0), vec2(600.0, 500.0));
        assert!(d.y == 0.0 && d.x > 0.0);
    }
}
