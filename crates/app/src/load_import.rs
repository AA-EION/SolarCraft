// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{e, Result};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeDelta};

pub(crate) struct Series {
    pub values: Vec<f64>,
    pub step_minutes: u32,
    pub local_year: Option<i32>,
}

fn number(value: &str) -> Option<f64> {
    value.trim().trim_matches('"').replace(',', ".").parse().ok()
}

fn stamp(raw: &str, offset: TimeDelta) -> Option<(NaiveDateTime, bool)> {
    let value = raw.trim().trim_matches('"');
    if let Ok(time) = DateTime::parse_from_rfc3339(value) {
        return Some((time.naive_utc().checked_add_signed(offset)?, true));
    }
    for format in ["%Y-%m-%d %H:%M:%S%:z", "%Y-%m-%d %H:%M%:z"] {
        if let Ok(time) = DateTime::parse_from_str(value, format) {
            return Some((time.naive_utc().checked_add_signed(offset)?, true));
        }
    }
    ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"]
        .iter().find_map(|fmt| NaiveDateTime::parse_from_str(value, fmt).ok()).map(|time| (time, false))
}

pub(crate) fn parse(text: &str, unit: &str, utc_offset_h: f64) -> Result<Series> {
    let in_kw = match unit.trim().to_ascii_lowercase().as_str() {
        "kw" => true,
        "kwh" => false,
        _ => return Err(e("LOAD_UNIT", format!("Expected kW or kWh, received '{unit}'."))),
    };
    if !(-24.0..24.0).contains(&utc_offset_h) {
        return Err(e("LOAD_TIME", "The project has an invalid standard-time UTC offset."));
    }
    let offset = TimeDelta::seconds((utc_offset_h * 3600.0).round() as i64);
    let mut values = vec![];
    let mut times = vec![];
    let mut explicit_zone = None;
    let mut saw_header = false;
    for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let line_number = i + 1;
        let parse_error = |message: &str| e("LOAD_PARSE", format!("Line {line_number}. {message}"));
        let row = number(line).map(|value| (None, value)).or_else(|| {
            ['\t', ';', ','].into_iter().find_map(|delimiter| {
                let (time, value) = line.split_once(delimiter)?;
                Some((Some(stamp(time, offset)?), number(value)?))
            })
        });
        let Some((time, value)) = row else {
            let header = line.split(['\t', ';', ',']).next().unwrap_or(line).trim().trim_matches('"').to_ascii_lowercase();
            if values.is_empty() && !saw_header && matches!(header.as_str(), "timestamp" | "time" | "datetime" | "date" | "load" | "value" | "consumption" | "kw" | "kwh") {
                saw_header = true;
                continue;
            }
            return Err(parse_error("Expected a nonnegative value, optionally preceded by an ISO date and time."));
        };
        if !value.is_finite() || value < 0.0 { return Err(parse_error("Load must be finite and nonnegative.")); }
        if !values.is_empty() && time.is_some() == times.is_empty() {
            return Err(parse_error("Supply timestamps on every row or omit them on every row."));
        }
        if let Some((local, aware)) = time {
            if explicit_zone.replace(aware).is_some_and(|zone| zone != aware) {
                return Err(parse_error("Use an explicit time zone on every timestamp or on none of them."));
            }
            times.push((line_number, local));
        }
        values.push(value);
    }
    let step_minutes = match values.len() {
        8760 => 60, 17520 => 30, 35040 => 15, 105120 => 5, 525600 => 1,
        n => return Err(e("LOAD_PARSE", format!("{n} values. Expected one 365-day year at 60, 30, 15, 5 or 1-minute intervals."))),
    };
    let local_year = if let Some((_, first)) = times.first() {
        let year = first.year();
        let midnight = |y| NaiveDate::from_ymd_opt(y, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0));
        if Some(*first) != midnight(year) {
            return Err(e("LOAD_TIME", "The first interval must start on January 1 at 00:00 in the site's standard time."));
        }
        let step = TimeDelta::minutes(i64::from(step_minutes));
        for pair in times.windows(2) {
            if pair[1].1 - pair[0].1 != step {
                return Err(e("LOAD_TIME", format!("Lines {} and {} are not consecutive {step_minutes}-minute intervals in the site's standard time.", pair[0].0, pair[1].0)));
            }
        }
        let end = times.last().and_then(|(_, time)| time.checked_add_signed(step));
        if end != midnight(year + 1) {
            return Err(e("LOAD_TIME", "Timestamped data must cover a full non-leap calendar year in the site's standard time."));
        }
        Some(year)
    } else { None };
    if in_kw {
        let hours = f64::from(step_minutes) / 60.0;
        for value in &mut values { *value *= hours; }
    }
    Ok(Series { values, step_minutes, local_year })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamped_load_checks_local_alignment_and_interval_spacing() {
        use std::fmt::Write;
        let start = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().and_hms_opt(5, 0, 0).unwrap();
        let make = |gap| {
            let mut csv = String::from("timestamp;value\n");
            for interval in 0..8760 {
                let hour = start + TimeDelta::hours(interval + i64::from(interval == gap));
                writeln!(csv, "{}Z;1,25", hour.format("%Y-%m-%dT%H:%M:%S")).unwrap();
            }
            csv
        };
        let series = parse(&make(-1), "kWh", -5.0).unwrap();
        assert_eq!(series.local_year, Some(2025));
        assert_eq!(series.step_minutes, 60);
        assert_eq!(series.values.iter().sum::<f64>(), 10950.0);
        assert_eq!(parse(&make(5), "kWh", -5.0).err().unwrap().code, "LOAD_TIME");
        assert_eq!(parse(&make(-1), "kWh", -4.0).err().unwrap().code, "LOAD_TIME");
        assert_eq!(parse("", "watts", 0.0).err().unwrap().code, "LOAD_UNIT");
        assert_eq!(parse(&"2\n".repeat(35040), "kW", 0.0).unwrap().values[0], 0.5);
    }
}
