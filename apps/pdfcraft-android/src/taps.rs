//! Tapping the screen with several fingers at once, as handwriting apps read it: two fingers
//! undo, three redo. A tap is short and still; fingers that move are a pinch or a scroll.

/// The longest tap, in seconds.
const MAX_TAP: f64 = 0.35;
/// How far a finger may drift in a tap, in points.
const SLOP: f32 = 18.0;

/// What a tap asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tap {
    Undo,
    Redo,
}

/// The touches of the gesture in progress.
#[derive(Debug, Default)]
pub(crate) struct Taps {
    /// The fingers down now, where each touched.
    down: Vec<(u64, [f32; 2])>,
    /// The most fingers down at once in this gesture.
    most: usize,
    /// When the first finger touched.
    start: f64,
    /// A finger moved past the slop, or a stylus took part: not a tap.
    spoiled: bool,
}

impl Taps {
    /// Touch `id` went down at `pos` at time `now`. `stylus`: the stylus, which never taps.
    pub(crate) fn start(&mut self, id: u64, pos: [f32; 2], now: f64, stylus: bool) {
        if self.down.is_empty() {
            *self = Self { start: now, ..Self::default() };
        }
        self.spoiled |= stylus;
        self.down.retain(|(d, _)| *d != id);
        self.down.push((id, pos));
        self.most = self.most.max(self.down.len());
    }

    /// Touch `id` is at `pos` now.
    pub(crate) fn moved(&mut self, id: u64, pos: [f32; 2]) {
        if let Some((_, at)) = self.down.iter().find(|(d, _)| *d == id) {
            let (dx, dy) = (pos[0] - at[0], pos[1] - at[1]);
            if dx * dx + dy * dy > SLOP * SLOP || dx.is_nan() || dy.is_nan() {
                self.spoiled = true;
            }
        }
    }

    /// Touch `id` lifted at time `now`: when it was the last, what the gesture was.
    pub(crate) fn end(&mut self, id: u64, now: f64) -> Option<Tap> {
        self.down.retain(|(d, _)| *d != id);
        let quick = now - self.start <= MAX_TAP;
        if !self.down.is_empty() || self.spoiled || !quick {
            return None;
        }
        let tap = match self.most {
            2 => Some(Tap::Undo),
            3 => Some(Tap::Redo),
            _ => None,
        };
        self.most = 0;
        tap
    }

    /// A touch was cancelled: the gesture is no tap.
    pub(crate) fn cancel(&mut self, id: u64) {
        self.spoiled = true;
        self.down.retain(|(d, _)| *d != id);
    }

    /// No touch is down.
    pub(crate) fn idle(&self) -> bool {
        self.down.is_empty()
    }
}

/// The most keys of one kind taken in a frame: a stuck counter can't flood the app.
const MAX_KEYS: u32 = 8;

/// The activity's "enters taps" answer: how many times the soft keyboard's Enter and a stylus
/// button were pressed. Anything unreadable counts as none.
pub(crate) fn parse_keys(answer: &str) -> (u32, u32) {
    let mut n = answer.split_whitespace().map(|s| s.parse::<u32>().unwrap_or(0).min(MAX_KEYS));
    (n.next().unwrap_or(0), n.next().unwrap_or(0))
}

/// A stylus button tapped: the eraser, or, when erasing already, back to the tool it was tapped
/// from (`back`; the `pen` when there is none). Generic over the tool so it is tested without
/// the app, which only builds for Android here.
pub(crate) fn toggle_eraser<T: Copy + PartialEq>(tool: T, back: &mut Option<T>, eraser: T, pen: T) -> T {
    if tool == eraser {
        back.take().unwrap_or(pen)
    } else {
        *back = Some(tool);
        eraser
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stylus_button_tap_switches_to_the_eraser_and_back() {
        let (pen, highlighter, eraser) = ("pen", "highlighter", "eraser");
        let mut back = None;
        let erasing = toggle_eraser(highlighter, &mut back, eraser, pen);
        assert_eq!(erasing, eraser);
        assert_eq!(toggle_eraser(erasing, &mut back, eraser, pen), highlighter);
        // Picked from the toolbar, the eraser goes back to the pen.
        assert_eq!(toggle_eraser(eraser, &mut None, eraser, pen), pen);
    }

    #[test]
    fn the_activitys_key_counts_are_read_safely() {
        assert_eq!(parse_keys("1 2"), (1, 2));
        assert_eq!(parse_keys("0 0"), (0, 0));
        assert_eq!(parse_keys("junk"), (0, 0));
        assert_eq!(parse_keys("99999999999 3"), (0, 3));
        assert_eq!(parse_keys("500 1"), (MAX_KEYS, 1));
    }

    fn tap(fingers: u64, hold: f64, drift: f32) -> Option<Tap> {
        let mut t = Taps::default();
        for id in 0..fingers {
            t.start(id, [100.0 * id as f32, 100.0], 0.0, false);
        }
        for id in 0..fingers {
            t.moved(id, [100.0 * id as f32 + drift, 100.0]);
        }
        let mut out = None;
        for id in 0..fingers {
            out = t.end(id, hold);
        }
        out
    }

    #[test]
    fn two_fingers_undo_three_redo() {
        assert_eq!(tap(2, 0.1, 2.0), Some(Tap::Undo));
        assert_eq!(tap(3, 0.1, 2.0), Some(Tap::Redo));
        assert_eq!(tap(1, 0.1, 0.0), None, "one finger is a click");
        assert_eq!(tap(4, 0.1, 0.0), None);
    }

    #[test]
    fn a_pinch_a_hold_or_the_stylus_is_no_tap() {
        assert_eq!(tap(2, 0.1, 40.0), None, "fingers that moved pinch or scroll");
        assert_eq!(tap(2, 0.8, 0.0), None, "a hold");
        let mut t = Taps::default();
        t.start(0, [0.0, 0.0], 0.0, true);
        t.start(1, [50.0, 0.0], 0.0, false);
        assert_eq!(t.end(0, 0.1), None);
        assert_eq!(t.end(1, 0.1), None);
        assert!(t.idle());
        // The next gesture starts afresh.
        assert_eq!(tap(2, 0.1, 0.0), Some(Tap::Undo));
    }
}
