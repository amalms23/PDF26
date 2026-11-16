//! Touchpad momentum (#531): a two-finger swipe keeps gliding after the fingers lift, and slows
//! to a stop.
//!
//! Wayland compositors leave momentum to the client. libinput reports the swipe as pixel deltas
//! (`MouseWheelUnit::Point`) and the lift as an axis stop, which winit passes on as
//! `TouchPhase::End`; GTK and Firefox glide on from there, and so does the page view.
//!
//! macOS sends its own momentum as a second `Start`…`End`, so gliding there would glide twice,
//! and the web has no touch phases: [`Fling::enabled`] is off on both. Windows and X11 report no
//! touch phases through winit either, so no `End` ever starts a glide there.

use egui::{Event, MouseWheelUnit, TouchPhase, Vec2};

/// Seconds of the gesture the lift speed is measured over.
const WINDOW: f64 = 0.1;
/// A lift this long after the last motion is a finger that stopped first: no glide.
const STILL: f64 = 0.05;
/// Passes of motion kept however old, so a slow device still measures a speed.
const KEEP: usize = 3;
/// The glide slows by `e` every this many seconds; it travels the lift speed times this.
const TAU: f32 = 0.35;
/// Below this speed (points per second) a lift doesn't glide, and a glide stops.
const START_SPEED: f32 = 150.0;
const STOP_SPEED: f32 = 10.0;
/// Fastest glide, in points per second: a flick the system measured over a hiccup can't throw
/// the document to its end.
const MAX_SPEED: f32 = 8000.0;
/// A pass this long after the last one (the view was hidden, the app stalled) ends a glide.
const STALE: f64 = 0.1;

pub(crate) struct Fling {
    /// Whether a lift glides on this platform (see the module docs).
    pub(crate) enabled: bool,
    /// The gesture in progress: when each pass's motion arrived, and how far it went.
    samples: Vec<(f64, Vec2)>,
    /// The glide's speed in points per second (positive moves the content down), or zero.
    velocity: Vec2,
    /// When the glide last moved.
    at: f64,
    /// Passes in a row the glide moved the view not at all: it is against an edge.
    stalled: u8,
}

impl Default for Fling {
    fn default() -> Self {
        Self::new(!cfg!(any(target_os = "macos", target_arch = "wasm32")))
    }
}

impl Fling {
    pub(crate) fn new(enabled: bool) -> Self {
        Self { enabled, samples: Vec::new(), velocity: Vec2::ZERO, at: 0.0, stalled: 0 }
    }

    pub(crate) fn gliding(&self) -> bool {
        self.velocity != Vec2::ZERO
    }

    pub(crate) fn stop(&mut self) {
        self.velocity = Vec2::ZERO;
        self.stalled = 0;
    }

    /// Reads one pass's wheel events, at time `now`. Any wheel input stops a glide; a touchpad
    /// gesture's `End` may start one. `glide` is false where the view must not glide this pass
    /// (animations off, single-page paging, a zoom, a dialog).
    pub(crate) fn read(&mut self, events: &[Event], now: f64, glide: bool) {
        for event in events {
            let Event::MouseWheel { unit, delta, phase, modifiers, .. } = event else { continue };
            self.stop();
            // Zooming, a mouse wheel and sideways Shift scrolling never glide.
            if *unit != MouseWheelUnit::Point || modifiers.command || modifiers.ctrl || modifiers.shift {
                self.samples.clear();
                continue;
            }
            match phase {
                TouchPhase::Start => {
                    self.samples.clear();
                    self.push(now, *delta);
                }
                TouchPhase::Move => self.push(now, *delta),
                TouchPhase::End => {
                    if glide && self.enabled {
                        self.velocity = self.lift_velocity(now);
                        self.at = now;
                    }
                    self.samples.clear();
                }
                TouchPhase::Cancel => self.samples.clear(),
            }
        }
    }

    /// A finger panning the page on a touch screen moved by `delta` (the pointer's motion, which
    /// has a wheel delta's sign) in the pass at `now`.
    pub(crate) fn drag(&mut self, now: f64, delta: Vec2) {
        self.stop();
        self.push(now, delta);
    }

    /// That finger lifted at `now`: the page glides on at its speed. `frame` is the time between
    /// passes: a finger that moved in the last pass before the lift still counts as moving when
    /// the passes come slower than [`STILL`].
    pub(crate) fn lift(&mut self, now: f64, frame: f64) {
        self.velocity = self.lift_velocity_within(now, STILL.max(frame * 1.5));
        self.at = now;
        self.stalled = 0;
        self.samples.clear();
    }

    fn push(&mut self, now: f64, delta: Vec2) {
        if !(delta.x.is_finite() && delta.y.is_finite()) {
            return;
        }
        match self.samples.last_mut() {
            // Several events in one pass are one sample: they share its time.
            Some((at, d)) if *at == now => *d += delta,
            _ => self.samples.push((now, delta)),
        }
        // The last few passes stay whatever their age: at a slow frame rate the window holds
        // too few to measure a speed from (a lift measures over the window it allows).
        let keep_from = self.samples.len().saturating_sub(KEEP);
        let mut i = 0;
        self.samples.retain(|(at, _)| {
            i += 1;
            i > keep_from || now - at <= WINDOW
        });
    }

