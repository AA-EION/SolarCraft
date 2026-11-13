// SPDX-License-Identifier: GPL-3.0-or-later
//! Module placement on a planar roof face (feature rows LAY-002, LAY-008, LAY-012).
//!
//! The roof is given in its own plane: x along the eave, y up the slope, in
//! metres. A module position is accepted only if the whole module lies inside
//! the roof outline, keeps `setback` from the outline, and keeps
//! `obstacle_clearance` from every obstacle (chimney, skylight, vent).
//! The grid origin is searched on a fine raster, and both orientations are
//! tried, to maximise the module count; ties go to the smallest offset.
use crate::poly::{point_in_ring, Rect, P};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoofFace { pub outline: Vec<P>, pub obstacles: Vec<Vec<P>> }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation { Portrait, Landscape, Best }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutParams {
    /// Module long side and short side, metres.
    pub module_length: f64,
    pub module_width: f64,
    pub orientation: Orientation,
    /// Gap between modules across the slope (x) and along it (y), metres.
    pub gap_x: f64,
    pub gap_y: f64,
    /// Minimum distance from the roof edge (fire paths, wind zones), metres.
    pub setback: f64,
    pub obstacle_clearance: f64,
    /// Raster step for the grid-origin search, metres.
    pub search_step: f64,
}

