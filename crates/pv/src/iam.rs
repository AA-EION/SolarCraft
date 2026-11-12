// SPDX-License-Identifier: GPL-3.0-or-later
//! Incidence angle modifiers. Physical (Fresnel/Snell + absorption):
//! De Soto et al. (2006) after Duffie & Beckman; ASHRAE (Souka & Safwat 1966);
//! diffuse averages by integrating the beam IAM over the isotropic sky and
//! ground seen by a tilted plane (method of Marion 2017, NREL/TP-5K00-68433).
use std::f64::consts::PI;

/// Physical IAM for angle of incidence `aoi` (deg). Defaults: n=1.526, K=4 /m, L=0.002 m.
pub fn physical(aoi: f64, n: f64, k: f64, l: f64) -> f64 {
    if aoi.abs() >= 90.0 { return 0.0; }
    let tau = |th: f64| -> f64 {
        let thr = (th.sin() / n).asin();
        let absorb = (-k * l / thr.cos()).exp();
        if th.abs() < 1e-9 {
            let r = ((1.0 - n) / (1.0 + n)).powi(2);
            return absorb * (1.0 - r);
        }
        let rs = (thr - th).sin().powi(2) / (thr + th).sin().powi(2);
        let rp = (thr - th).tan().powi(2) / (thr + th).tan().powi(2);
        absorb * (1.0 - 0.5 * (rs + rp))
    };
    (tau(aoi.abs().to_radians()) / tau(0.0)).clamp(0.0, 1.0)
}

/// ASHRAE IAM: 1 − b0 (1/cos θ − 1), floored at 0. Typical b0 = 0.05.
pub fn ashrae(aoi: f64, b0: f64) -> f64 {
    if aoi.abs() >= 90.0 { return 0.0; }
    (1.0 - b0 * (1.0 / aoi.to_radians().cos() - 1.0)).clamp(0.0, 1.0)
}

/// Irradiance-weighted average of `iam(aoi)` over the part of the isotropic
/// sky (`sky=true`) or ground visible from a plane tilted by `tilt` degrees.
pub fn diffuse_average(iam: impl Fn(f64) -> f64, tilt: f64, sky: bool) -> f64 {
    // Integrate over the plane's hemisphere in plane-local spherical coordinates:
    // weight cosθ·sinθ dθ dφ; a direction is sky if its world elevation > 0.
    let (nt, np) = (360usize, 720usize);
    let b = tilt.to_radians();
    let (mut num, mut den) = (0.0, 0.0);
    for i in 0..nt {
        let th = (i as f64 + 0.5) / nt as f64 * PI / 2.0;
        let w_th = th.cos() * th.sin();
        let f = iam(th.to_degrees());
        for j in 0..np {
            let ph = (j as f64 + 0.5) / np as f64 * 2.0 * PI;
            // world z of direction (θ from plane normal, φ around it); normal tilted by b about x.
            let z = th.cos() * b.cos() - th.sin() * ph.cos() * b.sin();
            if (z > 0.0) == sky { num += w_th * f; den += w_th; }
        }
    }
    if den > 0.0 { num / den } else { 0.0 }
}
