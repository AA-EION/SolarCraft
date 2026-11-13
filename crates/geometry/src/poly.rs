// SPDX-License-Identifier: GPL-3.0-or-later
//! Polygon predicates and distances (plane coordinates in metres).
pub type P = [f64; 2];

/// Even-odd point-in-ring test (boundary points count as inside).
pub fn point_in_ring(p: P, ring: &[P]) -> bool {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if point_segment_distance(p, a, b) < 1e-12 { return true; }
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] { inside = !inside; }
    }
    inside
}

pub fn point_segment_distance(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

fn orient(a: P, b: P, c: P) -> f64 { (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) }

/// Proper or touching intersection of segments ab and cd.
pub fn segments_intersect(a: P, b: P, c: P, d: P) -> bool {
    let (o1, o2, o3, o4) = (orient(a, b, c), orient(a, b, d), orient(c, d, a), orient(c, d, b));
    if ((o1 > 0.0) != (o2 > 0.0)) && ((o3 > 0.0) != (o4 > 0.0)) && o1 != 0.0 && o2 != 0.0 && o3 != 0.0 && o4 != 0.0 { return true; }
    point_segment_distance(c, a, b) < 1e-12 || point_segment_distance(d, a, b) < 1e-12 || point_segment_distance(a, c, d) < 1e-12 || point_segment_distance(b, c, d) < 1e-12
}

pub fn segment_segment_distance(a: P, b: P, c: P, d: P) -> f64 {
    if segments_intersect(a, b, c, d) { return 0.0; }
    point_segment_distance(a, c, d).min(point_segment_distance(b, c, d)).min(point_segment_distance(c, a, b)).min(point_segment_distance(d, a, b))
}

/// Axis-aligned rectangle (x0 < x1, y0 < y1).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rect { pub x0: f64, pub y0: f64, pub x1: f64, pub y1: f64 }

impl Rect {
    pub fn corners(&self) -> [P; 4] { [[self.x0, self.y0], [self.x1, self.y0], [self.x1, self.y1], [self.x0, self.y1]] }
    pub fn contains(&self, p: P) -> bool { p[0] >= self.x0 && p[0] <= self.x1 && p[1] >= self.y0 && p[1] <= self.y1 }
    /// Minimum distance between the rectangle (as a filled region) and a ring's boundary.
    pub fn distance_to_ring_boundary(&self, ring: &[P]) -> f64 {
        let c = self.corners();
        let mut d = f64::INFINITY;
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if self.contains(a) { return 0.0; }
            for k in 0..4 { d = d.min(segment_segment_distance(c[k], c[(k + 1) % 4], a, b)); }
        }
        d
    }
}

pub fn ring_area(r: &[P]) -> f64 { (0..r.len()).map(|i| { let (a, b) = (r[i], r[(i + 1) % r.len()]); a[0] * b[1] - b[0] * a[1] }).sum::<f64>() / 2.0 }