    /// The finger's speed as it lifted at `now`, or zero when it should not glide.
    fn lift_velocity(&self, now: f64) -> Vec2 {
        self.lift_velocity_within(now, STILL)
    }

    /// [`Self::lift_velocity`], with a finger still for longer than `still` seconds not gliding.
    fn lift_velocity_within(&self, now: f64, still: f64) -> Vec2 {
        let recent: Vec<&(f64, Vec2)> = self.samples.iter().filter(|(at, _)| now - at <= WINDOW.max(still * 2.0)).collect();
        let (Some(first), Some(last)) = (recent.first(), recent.last()) else { return Vec2::ZERO };
        let span = (last.0 - first.0) as f32;
        if recent.len() < 2 || span <= 0.0 || now - last.0 > still {
            return Vec2::ZERO;
        }
        // Each sample is the motion since the pass before it, so the first one's motion
        // happened before the span starts.
        let moved = recent.iter().skip(1).fold(Vec2::ZERO, |sum, (_, d)| sum + *d);
        let v = moved / span;
        if !(v.x.is_finite() && v.y.is_finite()) || v.length() < START_SPEED {
            return Vec2::ZERO;
        }
        if v.length() > MAX_SPEED { v * (MAX_SPEED / v.length()) } else { v }
    }

    /// How far the glide moves the view in the pass at `now` (zero when not gliding), as the
    /// exact distance a speed decaying with [`TAU`] covers since the last pass.
    pub(crate) fn step(&mut self, now: f64) -> Vec2 {
        if !self.gliding() {
            return Vec2::ZERO;
        }
        let dt = now - self.at;
        if !(0.0..=STALE).contains(&dt) {
            self.stop();
            return Vec2::ZERO;
        }
        self.at = now;
        let decay = (-(dt as f32) / TAU).exp();
        let moved = self.velocity * TAU * (1.0 - decay);
        self.velocity *= decay;
        if self.velocity.length() < STOP_SPEED {
            self.stop();
        }
        moved
    }

    /// After a pass the glide moved the view in: whether the offset changed. Two passes that
    /// didn't move it mean an edge, and the glide ends there.
    pub(crate) fn moved(&mut self, moved: bool) {
        if moved {
            self.stalled = 0;
        } else {
            self.stalled = self.stalled.saturating_add(1);
            if self.stalled >= 2 {
                self.stop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, vec2};

    fn wheel(dy: f32, phase: TouchPhase) -> Event {
        Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, dy),
            phase,
            modifiers: Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        }
    }

    /// A swipe of `moves` passes of `dy` each, 60 per second, lifted the pass after the last.
    fn swipe(fling: &mut Fling, dy: f32, moves: usize) -> f64 {
        let dt = 1.0 / 60.0;
        let mut now = 0.0;
        fling.read(&[wheel(dy, TouchPhase::Start)], now, true);
        for _ in 0..moves {
            now += dt;
            fling.read(&[wheel(dy, TouchPhase::Move)], now, true);
        }
        now += dt;
        fling.read(&[wheel(0.0, TouchPhase::End)], now, true);
        now
    }

    #[test]
    fn a_flick_glides_on_at_its_lift_speed_and_stops() {
        let mut fling = Fling::new(true);
        let mut now = swipe(&mut fling, -25.0, 6);
        assert!(fling.gliding());
        // 25 points a pass at 60 passes a second.
        assert!((fling.velocity.y + 1500.0).abs() < 1.0, "{:?}", fling.velocity);
        let mut travelled = 0.0;
        let mut passes = 0;
        while fling.gliding() {
            now += 1.0 / 60.0;
            let d = fling.step(now);
            assert!(d.y <= 0.0 && d.x == 0.0, "keeps the swipe's direction: {d:?}");
            travelled += d.y;
            passes += 1;
            assert!(passes < 600, "the glide ends");
        }
        // Speed × TAU, less the tail under STOP_SPEED.
        assert!((-travelled - 1500.0 * TAU).abs() < 10.0, "travelled {travelled}");
        assert_eq!(fling.step(now + 1.0 / 60.0), Vec2::ZERO);
    }

