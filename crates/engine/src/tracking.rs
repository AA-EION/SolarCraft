// SPDX-License-Identifier: GPL-3.0-or-later
use solarcraft_model::tracking::Tracker;
use solarcraft_pv::spa::SolarPosition;
use std::f64::consts::PI;

pub(crate) fn plane(tracker: Tracker, sun: &SolarPosition, fixed_tilt: f64, fixed_azimuth: f64) -> (f64, f64) {
    if sun.apparent_zenith >= 90.0 { return (fixed_tilt, fixed_azimuth); }
    let azimuth = |center: f64, travel: f64| {
        let error = (sun.azimuth - center + 180.0_f64).rem_euclid(360.0) - 180.0;
        (center + error.clamp(-travel * 0.5, travel * 0.5)).rem_euclid(360.0)
    };
    match tracker {
        Tracker::SingleAxis { axis_azimuth_deg, axis_tilt_deg, max_rotation_deg, backtracking_gcr } => {
            let (sa, ca) = axis_azimuth_deg.to_radians().sin_cos();
            let (st, ct) = axis_tilt_deg.to_radians().sin_cos();
            let (sz, cz) = sun.apparent_zenith.to_radians().sin_cos();
            let relative = (sun.azimuth - axis_azimuth_deg).to_radians();
            // Project the sun onto the plane perpendicular to the shaft.
            let lateral = sz * relative.sin();
            let upright = sz * relative.cos() * st + cz * ct;
            let limit = max_rotation_deg.to_radians();
            let ideal = lateral.atan2(upright).clamp(-limit, limit);
            let rotation = backtracking_gcr.map_or(ideal, |gcr| {
                let elevation = upright.clamp(-1.0, 1.0).asin();
                if elevation <= 0.0 { return 0.0; }
                let angle = ideal.abs();
                let threshold = (gcr * angle.sin()).atan2(1.0 - gcr * angle.cos());
                if elevation < threshold {
                    let clear = (upright / gcr).min(1.0).asin() - elevation;
                    clear.clamp(0.0, angle).copysign(ideal)
                } else { ideal }
            });
            let (r, n) = rotation.sin_cos();
            let east = sa * st * n + ca * r;
            let north = ca * st * n - sa * r;
            ((ct * n).clamp(0.0, 1.0).acos().to_degrees(),
                east.atan2(north).to_degrees().rem_euclid(360.0))
        }
        Tracker::VerticalAxis { center_azimuth_deg, azimuth_travel_deg } =>
            (fixed_tilt, azimuth(center_azimuth_deg, azimuth_travel_deg)),
        Tracker::DualAxis { center_azimuth_deg, max_tilt_deg, azimuth_travel_deg } =>
            (sun.apparent_zenith.clamp(0.0, max_tilt_deg), azimuth(center_azimuth_deg, azimuth_travel_deg)),
    }
}

/// Diffuse IAM interpolated at one-degree tilt points. Integrate the exact
/// sky/ground azimuth share at each zenith knot instead of sampling two axes.
pub(crate) struct DiffuseIam([[f64; 2]; 91]);

impl DiffuseIam {
    pub fn new(iam: impl Fn(f64) -> f64) -> Self {
        let samples: Vec<_> = (0..360).map(|i| {
            let theta = (f64::from(i) + 0.5) * PI / 720.0;
            let (s, c) = theta.sin_cos();
            (c / s, s * c, iam(theta.to_degrees()))
        }).collect();
        Self(std::array::from_fn(|tilt| {
            let (s, c) = (tilt as f64).to_radians().sin_cos();
            let (mut sum, mut weights) = ([0.0; 2], [0.0; 2]);
            for &(cot, weight, modifier) in &samples {
                let sky = if tilt == 0 { 1.0 } else { 1.0 - (cot * c / s).clamp(-1.0, 1.0).acos() / PI };
                for (i, fraction) in [sky, 1.0 - sky].into_iter().enumerate() {
                    sum[i] += weight * fraction * modifier;
                    weights[i] += weight * fraction;
                }
            }
            std::array::from_fn(|i| if weights[i] > 0.0 { sum[i] / weights[i] } else { 0.0 })
        }))
    }

    pub fn at(&self, tilt: f64) -> (f64, f64) {
        let first = tilt.floor() as usize;
        let x = tilt - first as f64;
        let a = self.0[first];
        let b = self.0[(first + 1).min(90)];
        (a[0] + x * (b[0] - a[0]), a[1] + x * (b[1] - a[1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun(zenith: f64, azimuth: f64) -> SolarPosition {
        SolarPosition { zenith, azimuth, apparent_zenith: zenith, apparent_elevation: 90.0 - zenith, equation_of_time: 0.0 }
    }

    #[test]
    fn tracking_moves_with_the_sun_and_obeys_mechanical_limits() {
        let one = |backtracking_gcr| Tracker::SingleAxis {
            axis_azimuth_deg: 180.0, axis_tilt_deg: 0.0, max_rotation_deg: 70.0, backtracking_gcr,
        };
        let low_east = sun(85.0, 90.0);
        let (tilt, az) = plane(one(None), &low_east, 25.0, 180.0);
        assert!((tilt - 70.0).abs() < 1e-9 && (az - 90.0).abs() < 1e-9);
        let (backtracked, az) = plane(one(Some(0.4)), &low_east, 25.0, 180.0);
        assert!((backtracked - 7.5851).abs() < 0.001 && (az - 90.0).abs() < 1e-9, "{backtracked}, {az}");
        let (tilt, az) = plane(one(None), &sun(42.0, 270.0), 25.0, 180.0);
        assert!((tilt - 42.0).abs() < 1e-9 && (az - 270.0).abs() < 1e-9);
        let two = Tracker::DualAxis { center_azimuth_deg: 180.0, max_tilt_deg: 65.0, azimuth_travel_deg: 200.0 };
        assert_eq!(plane(two, &sun(80.0, 60.0), 25.0, 180.0), (65.0, 80.0));
        assert_eq!(plane(two, &sun(105.0, 60.0), 25.0, 180.0), (25.0, 180.0));
        let vertical = Tracker::VerticalAxis { center_azimuth_deg: 0.0, azimuth_travel_deg: 180.0 };
        assert_eq!(plane(vertical, &sun(40.0, 250.0), 47.0, 0.0), (47.0, 270.0));
    }
}
