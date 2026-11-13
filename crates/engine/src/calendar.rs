// SPDX-License-Identifier: GPL-3.0-or-later
//! Local-time calendar for consumption profiles and tariffs.
//!
//! Weather years (TMY) mix months from different years, so the calendar is a
//! fixed 365-day reference year in local standard time (no daylight saving):
//! 1 January is a Wednesday (as in 2025) and 29 February does not exist. Each
//! weather hour is mapped by its UTC month/day/hour plus the site's UTC
//! offset, which keeps the mapping correct for mixed-year TMY files.
use solarcraft_pv::civil_from_days;

const CUM: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

/// Hour of the reference year in local standard time (0..8759) for a weather hour midpoint.
pub fn local_hour_of_year(unix_mid: i64, utc_offset_h: f64) -> u32 {
    let start = unix_mid - 1800;
    let (_, m, d) = civil_from_days(start.div_euclid(86400));
    let d = d.min(if m == 2 { 28 } else { 31 });
    let h = start.rem_euclid(86400) / 3600;
    let utc_hoy = (CUM[m as usize - 1] + d - 1) as i64 * 24 + h;
    (utc_hoy + utc_offset_h.round() as i64).rem_euclid(8760) as u32
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalTime { pub month: u32, pub day_of_year: u32, pub hour: u32, /// 0 = Monday
    pub weekday: u32 }

pub fn local_time(hoy: u32) -> LocalTime {
    let doy = hoy / 24;
    let month = (0..12).rev().find(|&i| CUM[i] <= doy).unwrap_or(0) as u32 + 1;
    // 1 January of the reference year is a Wednesday (weekday 2).
    LocalTime { month, day_of_year: doy, hour: hoy % 24, weekday: (doy + 2) % 7 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping() {
        // 2025-01-01 00:30 UTC → hour 0; Bogotá (UTC−5) → 19:00 on 31 Dec (hour 8755).
        let t = 1735689600 + 1800;
        assert_eq!(local_hour_of_year(t, 0.0), 0);
        assert_eq!(local_hour_of_year(t, -5.0), 8755);
        // A March hour taken from a leap year maps to the same day of the reference year.
        let mar1_2020 = 1583020800 + 1800; // 2020-03-01 00:30 UTC
        assert_eq!(local_hour_of_year(mar1_2020, 0.0), 59 * 24);
        assert_eq!(local_time(59 * 24).month, 3);
        assert_eq!(local_time(0).weekday, 2);
    }
}