    #[test]
    fn a_finger_flick_glides_on_even_at_a_slow_frame_rate() {
        // Touch screens: 30 points a pass at 60 passes a second, lifted moving.
        let mut fling = Fling::new(false);
        let mut now = 0.0;
        for _ in 0..6 {
            now += 1.0 / 60.0;
            fling.drag(now, vec2(0.0, -30.0));
        }
        fling.lift(now, 1.0 / 60.0);
        assert!((fling.velocity.y + 1800.0).abs() < 1.0, "glides whatever `enabled` says: {:?}", fling.velocity);
        // A slow device: 10 passes a second, the lift a pass after the last motion.
        let mut fling = Fling::new(true);
        let mut now = 0.0;
        for _ in 0..4 {
            now += 0.1;
            fling.drag(now, vec2(0.0, 100.0));
        }
        fling.lift(now + 0.1, 0.1);
        assert!(fling.gliding(), "a finger moving in the last pass glides");
        // As an emulator drew it: two passes of motion 52 ms apart, then the lift's pass with none.
        let mut fling = Fling::new(true);
        fling.drag(7.390, vec2(0.0, -121.7));
        fling.drag(7.442, vec2(0.0, -65.0));
        fling.drag(7.501, Vec2::ZERO);
        fling.lift(7.501, 0.058);
        assert!(fling.velocity.y < -START_SPEED, "{:?}", fling.velocity);
        // A finger that stopped before lifting doesn't.
        let mut fling = Fling::new(true);
        for i in 1..=6 {
            fling.drag(f64::from(i) / 60.0, vec2(0.0, -30.0));
        }
        fling.lift(0.5, 1.0 / 60.0);
        assert!(!fling.gliding());
    }

    #[test]
    fn a_slow_drag_or_a_pause_before_lifting_does_not_glide() {
        let mut fling = Fling::new(true);
        swipe(&mut fling, -1.0, 6);
        assert!(!fling.gliding(), "60 points a second is below the start speed");

        let mut fling = Fling::new(true);
        let dt = 1.0 / 60.0;
        fling.read(&[wheel(-25.0, TouchPhase::Start)], 0.0, true);
        fling.read(&[wheel(-25.0, TouchPhase::Move)], dt, true);
        fling.read(&[wheel(0.0, TouchPhase::End)], dt + 0.2, true);
        assert!(!fling.gliding(), "the finger rested before it lifted");
    }

    /// Check that new finger or wheel input stops an in-progress glide.
    #[test]
    fn new_input_stops_a_glide() {
        let mut fling = Fling::new(true);
        let now = swipe(&mut fling, -25.0, 6);
        fling.read(&[wheel(-2.0, TouchPhase::Start)], now + 0.1, true);
        assert!(!fling.gliding(), "fingers back on the touchpad");

        let mut fling = Fling::new(true);
        let now = swipe(&mut fling, -25.0, 6);
        let notch = Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        };
        fling.read(&[notch], now + 0.1, true);
        assert!(!fling.gliding(), "a wheel notch");
    }

    /// Check that disabled flinging, a false glide flag, and a command-modified zoom gesture prevent glide.
    #[test]
    fn no_glide_where_it_is_off_or_not_wanted() {
        let mut off = Fling::new(false);
        swipe(&mut off, -25.0, 6);
        assert!(!off.gliding(), "macOS and the web glide by themselves or not at all");

        let mut fling = Fling::new(true);
        let dt = 1.0 / 60.0;
        fling.read(&[wheel(-25.0, TouchPhase::Start)], 0.0, true);
        fling.read(&[wheel(-25.0, TouchPhase::Move)], dt, true);
        fling.read(&[wheel(-25.0, TouchPhase::Move)], 2.0 * dt, true);
        fling.read(&[wheel(0.0, TouchPhase::End)], 3.0 * dt, false);
        assert!(!fling.gliding(), "animations off, paging or a dialog");

        let mut zoom = Fling::new(true);
        let ctrl = |phase| Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -25.0),
            phase,
            modifiers: Modifiers::COMMAND,
            source: egui::MouseWheelSource::Unknown,
        };
        zoom.read(&[ctrl(TouchPhase::Start)], 0.0, true);
        zoom.read(&[ctrl(TouchPhase::Move)], dt, true);
        zoom.read(&[ctrl(TouchPhase::Move)], 2.0 * dt, true);
        zoom.read(&[ctrl(TouchPhase::End)], 3.0 * dt, true);
        assert!(!zoom.gliding(), "a pinch or ⌘/Ctrl swipe zooms");
    }

    #[test]
    fn an_edge_a_stall_or_a_wild_speed_ends_or_caps_the_glide() {
        let mut fling = Fling::new(true);
        let now = swipe(&mut fling, -25.0, 6);
        fling.step(now + 1.0 / 60.0);
        fling.moved(false);
        assert!(fling.gliding(), "one pass may not show the move yet");
        fling.moved(false);
        assert!(!fling.gliding(), "against the edge");

        let mut fling = Fling::new(true);
        let now = swipe(&mut fling, -25.0, 6);
        assert_eq!(fling.step(now + 1.0), Vec2::ZERO, "a second-long gap (the tab was hidden)");
        assert!(!fling.gliding());

        let mut fling = Fling::new(true);
        swipe(&mut fling, -5000.0, 6);
        assert!((fling.velocity.length() - MAX_SPEED).abs() < 1.0, "{:?}", fling.velocity);

        let mut fling = Fling::new(true);
        swipe(&mut fling, f32::NAN, 6);
        assert!(!fling.gliding(), "a speed that isn't a number");
    }
}
