//! Hold-to-snap: a pen stroke held still at its end turns into the clean shape it looks like (a
//! line, a rectangle, an oval, a triangle or another polygon), as handwriting apps do. Pure
//! geometry in user space, so it is tested without a window.

use pdfcraft_engine::{LineEnding, Shape};

/// A shape a stroke snapped to, in user space.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Snap {
    Line { from: [f64; 2], to: [f64; 2] },
    Rect([f64; 4]),
    Oval([f64; 4]),
    Polygon(Vec<[f64; 2]>),
}

/// The most points looked at; longer strokes are thinned first (the recogniser is quadratic at worst).
const MAX_POINTS: usize = 2000;
/// The most corners a snapped polygon has; more is a scribble, not a shape.
const MAX_CORNERS: usize = 8;

impl Snap {
    /// The comment drawn for it.
    pub(crate) fn shape(&self) -> Shape {
        match self {
            Self::Line { from, to } => Shape::Line { from: *from, to: *to, start: LineEnding::None, end: LineEnding::None },
            Self::Rect(rect) => Shape::Rectangle { rect: *rect },
            Self::Oval(rect) => Shape::Oval { rect: *rect },
            Self::Polygon(vertices) => Shape::Polygon { vertices: vertices.clone(), cloud: false },
        }
    }

