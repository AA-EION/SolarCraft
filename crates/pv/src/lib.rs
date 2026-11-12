// SPDX-License-Identifier: GPL-3.0-or-later
//! SolarCraft photovoltaic physics.
//!
//! Conventions: angles in degrees at the API boundary, azimuth clockwise from
//! north (N=0, E=90, S=180), irradiance in W/m², temperature in °C, time as
//! Unix seconds UTC. Every model cites its publication.
pub mod diode;
pub mod iam;
pub mod inverter;
pub mod irradiance;
pub mod spa;
mod spa_tables;
pub mod temperature;

pub(crate) fn sind(x: f64) -> f64 { x.to_radians().sin() }
pub(crate) fn cosd(x: f64) -> f64 { x.to_radians().cos() }

/// Day of year (1..=366) for a Unix time in UTC.
pub fn day_of_year_utc(unix_seconds: f64) -> u32 {
    let days = (unix_seconds / 86400.0).floor() as i64;
    let (y, _, _) = civil_from_days(days);
    (days - days_from_civil(y, 1, 1)) as u32 + 1
}

/// Howard Hinnant's civil-from-days algorithm (public domain).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
