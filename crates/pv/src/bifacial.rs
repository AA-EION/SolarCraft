// SPDX-License-Identifier: GPL-3.0-or-later
//! Rear irradiance for periodic, infinitely long parallel rows over a
//! horizontal Lambertian surface. Cosine-weighted view factors integrate the
//! visible sky and ground in the two-dimensional row cross-section.
use crate::irradiance::{Components, Geometry};

const GROUND_SAMPLES: usize = 96;
const ROW_SAMPLES: usize = 16;
const RAYS: usize = 96;
const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, Copy, Default)]
pub struct RearPoa {
    pub direct: f64,
    pub sky: f64,
    pub reflected: f64,
}

#[derive(Debug)]
pub struct Rows {
    pitch: f64,
    lower_edge: f64,
    run: f64,
    rise: f64,
    sin_tilt: f64,
    cos_tilt: f64,
    sky_view: f64,
    ground_view: [f64; GROUND_SAMPLES],
    ground_sky: f64,
}

impl Rows {
    /// The low edge is at x = 0 and the high edge lies in the +x direction.
    pub fn new(tilt_deg: f64, slant_width: f64, pitch: f64, lower_edge: f64) -> Option<Self> {
        if !(0.0..=90.0).contains(&tilt_deg) || !slant_width.is_finite() || !pitch.is_finite()
            || !lower_edge.is_finite() || pitch <= 0.0 || slant_width <= 0.0
            || lower_edge < 0.0 || slant_width * crate::cosd(tilt_deg) > pitch {
            return None;
        }
        let (sin_tilt, cos_tilt) = tilt_deg.to_radians().sin_cos();
        let mut rows = Self {
            pitch, lower_edge, run: slant_width * cos_tilt, rise: slant_width * sin_tilt,
            sin_tilt, cos_tilt, sky_view: 0.0, ground_view: [0.0; GROUND_SAMPLES], ground_sky: 0.0,
        };
        let weight = 1.0 / (ROW_SAMPLES * RAYS) as f64;
        for sensor in 0..ROW_SAMPLES {
            let origin = rows.row_point(sensor);
            for ray in 0..RAYS {
                let (tangent, normal) = hemisphere(ray);
                let dx = sin_tilt * normal + cos_tilt * tangent;
                let dz = -cos_tilt * normal + sin_tilt * tangent;
                if rows.intercepts_row(origin, dx, dz) { continue; }
                if dz > 0.0 {
                    rows.sky_view += weight;
                } else if dz < -EPSILON {
                    let ground_x = origin.0 - origin.1 * dx / dz;
                    let index = ((ground_x.rem_euclid(pitch) / pitch) * GROUND_SAMPLES as f64) as usize;
                    rows.ground_view[index.min(GROUND_SAMPLES - 1)] += weight;
                }
            }
        }
        for (i, &view) in rows.ground_view.iter().enumerate().filter(|(_, view)| **view > 0.0) {
            let x = rows.ground_x(i);
            let visible = (0..RAYS).filter(|&ray| {
                let (dx, dz) = hemisphere(ray);
                !rows.intercepts_row((x, 0.0), dx, dz)
            }).count();
            rows.ground_sky += view * visible as f64 / RAYS as f64;
        }
        Some(rows)
    }

    pub fn irradiance(&self, sun: &Geometry, light: &Components, albedo: f64) -> RearPoa {
        let z = crate::cosd(sun.zenith).max(0.0);
        let x = -crate::sind(sun.zenith) * crate::cosd(sun.sun_azimuth - sun.surface_azimuth);
        let rear_cosine = (self.sin_tilt * x - self.cos_tilt * z).max(0.0);
        let horizontal_beam = if z > EPSILON { light.dni.max(0.0) * z } else { 0.0 };
        let diffuse = light.dhi.max(0.0);
        let direct = if rear_cosine > 0.0 && horizontal_beam > 0.0 {
            let visible = (0..ROW_SAMPLES).filter(|&i| !self.intercepts_row(self.row_point(i), x, z)).count();
            light.dni * rear_cosine * visible as f64 / ROW_SAMPLES as f64
        } else { 0.0 };
        let sunlit_ground_view: f64 = if horizontal_beam > 0.0 {
            self.ground_view.iter().enumerate().filter(|(i, w)| {
                **w > 0.0 && !self.intercepts_row((self.ground_x(*i), 0.0), x, z)
            }).map(|(_, weight)| weight).sum()
        } else { 0.0 };
        RearPoa {
            direct,
            sky: diffuse * self.sky_view,
            reflected: albedo * (diffuse * self.ground_sky + horizontal_beam * sunlit_ground_view),
        }
    }

