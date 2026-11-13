// SPDX-License-Identifier: GPL-3.0-or-later
//! Consumption profiles on the local reference calendar.
//!
//! The synthetic shapes are SolarCraft's own parametric profiles (daily shape
//! × weekday/weekend × a seasonal cosine outside the tropics), scaled to the
//! annual total. They are illustrative defaults, not a published standard
//! load profile; imported metered series should be used for real designs.
use crate::calendar::local_time;
use solarcraft_model::plant::SyntheticProfile;

const RES_WD: [f64; 24] = [0.45, 0.40, 0.38, 0.37, 0.38, 0.50, 0.85, 1.10, 0.95, 0.75, 0.70, 0.72, 0.80, 0.75, 0.70, 0.75, 0.90, 1.25, 1.55, 1.60, 1.45, 1.20, 0.90, 0.60];
const RES_WE: [f64; 24] = [0.50, 0.42, 0.40, 0.38, 0.38, 0.42, 0.55, 0.80, 1.05, 1.15, 1.10, 1.10, 1.15, 1.05, 0.95, 0.95, 1.05, 1.30, 1.55, 1.55, 1.40, 1.20, 0.95, 0.65];

fn shape(p: SyntheticProfile, hour: u32, weekday: u32) -> f64 {
    let h = hour as usize;
    let weekend = weekday >= 5;
    match p {
        SyntheticProfile::Residential => if weekend { RES_WE[h] } else { RES_WD[h] },
        SyntheticProfile::Office => if weekend { 0.25 } else { match h { 0..=5 => 0.25, 6 => 0.4, 7 => 0.8, 8..=11 => 1.6, 12 => 1.4, 13..=16 => 1.6, 17 => 1.0, 18 => 0.6, _ => 0.3 } },
        SyntheticProfile::Retail => if weekday == 6 { 0.3 } else { match h { 0..=6 => 0.3, 7 | 8 => 0.8, 9..=19 => 1.5, 20 => 0.8, _ => 0.3 } },
        SyntheticProfile::Flat => 1.0,
    }
}

/// Hourly consumption (Wh) for the 8760 local hours of the reference year.
pub fn synthetic(p: SyntheticProfile, annual_kwh: f64, latitude: f64) -> Vec<f64> {
    let peak_doy = if latitude >= 0.0 { 15.0 } else { 196.0 };
    let amp = if latitude.abs() > 23.5 && p != SyntheticProfile::Flat { 0.15 } else { 0.0 };
    let raw: Vec<f64> = (0..8760u32).map(|hoy| {
        let t = local_time(hoy);
        shape(p, t.hour, t.weekday) * (1.0 + amp * (std::f64::consts::TAU * (t.day_of_year as f64 - peak_doy) / 365.0).cos())
    }).collect();
    let sum: f64 = raw.iter().sum();
    raw.iter().map(|x| x / sum * annual_kwh * 1000.0).collect()
}

/// Aggregate an interval series (kWh per interval) to 8760 hourly Wh values.
pub fn hourly_from_series(values: &[f64]) -> Result<Vec<f64>, String> {
    let per_hour = match values.len() { 8760 => 1, 17520 => 2, 35040 => 4, 105120 => 12, 525600 => 60,
        n => return Err(format!("{n} values: expected one year at 60, 30, 15, 5 or 1 minute steps (8760, 17520, 35040, 105120 or 525600 values, no leap day)")) };
    Ok(values.chunks(per_hour).map(|c| c.iter().sum::<f64>() * 1000.0).collect())
}
