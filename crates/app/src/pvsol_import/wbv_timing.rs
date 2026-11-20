// SPDX-License-Identifier: GPL-3.0-or-later
//! Infer the representative time of a WBV irradiance row without using saved PV production.

const HOURS_PER_DAY: usize = 24;
const DAYS: usize = 365;

fn clear_sky(day: usize, zenith: f64) -> f64 {
    let solar_constant = 1367.0 * (1.0 + 0.033 * (std::f64::consts::TAU * (day + 1) as f64 / DAYS as f64).cos());
    // Bourges clear-sky envelope on the horizontal plane.
    0.81 * solar_constant * zenith.to_radians().cos().max(0.0).powf(1.15)
}

/// Returns whether the weather represents interval midpoints when the phase is clear.
/// Warm-up, saved PV yields and daily cloud timing are not inputs to this decision.
pub(super) fn infer_midpoint(ghi: &[f64], hour_mark_zeniths: &[f64]) -> Option<bool> {
    if ghi.len() != DAYS * HOURS_PER_DAY || hour_mark_zeniths.len() != ghi.len()
        || !ghi.iter().all(|v| (0.0..=1800.0).contains(v))
        || !hour_mark_zeniths.iter().all(|v| (0.0..=180.0).contains(v)) {
        return None;
    }

    let mut phases = Vec::new();
    let mut year_segments = [0_u8; 12];
    for (day, (radiation, zeniths)) in ghi.chunks_exact(HOURS_PER_DAY)
        .zip(hour_mark_zeniths.chunks_exact(HOURS_PER_DAY)).enumerate() {
        let (mut observed, mut clear, mut observed_moment, mut clear_moment) = (0.0, 0.0, 0.0, 0.0);
        for (hour, (&ghi, &zenith)) in radiation.iter().zip(zeniths).enumerate() {
            let expected = clear_sky(day, zenith);
            observed += ghi;
            clear += expected;
            observed_moment += hour as f64 * ghi;
            clear_moment += hour as f64 * expected;
        }
        // A strong near-clear envelope provides a usable estimate of solar noon.
        if clear >= 1000.0 && (0.90..=1.25).contains(&(observed / clear)) {
            phases.push((observed_moment / observed - clear_moment / clear) * 60.0);
            year_segments[day * year_segments.len() / DAYS] += 1;
        }
    }

    if phases.len() < 16 || year_segments.iter().filter(|&&count| count >= 2).count() < 3 {
        return None;
    }
    phases.sort_by(f64::total_cmp);
    let phase = phases[phases.len() / 2];
    let nearest = if phase.abs() <= 7.5 { 0.0 }
        else if (phase + 30.0).abs() <= 7.5 { -30.0 }
        else { return None };

    // Require agreement across qualified days. -30 min in row indices means that
    // the sun reaches its peak half an interval after the nominal hour mark.
    let close = phases.iter().filter(|&&p| (p - nearest).abs() <= 10.0).count();
    (close * 4 >= phases.len() * 3).then_some(nearest < 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solarcraft_pv::{days_from_civil, spa::{delta_t_estimate, solar_position, Observer}};

    #[test]
    fn identifies_both_wbv_time_conventions_and_rejects_ambiguous_data() {
        let observer = Observer::new(37.62, -122.38, 4.0);
        let start = days_from_civil(2025, 1, 1) * 86400 + 8 * 3600;
        let zeniths = |offset| (0..DAYS * HOURS_PER_DAY).map(|k|
            solar_position((start + k as i64 * 3600 + offset) as f64, &observer, delta_t_estimate(2025.0)).zenith
        ).collect::<Vec<_>>();
        let marks = zeniths(0);
        let radiation = |zs: &[f64]| zs.iter().enumerate().map(|(k, &z)| clear_sky(k / HOURS_PER_DAY, z)).collect::<Vec<_>>();
        assert_eq!(infer_midpoint(&radiation(&marks), &marks), Some(false));
        assert_eq!(infer_midpoint(&radiation(&zeniths(1800)), &marks), Some(true));
        assert_eq!(infer_midpoint(&radiation(&zeniths(900)), &marks), None);
        assert_eq!(infer_midpoint(&vec![0.0; marks.len()], &marks), None);
    }
}
