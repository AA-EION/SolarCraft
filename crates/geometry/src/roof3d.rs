// SPDX-License-Identifier: GPL-3.0-or-later
//! Roof faces in 3D. A face is a plane with tilt β and facing azimuth A
//! (clockwise from north). Roof-plane coordinates: x along the eave, y up the
//! slope. World: x east, y north, z up.
//!   u (eave axis)  = (−cos A, sin A, 0)
//!   v (up-slope)   = (−sin A·cos β, −cos A·cos β, sin β)
//!   n = u × v      = outward normal (faces A, tilted up by 90° − β)
use crate::earcut::earcut;
use crate::layout::RoofFace;
use crate::poly::P;

pub type V3 = [f64; 3];

#[derive(Debug, Clone, Copy)]
pub struct Frame { pub origin: V3, pub u: V3, pub v: V3, pub n: V3 }

impl Frame {
    pub fn new(tilt_deg: f64, azimuth_deg: f64, eave_height: f64) -> Self {
        let (b, a) = (tilt_deg.to_radians(), azimuth_deg.to_radians());
        let u = [-a.cos(), a.sin(), 0.0];
        let v = [-a.sin() * b.cos(), -a.cos() * b.cos(), b.sin()];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        Frame { origin: [0.0, 0.0, eave_height], u, v, n }
    }
    pub fn to_world(&self, p: P) -> V3 {
        [0, 1, 2].map(|k| self.origin[k] + self.u[k] * p[0] + self.v[k] * p[1])
    }
    /// Point on the face lifted `h` metres along the normal.
    pub fn above(&self, p: P, h: f64) -> V3 { let w = self.to_world(p); [0, 1, 2].map(|k| w[k] + self.n[k] * h) }
}

/// Obstacles as vertical prisms standing on the roof: base on the face, top
/// raised `height` vertically. Returns triangles with the obstacle index.
pub fn obstacle_triangles(face: &RoofFace, frame: &Frame) -> Vec<([V3; 3], usize)> {
    let mut out = vec![];
    for (i, ob) in face.obstacles.iter().enumerate() {
        if ob.len() < 3 { continue; }
        let h = face.obstacle_heights.get(i).copied().unwrap_or(1.0).max(0.0);
        let base: Vec<V3> = ob.iter().map(|&p| frame.to_world(p)).collect();
        let top: Vec<V3> = base.iter().map(|b| [b[0], b[1], b[2] + h]).collect();
        for t in earcut(ob, &[]) {
            out.push(([top[t[0]], top[t[1]], top[t[2]]], i));
            out.push(([base[t[0]], base[t[2]], base[t[1]]], i));
        }
        for k in 0..ob.len() {
            let j = (k + 1) % ob.len();
            out.push(([base[k], base[j], top[j]], i));
            out.push(([base[k], top[j], top[k]], i));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn south_facing_frame() {
        let f = Frame::new(30.0, 180.0, 3.0);
        let close = |a: V3, b: V3| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-12);
        assert!(close(f.u, [1.0, 0.0, 0.0]), "{:?}", f.u);
        assert!(close(f.v, [0.0, 30f64.to_radians().cos(), 0.5]), "{:?}", f.v);
        // Normal points south and up, tilted 30° from vertical.
        assert!(f.n[1] < 0.0 && (f.n[2] - 30f64.to_radians().cos()).abs() < 1e-12);
        // Ridge point 4 m up the slope is 2 m higher.
        assert!((f.to_world([0.0, 4.0])[2] - 5.0).abs() < 1e-12);
    }
    #[test]
    fn chimney_prism_is_closed() {
        let face = RoofFace { outline: vec![], obstacles: vec![vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]]], obstacle_heights: vec![1.5] };
        let tris = obstacle_triangles(&face, &Frame::new(20.0, 180.0, 3.0));
        assert_eq!(tris.len(), 2 + 2 + 8);
    }
}