    /// Its outline as one path (closed shapes end where they start), to preview it as a stroke.
    pub(crate) fn outline(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Line { from, to } => vec![*from, *to],
            Self::Rect([x0, y0, x1, y1]) => vec![[*x0, *y0], [*x1, *y0], [*x1, *y1], [*x0, *y1], [*x0, *y0]],
            Self::Oval([x0, y0, x1, y1]) => {
                let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
                (0..=48).map(|i| f64::from(i) / 48.0 * std::f64::consts::TAU).map(|t| [cx + rx * t.cos(), cy + ry * t.sin()]).collect()
            }
            Self::Polygon(v) => v.iter().chain(v.first()).copied().collect(),
        }
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// How far `p` lies from the line through `a` and `b` (from `a`, when they meet).
fn off_line(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let len = dist(a, b);
    if len < 1e-9 {
        return dist(p, a);
    }
    ((b[0] - a[0]) * (a[1] - p[1]) - (a[0] - p[0]) * (b[1] - a[1])).abs() / len
}

/// Ramer–Douglas–Peucker, with a stack instead of recursion: the points of `pts` that keep its
/// shape within `eps`.
pub(super) fn simplify(pts: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0, n - 1)];
    while let Some((a, b)) = stack.pop() {
        let (Some(pa), Some(pb)) = (pts.get(a), pts.get(b)) else { continue };
        let mut far = (0.0, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = off_line(*p, *pa, *pb);
            if d > far.0 {
                far = (d, i);
            }
        }
        if far.0 > eps {
            if let Some(k) = keep.get_mut(far.1) {
                *k = true;
            }
            stack.push((a, far.1));
            stack.push((far.1, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// The turn at `b` going from `a` to `c`, in degrees (0: straight on).
fn turn(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
    let cross = u[0] * v[1] - u[1] * v[0];
    let dot = u[0] * v[0] + u[1] * v[1];
    cross.atan2(dot).abs().to_degrees()
}

/// The shape the stroke `points` (user space) looks like, if any.
pub(crate) fn recognise(points: &[[f64; 2]]) -> Option<Snap> {
    let mut pts: Vec<[f64; 2]> = points.iter().copied().filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
    if pts.len() > MAX_POINTS {
        let step = pts.len().div_ceil(MAX_POINTS);
        let last = pts.last().copied();
        pts = pts.iter().step_by(step).copied().chain(last).collect();
    }
    let (first, last) = (*pts.first()?, *pts.last()?);
    if pts.len() < 3 {
        return None;
    }
    let length: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in &pts {
        (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let size = w.hypot(h);
    if length < 8.0 || size < 6.0 {
        return None;
    }
    // Straight: every point close to the line from the first to the last.
    let chord = dist(first, last);
    if chord >= 0.7 * length && pts.iter().all(|p| off_line(*p, first, last) <= (0.07 * chord).max(1.0)) {
        return Some(Snap::Line { from: first, to: last });
    }
    // Only a closed stroke (ending near its start) makes a shape.
    if chord > 0.25 * size {
        return None;
    }
    // An oval: every point near the ellipse that fills the stroke's box.
    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, w / 2.0, h / 2.0);
    if rx > 2.0 && ry > 2.0 {
        let off: f64 = pts.iter().map(|p| (((p[0] - cx) / rx).hypot((p[1] - cy) / ry) - 1.0).abs()).sum::<f64>() / pts.len() as f64;
        if off <= 0.09 {
            return Some(Snap::Oval([x0, y0, x1, y1]));
        }
    }
    // A polygon: its corners, where the stroke turns.
    let mut corners = simplify(&pts, 0.05 * size);
    if corners.len() >= 2 && corners.first().zip(corners.last()).is_some_and(|(a, b)| dist(*a, *b) <= 0.25 * size) {
        corners.pop();
    }
    // Drop points where the outline goes nearly straight on (the stroke may start mid-side).
    loop {
        let n = corners.len();
        if n < 3 {
            return None;
        }
        let flat = (0..n).find(|&i| {
            let (a, b, c) = (corners[(i + n - 1) % n], corners[i], corners[(i + 1) % n]);
            turn(a, b, c) < 25.0
        });
        match flat {
            Some(i) => {
                corners.remove(i);
            }
            None => break,
        }
    }
    match corners.len() {
        4 => {
            // Sides along the page's axes: a rectangle; otherwise a four-sided polygon.
            let n = corners.len();
            let square = (0..n).all(|i| {
                let (a, b) = (corners[i], corners[(i + 1) % n]);
                let deg = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees().rem_euclid(90.0);
                !(15.0..=75.0).contains(&deg)
            });
            Some(if square { Snap::Rect([x0, y0, x1, y1]) } else { Snap::Polygon(corners) })
        }
        3..=MAX_CORNERS => Some(Snap::Polygon(corners)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points along the sides of `corners`, closed, `per` to a side, with a little wobble.
    fn traced(corners: &[[f64; 2]], per: usize) -> Vec<[f64; 2]> {
        let mut out = Vec::new();
        for (i, a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % corners.len()];
            for k in 0..per {
                let t = k as f64 / per as f64;
                let wobble = if k % 2 == 0 { 0.6 } else { -0.6 };
                out.push([a[0] + (b[0] - a[0]) * t + wobble, a[1] + (b[1] - a[1]) * t]);
            }
        }
        out.push(corners[0]);
        out
    }

    #[test]
    fn a_straight_stroke_snaps_to_a_line() {
        let pts: Vec<[f64; 2]> = (0..30).map(|i| [10.0 + i as f64 * 5.0, 50.0 + if i % 2 == 0 { 1.0 } else { -1.0 }]).collect();
        assert!(matches!(recognise(&pts), Some(Snap::Line { .. })));
    }

    #[test]
    fn a_closed_loop_snaps_to_an_oval() {
        let pts: Vec<[f64; 2]> =
            (0..=60).map(|i| f64::from(i) / 60.0 * std::f64::consts::TAU).map(|t| [100.0 + 60.0 * t.cos() * 1.02, 100.0 + 30.0 * t.sin()]).collect();
        let Some(Snap::Oval(r)) = recognise(&pts) else { panic!("{:?}", recognise(&pts)) };
        assert!((r[2] - r[0] - 122.4).abs() < 2.0 && (r[3] - r[1] - 60.0).abs() < 2.0, "{r:?}");
    }

    #[test]
    fn boxes_and_triangles_snap_to_their_corners() {
        let rect = traced(&[[20.0, 20.0], [180.0, 20.0], [180.0, 120.0], [20.0, 120.0]], 12);
        assert!(matches!(recognise(&rect), Some(Snap::Rect(_))), "{:?}", recognise(&rect));
        // Starting mid-side changes nothing.
        let mut from_middle = rect.clone();
        from_middle.rotate_left(6);
        from_middle.push(from_middle[0]);
        assert!(matches!(recognise(&from_middle), Some(Snap::Rect(_))), "{:?}", recognise(&from_middle));
        let tri = traced(&[[20.0, 20.0], [180.0, 20.0], [100.0, 150.0]], 15);
        let Some(Snap::Polygon(v)) = recognise(&tri) else { panic!("{:?}", recognise(&tri)) };
        assert_eq!(v.len(), 3);
        // A diamond is a polygon, not a rectangle.
        let diamond = traced(&[[100.0, 20.0], [180.0, 100.0], [100.0, 180.0], [20.0, 100.0]], 12);
        assert!(matches!(recognise(&diamond), Some(Snap::Polygon(v)) if v.len() == 4), "{:?}", recognise(&diamond));
    }

    #[test]
    fn scribbles_and_open_curves_stay_as_they_are() {
        // An open arc: neither straight nor closed.
        let arc: Vec<[f64; 2]> =
            (0..=30).map(|i| f64::from(i) / 30.0 * std::f64::consts::PI).map(|t| [100.0 + 50.0 * t.cos(), 50.0 * t.sin()]).collect();
        assert_eq!(recognise(&arc), None);
        assert_eq!(recognise(&[[0.0, 0.0], [1.0, 1.0]]), None, "too short");
        assert_eq!(recognise(&[[f64::NAN, 0.0]; 10]), None);
        // A zigzag that closes: too many corners.
        let zig: Vec<[f64; 2]> = (0..40)
            .map(|i| f64::from(i) / 40.0 * std::f64::consts::TAU)
            .enumerate()
            .map(|(i, t)| {
                let r = if i % 2 == 0 { 80.0 } else { 30.0 };
                [100.0 + r * t.cos(), 100.0 + r * t.sin()]
            })
            .chain([[180.0, 100.0]])
            .collect();
        assert_eq!(recognise(&zig), None, "{:?}", recognise(&zig));
    }
}
