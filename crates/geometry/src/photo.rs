// SPDX-License-Identifier: GPL-3.0-or-later
//! Calibrate a surveyed rectangle on a plane from four image correspondences.
use crate::poly::P;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Control {
    /// Normalized image points, ordered eave-left, eave-right, upper-right, upper-left.
    pub corners: [P; 4],
    pub width_m: f64,
    pub depth_m: f64,
}

type Matrix = [[f64; 3]; 3];

pub struct Calibration {
    forward: Matrix,
    inverse: Matrix,
    extent: P,
}

fn cross(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
}

fn apply(m: &Matrix, [x, y]: P) -> Option<P> {
    let row = |i: usize| m[i][0] * x + m[i][1] * y + m[i][2];
    let w = row(2);
    if !w.is_finite() || w.abs() < 1e-10 { return None; }
    let p = [row(0) / w, row(1) / w];
    p.iter().all(|x| x.is_finite()).then_some(p)
}

impl Calibration {
    pub fn new(control: &Control) -> Result<Self, &'static str> {
        let p = control.corners;
        if !p.iter().flatten().all(|v| (0.0..=1.0).contains(v))
            || ![control.width_m, control.depth_m].iter().all(|v| v.is_finite() && *v > 0.0) {
            return Err("Coordinates must be within the image and dimensions must be positive.");
        }
        let signs = [0, 1, 2, 3].map(|i| cross(p[i], p[(i + 1) % 4], p[(i + 2) % 4]));
        if signs.iter().any(|&v| v.abs() < 1e-7 || v.signum() != signs[0].signum()) {
            return Err("The four corners must form a convex rectangle in the stated order.");
        }
        let [a, b, c, d] = p;
        let x1 = b[0] - c[0];
        let x2 = d[0] - c[0];
        let y1 = b[1] - c[1];
        let y2 = d[1] - c[1];
        let sx = a[0] - b[0] + c[0] - d[0];
        let sy = a[1] - b[1] + c[1] - d[1];
        let denominator = x1 * y2 - x2 * y1;
        let g = (sx * y2 - sy * x2) / denominator;
        let h = (x1 * sy - y1 * sx) / denominator;
        if [1.0 + g, 1.0 + h, 1.0 + g + h].iter().any(|w| !w.is_finite() || *w <= 1e-6) {
            return Err("The calibration crosses the projective horizon.");
        }
        let forward = [
            [b[0] * (1.0 + g) - a[0], d[0] * (1.0 + h) - a[0], a[0]],
            [b[1] * (1.0 + g) - a[1], d[1] * (1.0 + h) - a[1], a[1]],
            [g, h, 1.0],
        ];
        // Gauss–Jordan with row pivoting preserves the inverse for sloped views.
        let mut rows = [[0.0; 6]; 3];
        for (i, (row, m)) in rows.iter_mut().zip(forward).enumerate() {
            row[..3].copy_from_slice(&m);
            row[3 + i] = 1.0;
        }
        for i in 0..3 {
            let pivot = (i..3).max_by(|&a, &b| rows[a][i].abs().total_cmp(&rows[b][i].abs())).unwrap();
            rows.swap(i, pivot);
            let scale = rows[i][i];
            if scale.abs() < 1e-10 { return Err("The reference rectangle cannot be inverted."); }
            for entry in &mut rows[i] { *entry /= scale; }
            let normalized = rows[i];
            for (_, row) in rows.iter_mut().enumerate().filter(|(j, _)| *j != i) {
                let factor = row[i];
                for (entry, basis) in row.iter_mut().zip(normalized) { *entry -= factor * basis; }
            }
        }
        let inverse = rows.map(|row| [row[3], row[4], row[5]]);
        Ok(Self { forward, inverse, extent: [control.width_m, control.depth_m] })
    }

    pub fn image_at(&self, [x, y]: P) -> Option<P> {
        apply(&self.forward, [x / self.extent[0], y / self.extent[1]])
    }

    pub fn plane_at(&self, p: P) -> Option<P> {
        let [x, y] = apply(&self.inverse, p)?;
        Some([x * self.extent[0], y * self.extent[1]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrates_known_perspective_and_rejects_degenerate_corners() {
        let known = [[0.65, 0.16, 0.12], [-0.08, -0.61, 0.92], [0.24, 0.31, 1.0]];
        let plane = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let mut control = Control { corners: plane.map(|p| apply(&known, p).unwrap()), width_m: 10.0, depth_m: 5.0 };
        let c = Calibration::new(&control).unwrap();
        let image = apply(&known, [0.37, 0.62]).unwrap();
        let actual = c.image_at([3.7, 3.1]).unwrap();
        assert!((actual[0] - image[0]).abs() < 1e-10 && (actual[1] - image[1]).abs() < 1e-10);
        let [x, y] = c.plane_at(image).unwrap();
        assert!((x - 3.7).abs() < 1e-9 && (y - 3.1).abs() < 1e-9);
        control.corners.swap(1, 2);
        assert!(Calibration::new(&control).is_err());
        control.corners = [[0.0, 0.0]; 4];
        assert!(Calibration::new(&control).is_err());
    }
}
