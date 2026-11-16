//! The Lasso: a loop drawn around drawings and comments selects them together, so they can be
//! moved, recoloured, copied or deleted as one, as in handwriting apps.

use super::state::PageCx;

/// Whether `p` lies inside the closed loop `ring` (even-odd rule). Fewer than three points enclose
/// nothing; non-finite points never do.
pub(super) fn inside(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    if ring.len() < 3 || !(p[0].is_finite() && p[1].is_finite()) {
        return false;
    }
    let mut odd = false;
    let mut j = ring.len() - 1;
    for (i, a) in ring.iter().enumerate() {
        let b = ring.get(j).copied().unwrap_or(*a);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let t = (p[1] - a[1]) / (b[1] - a[1]);
            if p[0] < a[0] + t * (b[0] - a[0]) {
                odd = !odd;
            }
        }
        j = i;
    }
    odd
}

/// Whether the box `rect` (x0, y0, x1, y1) counts as inside the loop: its centre is, or at least
/// five of nine points spread over it are (a long stroke circled loosely still counts).
pub(super) fn takes(ring: &[[f64; 2]], rect: [f64; 4]) -> bool {
    let [x0, y0, x1, y1] = rect;
    let centre = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
    if inside(ring, centre) {
        return true;
    }
    let mut n = 0;
    for fx in [0.1, 0.5, 0.9] {
        for fy in [0.1, 0.5, 0.9] {
            if inside(ring, [x0 + (x1 - x0) * fx, y0 + (y1 - y0) * fy]) {
                n += 1;
            }
        }
    }
    n >= 5
}

/// The comments on this page the loop `ring` (user space) takes, by index: replies and text
/// markup stay out (markup belongs to its text and can't be moved).
pub(super) fn lassoed(cx: &PageCx<'_>, ring: &[[f64; 2]]) -> Vec<usize> {
    cx.comments().filter(|a| !a.is_reply() && !super::geometry::on_text(a)).filter(|a| takes(ring, a.rect.map(f64::from))).map(|a| a.index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f64; 2]; 4] = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];

    #[test]
    fn points_inside_and_outside_a_loop() {
        assert!(inside(&SQUARE, [50.0, 50.0]));
        assert!(!inside(&SQUARE, [150.0, 50.0]));
        assert!(!inside(&SQUARE, [f64::NAN, 50.0]));
        assert!(!inside(&SQUARE[..2], [50.0, 0.0]), "two points enclose nothing");
    }

    #[test]
    fn a_box_is_taken_by_its_centre_or_most_of_it() {
        assert!(takes(&SQUARE, [10.0, 10.0, 40.0, 40.0]));
        assert!(takes(&SQUARE, [-20.0, 20.0, 60.0, 80.0]), "centre inside");
        assert!(!takes(&SQUARE, [80.0, 80.0, 200.0, 200.0]), "mostly outside");
        assert!(!takes(&SQUARE, [200.0, 200.0, 300.0, 300.0]));
    }
}