impl Default for LayoutParams {
    fn default() -> Self { Self { module_length: 1.722, module_width: 1.134, orientation: Orientation::Best, gap_x: 0.02, gap_y: 0.02, setback: 0.4, obstacle_clearance: 0.3, search_step: 0.05 } }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout { pub modules: Vec<Rect>, pub orientation: Orientation, pub origin: P }

pub fn module_fits(face: &RoofFace, r: &Rect, p: &LayoutParams) -> bool {
    if !r.corners().iter().all(|&c| point_in_ring(c, &face.outline)) { return false; }
    if r.distance_to_ring_boundary(&face.outline) < p.setback - 1e-9 { return false; }
    for ob in &face.obstacles {
        if r.corners().iter().any(|&c| point_in_ring(c, ob)) || ob.iter().any(|&v| r.contains(v)) { return false; }
        if r.distance_to_ring_boundary(ob) < p.obstacle_clearance - 1e-9 { return false; }
    }
    true
}

fn fill_one(face: &RoofFace, p: &LayoutParams, w: f64, h: f64, orientation: Orientation) -> Layout {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for v in &face.outline { x0 = x0.min(v[0]); y0 = y0.min(v[1]); x1 = x1.max(v[0]); y1 = y1.max(v[1]); }
    let (px, py) = (w + p.gap_x, h + p.gap_y);
    let steps = |pitch: f64| ((pitch / p.search_step).ceil() as usize).max(1);
    let mut best = Layout { modules: vec![], orientation, origin: [x0, y0] };
    for iy in 0..steps(py) {
        for ix in 0..steps(px) {
            let (ox, oy) = (x0 + ix as f64 * p.search_step, y0 + iy as f64 * p.search_step);
            let mut mods = vec![];
            let mut y = oy;
            while y + h <= y1 + 1e-9 {
                let mut x = ox;
                while x + w <= x1 + 1e-9 {
                    let r = Rect { x0: x, y0: y, x1: x + w, y1: y + h };
                    if module_fits(face, &r, p) { mods.push(r); }
                    x += px;
                }
                y += py;
            }
            if mods.len() > best.modules.len() { best = Layout { modules: mods, orientation, origin: [ox, oy] }; }
        }
    }
    best
}

/// Place as many modules as fit.
pub fn fill(face: &RoofFace, p: &LayoutParams) -> Layout {
    let portrait = || fill_one(face, p, p.module_width, p.module_length, Orientation::Portrait);
    let landscape = || fill_one(face, p, p.module_length, p.module_width, Orientation::Landscape);
    match p.orientation {
        Orientation::Portrait => portrait(),
        Orientation::Landscape => landscape(),
        Orientation::Best => { let (a, b) = (portrait(), landscape()); if b.modules.len() > a.modules.len() { b } else { a } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poly::point_segment_distance;

    fn rect_roof(w: f64, h: f64) -> Vec<P> { vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]] }
    fn params(o: Orientation) -> LayoutParams { LayoutParams { module_length: 1.7, module_width: 1.0, orientation: o, gap_x: 0.02, gap_y: 0.02, setback: 0.4, obstacle_clearance: 0.3, search_step: 0.05 } }

    /// Independent check: dense samples inside each module, setback by vertex/edge distances.
    fn verify(face: &RoofFace, l: &Layout, p: &LayoutParams) {
        for (i, r) in l.modules.iter().enumerate() {
            for a in 0..=4 { for b in 0..=4 {
                let q = [r.x0 + (r.x1 - r.x0) * a as f64 / 4.0, r.y0 + (r.y1 - r.y0) * b as f64 / 4.0];
                assert!(point_in_ring(q, &face.outline), "module {i} sample outside roof");
                let edge_d = (0..face.outline.len()).map(|k| point_segment_distance(q, face.outline[k], face.outline[(k + 1) % face.outline.len()])).fold(f64::MAX, f64::min);
                assert!(edge_d >= p.setback - 1e-6, "module {i} violates setback ({edge_d})");
                for ob in &face.obstacles { assert!(!point_in_ring(q, ob), "module {i} over obstacle"); }
            } }
            for o in &l.modules[i + 1..] { assert!(r.x1 <= o.x0 + 1e-9 || o.x1 <= r.x0 + 1e-9 || r.y1 <= o.y0 + 1e-9 || o.y1 <= r.y0 + 1e-9, "overlap"); }
        }
    }

    #[test]
    fn rectangular_roof_matches_hand_count() {
        // Usable 9.2 × 5.2 m. Portrait 1.0 × 1.7: floor(9.22/1.02)=9 × floor(5.22/1.72)=3 = 27.
        // Landscape 1.7 × 1.0: floor(9.22/1.72)=5 × floor(5.22/1.02)=5 = 25.
        let face = RoofFace { outline: rect_roof(10.0, 6.0), obstacles: vec![] };
        assert_eq!(fill(&face, &params(Orientation::Portrait)).modules.len(), 27);
        assert_eq!(fill(&face, &params(Orientation::Landscape)).modules.len(), 25);
        let best = fill(&face, &params(Orientation::Best));
        assert_eq!((best.modules.len(), best.orientation), (27, Orientation::Portrait));
        verify(&face, &best, &params(Orientation::Best));
    }

    #[test]
    fn chimney_and_trapezoid_hip_roof() {
        let chimney = vec![[4.5, 2.5], [5.5, 2.5], [5.5, 3.5], [4.5, 3.5]];
        let face = RoofFace { outline: rect_roof(10.0, 6.0), obstacles: vec![chimney] };
        let l = fill(&face, &params(Orientation::Best));
        assert!(l.modules.len() < 27 && l.modules.len() >= 20, "{}", l.modules.len());
        verify(&face, &l, &params(Orientation::Best));
        // Hip roof face: trapezoid 12 m eave, 6 m ridge, 5 m slope.
        let hip = RoofFace { outline: vec![[0.0, 0.0], [12.0, 0.0], [9.0, 5.0], [3.0, 5.0]], obstacles: vec![] };
        let l = fill(&hip, &params(Orientation::Best));
        assert!(!l.modules.is_empty());
        verify(&hip, &l, &params(Orientation::Best));
    }

    #[test]
    fn too_small_roof_gives_nothing() {
        let face = RoofFace { outline: rect_roof(1.5, 1.5), obstacles: vec![] };
        assert!(fill(&face, &params(Orientation::Best)).modules.is_empty());
    }
}