    fn row_point(&self, index: usize) -> (f64, f64) {
        let t = (index as f64 + 0.5) / ROW_SAMPLES as f64;
        (self.run * t, self.lower_edge + self.rise * t)
    }

    fn ground_x(&self, index: usize) -> f64 {
        self.pitch * (index as f64 + 0.5) / GROUND_SAMPLES as f64
    }

    // At a given height the ray has a unique x. A hit occurs if the
    // interval of that x minus the panel's x spans a translate k*pitch.
    fn intercepts_row(&self, (x, z): (f64, f64), dx: f64, dz: f64) -> bool {
        if dz.abs() <= EPSILON { return false; }
        let mut lo = self.lower_edge;
        let mut hi = self.lower_edge + self.rise;
        if dz > 0.0 { lo = lo.max(z + EPSILON); } else { hi = hi.min(z - EPSILON); }
        if lo > hi { return false; }
        let slope = dx / dz;
        if self.rise <= EPSILON {
            let hit = (x + (lo - z) * slope).rem_euclid(self.pitch);
            return hit <= self.run;
        }
        let row_translate = |height: f64| {
            (x + (height - z) * slope - (height - self.lower_edge) * self.run / self.rise) / self.pitch
        };
        let (a, b) = (row_translate(lo), row_translate(hi));
        a.min(b).ceil() <= a.max(b).floor()
    }
}

// Equal increments in sine of angle are equal cosine-weighted solid angles
// after integrating out the dimension along the rows.
fn hemisphere(index: usize) -> (f64, f64) {
    let tangent = 2.0 * (index as f64 + 0.5) / RAYS as f64 - 1.0;
    (tangent, (1.0 - tangent * tangent).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unobstructed_rear_and_ground_respond_to_weather_and_albedo() {
        let rows = Rows::new(60.0, 1.0, 500.0, 1.0).unwrap();
        let sun = Geometry { tilt: 60.0, surface_azimuth: 180.0, zenith: 55.0, sun_azimuth: 0.0 };
        let light = Components { ghi: 640.0, dni: 800.0, dhi: 150.0, dni_extra: 1366.1 };
        let rear = rows.irradiance(&sun, &light, 0.0);
        assert!((rear.direct - light.dni * -crate::cosd(115.0)).abs() < 1e-9);
        assert!((rear.sky - 150.0 * 0.25).abs() < 3.0);
        assert_eq!(rear.reflected, 0.0);
        let reflected = rows.irradiance(&sun, &light, 0.2).reflected;
        assert!(reflected > 0.0);
        assert!((rows.irradiance(&sun, &light, 0.4).reflected - 2.0 * reflected).abs() < 1e-9);
        let tight = Rows::new(60.0, 1.0, 1.15, 0.2).unwrap();
        assert!(tight.irradiance(&sun, &light, 0.2).reflected < reflected);
        let grazing = Geometry { zenith: 80.0, ..sun };
        assert!(tight.irradiance(&grazing, &light, 0.0).direct < rows.irradiance(&grazing, &light, 0.0).direct);
        let dark = rows.irradiance(&sun, &Components { ghi: 0.0, dni: 0.0, dhi: 0.0, ..light }, 0.2);
        assert_eq!(dark.direct + dark.sky + dark.reflected, 0.0);
    }

    #[test]
    fn tightly_packed_horizontal_rows_have_no_rear_light() {
        let rows = Rows::new(0.0, 2.0, 2.0, 0.3).unwrap();
        let sun = Geometry { tilt: 0.0, surface_azimuth: 180.0, zenith: 30.0, sun_azimuth: 180.0 };
        let light = Components { ghi: 798.0, dni: 800.0, dhi: 105.0, dni_extra: 1366.1 };
        let rear = rows.irradiance(&sun, &light, 0.2);
        assert_eq!(rear.direct + rear.sky + rear.reflected, 0.0);
        assert!(Rows::new(0.0, 2.1, 2.0, 0.3).is_none());
    }
}
