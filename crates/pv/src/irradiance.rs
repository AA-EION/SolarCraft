// SPDX-License-Identifier: GPL-3.0-or-later
//! Plane-of-array irradiance. Sources: Liu & Jordan (1963); Hay & Davies
//! (1980); Klucher (1979); Reindl et al. (1990); Perez et al. (1990, Solar
//! Energy 44(5)); Kasten & Young (1989) air mass; Spencer (1971) Earth-Sun
//! distance; Erbs et al. (1982) decomposition. Edge-case conventions match
//! pvlib-python so it can serve as a validation oracle.
use crate::spa_tables::{PEREZ_F1, PEREZ_F2};
use crate::{cosd, sind};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkyModel { Isotropic, HayDavies, Klucher, Reindl, Perez }

impl SkyModel {
    pub const ALL: [SkyModel; 5] = [SkyModel::Isotropic, SkyModel::HayDavies, SkyModel::Klucher, SkyModel::Reindl, SkyModel::Perez];
    pub fn name(self) -> &'static str {
        match self { Self::Isotropic => "isotropic", Self::HayDavies => "haydavies", Self::Klucher => "klucher", Self::Reindl => "reindl", Self::Perez => "perez" }
    }
}

/// Extraterrestrial normal irradiance (Spencer 1971), W/m², day of year 1..366.
/// Day angle b = 2π(doy − 1)/365.
pub fn dni_extra(day_of_year: f64, solar_constant: f64) -> f64 {
    let b = 2.0 * PI * (day_of_year - 1.0) / 365.0;
    solar_constant * (1.00011 + 0.034221 * b.cos() + 0.00128 * b.sin() + 0.000719 * (2.0 * b).cos() + 0.000077 * (2.0 * b).sin())
}

/// Relative optical air mass (Kasten & Young 1989); NaN for zenith > 90°.
pub fn airmass_relative(zenith: f64) -> f64 {
    if zenith > 90.0 { return f64::NAN; }
    1.0 / (cosd(zenith) + 0.50572 * (6.07995 + (90.0 - zenith)).powf(-1.6364))
}

/// Cosine of the angle of incidence (may be negative).
pub fn aoi_projection(tilt: f64, surf_az: f64, zenith: f64, sun_az: f64) -> f64 {
    let p = cosd(tilt) * cosd(zenith) + sind(tilt) * sind(zenith) * cosd(sun_az - surf_az);
    p.clamp(-1.0, 1.0)
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Poa { pub global: f64, pub direct: f64, pub sky_diffuse: f64, pub ground_diffuse: f64 }

#[derive(Debug, Clone, Copy)]
pub struct Geometry { pub tilt: f64, pub surface_azimuth: f64, pub zenith: f64, pub sun_azimuth: f64 }

#[derive(Debug, Clone, Copy)]
pub struct Components { pub ghi: f64, pub dni: f64, pub dhi: f64, pub dni_extra: f64 }

pub fn sky_diffuse(model: SkyModel, g: &Geometry, c: &Components) -> f64 {
    let cos_tilt = cosd(g.tilt);
    let cos_tt = aoi_projection(g.tilt, g.surface_azimuth, g.zenith, g.sun_azimuth).max(0.0);
    let cos_z = cosd(g.zenith);
    let iso = 0.5 * (1.0 + cos_tilt);
    let v = match model {
        SkyModel::Isotropic => c.dhi * iso,
        SkyModel::HayDavies => {
            let rb = cos_tt / cos_z.max(0.01745);
            let ai = c.dni / c.dni_extra;
            (c.dhi * (1.0 - ai) * iso).max(0.0) + (c.dhi * ai * rb).max(0.0)
        }
        SkyModel::Klucher => {
            let f = if c.ghi != 0.0 { 1.0 - (c.dhi / c.ghi).powi(2) } else { 0.0 };
            let f = if f.is_finite() { f } else { 0.0 };
            c.dhi * iso * (1.0 + f * sind(g.tilt / 2.0).powi(3)) * (1.0 + f * cos_tt.powi(2) * sind(g.zenith).powi(3))
        }
        SkyModel::Reindl => {
            let rb = cos_tt / cos_z.max(0.01745);
            let ai = c.dni / c.dni_extra;
            let hb = (c.dni * cos_z).max(0.0);
            let t3 = if c.ghi > 0.0 { 1.0 + (hb / c.ghi).sqrt() * sind(0.5 * g.tilt).powi(3) } else { f64::NAN };
            let v = c.dhi * (ai * rb + (1.0 - ai) * iso * t3);
            if v.is_nan() { 0.0 } else { v.max(0.0) }
        }
        SkyModel::Perez => perez(g, c, cos_tt, cos_z),
    };
    if v.is_finite() { v } else { 0.0 }
}

fn perez(g: &Geometry, c: &Components, a: f64, cos_z: f64) -> f64 {
    let am = airmass_relative(g.zenith);
    if am.is_nan() || c.dhi <= 0.0 { return 0.0; }
    let kappa = 1.041;
    let z = g.zenith.to_radians();
    let delta = c.dhi * am / c.dni_extra;
    let eps = ((c.dhi + c.dni) / c.dhi + kappa * z.powi(3)) / (1.0 + kappa * z.powi(3));
    const EDGES: [f64; 7] = [1.065, 1.23, 1.5, 1.95, 2.8, 4.5, 6.2];
    let bin = EDGES.iter().take_while(|&&e| eps >= e).count();
    let (f1c, f2c) = (PEREZ_F1[bin], PEREZ_F2[bin]);
    let f1 = (f1c[0] + f1c[1] * delta + f1c[2] * z).max(0.0);
    let f2 = f2c[0] + f2c[1] * delta + f2c[2] * z;
    let b = cos_z.max(cosd(85.0));
    let t1 = 0.5 * (1.0 - f1) * (1.0 + cosd(g.tilt));
    (c.dhi * (t1 + f1 * a / b + f2 * sind(g.tilt))).max(0.0)
}

pub fn ground_diffuse(tilt: f64, ghi: f64, albedo: f64) -> f64 { ghi * albedo * (1.0 - cosd(tilt)) * 0.5 }

pub fn poa(model: SkyModel, g: &Geometry, c: &Components, albedo: f64) -> Poa {
    let direct = (c.dni * aoi_projection(g.tilt, g.surface_azimuth, g.zenith, g.sun_azimuth)).max(0.0);
    let sky = sky_diffuse(model, g, c);
    let ground = ground_diffuse(g.tilt, c.ghi, albedo);
    Poa { global: direct + sky + ground, direct, sky_diffuse: sky, ground_diffuse: ground }
}

/// Erbs et al. (1982) GHI → (DNI, DHI). `zenith` in degrees.
pub fn erbs(ghi: f64, zenith: f64, dni_extra: f64) -> (f64, f64) {
    let cz = cosd(zenith).max(0.065);
    let kt = (ghi / (dni_extra * cz)).clamp(0.0, 1.0);
    let df = if kt <= 0.22 { 1.0 - 0.09 * kt } else if kt <= 0.8 {
        0.9511 - 0.1604 * kt + 4.388 * kt.powi(2) - 16.638 * kt.powi(3) + 12.336 * kt.powi(4)
    } else { 0.165 };
    let dhi = df * ghi;
    let dni = if zenith < 87.0 { ((ghi - dhi) / cosd(zenith)).max(0.0) } else { 0.0 };
    (dni, dhi)
}
