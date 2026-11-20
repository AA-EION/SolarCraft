// SPDX-License-Identifier: GPL-3.0-or-later
//! Reconstruct WBV radiation components when an archive has no saved hourly DHI.

use solarcraft_pv::irradiance::{dni_extra, erbs};
use std::f64::consts::TAU;

#[derive(Clone, Copy)]
pub(super) enum Model {
    Erbs,
    ReindlReduced,
    TrackedReindlFallback,
    HofmannHourly,
    HofmannFallback,
}

impl Model {
    pub(super) fn for_saved(name: Option<&str>, tracking: bool) -> Self {
        match (name, tracking) {
            (Some("ReindlReduced"), true) => Self::TrackedReindlFallback,
            (Some("ReindlReduced"), false) => Self::ReindlReduced,
            (Some("Hofmann"), _) => Self::HofmannFallback,
            _ => Self::Erbs,
        }
    }

    pub(super) fn hourly_hofmann(self) -> Self {
        match self {
            Self::HofmannFallback => Self::HofmannHourly,
            _ => self,
        }
    }

    pub(super) fn source(self) -> &'static str {
        match self {
            Self::Erbs => "erbs_from_wbv_global",
            Self::ReindlReduced => "reindl_reduced_from_wbv_global",
            Self::TrackedReindlFallback => "erbs_fallback_for_tracked_reindl",
            Self::HofmannHourly => "hofmann_hourly_base_from_wbv_global",
            Self::HofmannFallback => "erbs_fallback_for_hofmann",
        }
    }

    pub(super) fn split_with_neighbors(self, ghi: f64, zenith_deg: f64, day: u32, adjacent_dark: bool) -> (f64, f64) {
        let sky_factor = match self {
            Self::ReindlReduced => 0.81,
            Self::HofmannHourly => 0.78,
            Self::Erbs | Self::HofmannFallback | Self::TrackedReindlFallback => return erbs(ghi, zenith_deg, dni_extra(f64::from(day), 1366.1)),
        };
        let sine = zenith_deg.to_radians().cos();
        if ghi <= 0.0 || sine < 0.08 {
            return (0.0, ghi);
        }
        let extra = 1367.0 * (1.0 + 0.033 * (TAU * f64::from(day) / 365.0).cos());
        let clearness = ghi / (sky_factor * extra * sine.powf(1.15));
        let fraction = match self {
            Self::HofmannHourly => hofmann_hourly_fraction(clearness.min(if adjacent_dark { 1.0 } else { 1.5 })),
            _ => reindl_reduced_fraction(clearness, sine),
        };
        let dhi = ghi * fraction;
        ((ghi - dhi) / sine, dhi)
    }
}

// The reduced Reindl option uses Bourges clear-sky irradiance for its ratio.
fn reindl_reduced_fraction(clearness: f64, sine: f64) -> f64 {
    let fraction = if clearness <= 0.3 {
        1.02 - 0.254 * clearness + 0.0123 * sine
    } else if clearness <= 0.78 {
        (1.4 - 1.749 * clearness + 0.177 * sine).clamp(0.1, 0.97)
    } else {
        (0.486 * clearness - 0.182 * sine).max(0.1)
    };
    fraction.clamp(0.0, 1.0)
}

// For hourly inputs the transition and air-mass terms have zero weight. The
// optional aerosol and water-vapor factors require climatology grids.
fn hofmann_hourly_fraction(clearness: f64) -> f64 {
    (((((9.2921 * clearness - 28.891) * clearness + 35.541) * clearness - 22.029) * clearness + 5.918)
        * clearness * clearness - 0.5961 * clearness + 1.0008).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_reindl_applies_clear_sky_envelope_and_conserves_horizontal_irradiance() {
        // At 60° zenith on day 172, Bourges yields 482.766234 W/m².
        let (dni, dhi) = Model::ReindlReduced.split_with_neighbors(300.0, 60.0, 172, false);
        assert!((dhi - 120.491_571).abs() < 1e-5);
        assert!((dni * 0.5 + dhi - 300.0).abs() < 1e-10);
        assert_eq!(Model::ReindlReduced.split_with_neighbors(100.0, 87.0, 172, false), (0.0, 100.0));
    }

    #[test]
    fn hourly_hofmann_limits_the_first_and_last_daylight_hours() {
        let model = Model::for_saved(Some("Hofmann"), false).hourly_hofmann();
        let (dni, dhi) = model.split_with_neighbors(900.0, 60.0, 172, true);
        // The first daylight interval caps the polynomial input at 1.0.
        assert!((dhi - 212.22).abs() < 1e-8);
        assert!((dni * 0.5 + dhi - 900.0).abs() < 1e-9);
        assert_ne!(model.split_with_neighbors(900.0, 60.0, 172, false), (dni, dhi));
    }
}
