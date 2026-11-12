// SPDX-License-Identifier: GPL-3.0-or-later
//! Hourly weather series with provenance.
//!
//! Time convention (assumption A-08): each record carries the UTC Unix time
//! at which its irradiance is representative (`unix_mid`), i.e. the interval
//! midpoint for hourly means. Importers convert their source conventions.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hour {
    /// UTC Unix seconds at which irradiance is representative (interval midpoint).
    pub unix_mid: i64,
    pub ghi: f64,
    pub dni: f64,
    pub dhi: f64,
    /// °C
    pub temp_air: f64,
    /// m/s at 10 m
    pub wind: f64,
    /// Pa (NaN if unknown)
    pub pressure: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub provider: String,
    pub dataset: String,
    pub url: String,
    pub retrieved_unix: i64,
    pub licence: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Weather {
    pub latitude: f64,
    pub longitude: f64,
    pub elevation: f64,
    /// True if direct irradiance already includes far (horizon) shading.
    pub horizon_applied: bool,
    pub provenance: Provenance,
    pub hours: Vec<Hour>,
}

#[derive(Debug)]
pub struct WeatherError(pub String);
impl std::fmt::Display for WeatherError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) } }
impl std::error::Error for WeatherError {}
fn err<T>(m: impl Into<String>) -> Result<T, WeatherError> { Err(WeatherError(m.into())) }

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 { solarcraft_pv::days_from_civil(y, m, d) }
fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

pub const PVGIS_LICENCE_NOTE: &str = "PVGIS © European Union, 2001–2026. Reuse per Commission Decision 2011/833/EU; cite the source. Terms to be confirmed (provider register PRV-01).";

/// Parse a PVGIS v5.x TMY JSON response (`outputformat=json`).
pub fn parse_pvgis_tmy(json: &str, url: &str) -> Result<Weather, WeatherError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| WeatherError(format!("PVGIS JSON: {e}")))?;
    if let Some(m) = v.get("message").and_then(|m| m.as_str()) { return err(format!("PVGIS error: {m}")); }
    let loc = &v["inputs"]["location"];
    let meteo = &v["inputs"]["meteo_data"];
    let offset_h = loc["irradiance_time_offset"].as_f64().unwrap_or(0.0);
    let rows = v["outputs"]["tmy_hourly"].as_array().ok_or_else(|| WeatherError("PVGIS JSON has no outputs.tmy_hourly".into()))?;
    let mut hours = Vec::with_capacity(rows.len());
    for r in rows {
        let t = r["time(UTC)"].as_str().ok_or_else(|| WeatherError("missing time(UTC)".into()))?;
        // "YYYYMMDD:HHMM" — start of the hour in UTC.
        let p = |a: usize, b: usize| t.get(a..b).and_then(|s| s.parse::<i64>().ok()).ok_or_else(|| WeatherError(format!("bad time {t}")));
        let (y, mo, d, hh, mm) = (p(0, 4)?, p(4, 6)? as u32, p(6, 8)? as u32, p(9, 11)?, p(11, 13)?);
        let start = days_from_civil(y, mo, d) * 86400 + hh * 3600 + mm * 60;
        let f = |k: &str| r[k].as_f64().unwrap_or(f64::NAN);
        hours.push(Hour { unix_mid: start + (offset_h * 3600.0).round() as i64, ghi: f("G(h)"), dni: f("Gb(n)"), dhi: f("Gd(h)"), temp_air: f("T2m"), wind: f("WS10m"), pressure: f("SP") });
    }
    let mut notes = vec![format!("PVGIS TMY, radiation database {}, years {}–{}", meteo["radiation_db"].as_str().unwrap_or("?"), meteo["year_min"], meteo["year_max"])];
    let horizon = meteo["use_horizon"].as_bool().unwrap_or(false);
    if horizon { notes.push(format!("Beam irradiance includes horizon shading ({})", meteo["horizon_db"].as_str().unwrap_or("?"))); }
    notes.push(format!("Irradiance time offset {offset_h} h applied"));
    Ok(Weather {
        latitude: loc["latitude"].as_f64().unwrap_or(f64::NAN),
        longitude: loc["longitude"].as_f64().unwrap_or(f64::NAN),
        elevation: loc["elevation"].as_f64().unwrap_or(f64::NAN),
        horizon_applied: horizon,
        provenance: Provenance { provider: "PVGIS (European Commission JRC)".into(), dataset: format!("TMY {}", meteo["radiation_db"].as_str().unwrap_or("")), url: url.into(), retrieved_unix: now(), licence: PVGIS_LICENCE_NOTE.into(), notes },
        hours,
    })
}

pub fn pvgis_tmy_url(lat: f64, lon: f64) -> String {
    format!("https://re.jrc.ec.europa.eu/api/v5_3/tmy?lat={lat:.5}&lon={lon:.5}&outputformat=json")
}

/// Fetch a TMY from the PVGIS API. Only coordinates leave the machine.
#[cfg(feature = "online")]
pub fn fetch_pvgis_tmy(lat: f64, lon: f64) -> Result<Weather, WeatherError> {
    let url = pvgis_tmy_url(lat, lon);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .user_agent(concat!("SolarCraft/", env!("CARGO_PKG_VERSION"), " (+https://github.com/AA-EION/SolarCraft)"))
        .http_status_as_error(false)
        .build().into();
    let mut resp = agent.get(&url).call().map_err(|e| WeatherError(format!("PVGIS request failed: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().map_err(|e| WeatherError(format!("PVGIS read failed: {e}")))?;
    if status != 200 {
        let msg = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or(body.chars().take(200).collect());
        return err(format!("PVGIS returned HTTP {status}: {msg}"));
    }
    parse_pvgis_tmy(&body, &url)
}

/// Parse an EnergyPlus EPW file. EPW hour h (1–24) covers the interval ending
/// at h:00 local standard time; irradiance is the hourly sum, so the midpoint
/// is h − 0.5 local standard time.
pub fn parse_epw(text: &str, source_name: &str) -> Result<Weather, WeatherError> {
    let mut lines = text.lines();
    let header = lines.next().ok_or_else(|| WeatherError("empty EPW".into()))?;
    let h: Vec<&str> = header.split(',').collect();
    if h.first().map(|s| s.trim()) != Some("LOCATION") || h.len() < 10 { return err("EPW: first line must be LOCATION"); }
    let num = |i: usize| h[i].trim().parse::<f64>().map_err(|_| WeatherError(format!("EPW LOCATION field {i} not numeric")));
    let (lat, lon, tz, elev) = (num(6)?, num(7)?, num(8)?, num(9)?);
    let mut hours = vec![];
    for l in lines {
        let c: Vec<&str> = l.split(',').collect();
        if c.len() < 22 { continue; }
        let Ok(year) = c[0].trim().parse::<i64>() else { continue };
        let (Ok(mo), Ok(d), Ok(hr)) = (c[1].trim().parse::<u32>(), c[2].trim().parse::<u32>(), c[3].trim().parse::<i64>()) else { continue };
        let f = |i: usize, missing: f64| c[i].trim().parse::<f64>().ok().filter(|v| *v < missing).unwrap_or(f64::NAN);
        let local_mid = days_from_civil(year, mo, d) * 86400 + hr * 3600 - 1800;
        hours.push(Hour {
            unix_mid: local_mid - (tz * 3600.0).round() as i64,
            temp_air: f(6, 99.9), pressure: f(9, 999_999.0), ghi: f(13, 9999.0), dni: f(14, 9999.0), dhi: f(15, 9999.0), wind: f(21, 999.0),
        });
    }
    if hours.is_empty() { return err("EPW contains no data rows"); }
    Ok(Weather {
        latitude: lat, longitude: lon, elevation: elev, horizon_applied: false,
        provenance: Provenance { provider: "EPW file".into(), dataset: header.split(',').skip(1).take(4).collect::<Vec<_>>().join(", "), url: source_name.into(), retrieved_unix: now(), licence: "As supplied by the user; check the file's source licence.".into(), notes: vec![format!("Time zone {tz:+} h; midpoint convention")] },
        hours,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct QcReport { pub hours: usize, pub missing_values: usize, pub night_irradiance: usize, pub over_extraterrestrial: usize, pub component_mismatch: usize, pub annual_ghi_kwh_m2: f64, pub ok: bool, pub messages: Vec<String> }

/// Basic quality checks (BSRN-style physically possible limits, simplified).
pub fn quality_check(w: &Weather) -> QcReport {
    use solarcraft_pv::{irradiance as irr, spa};
    let obs = spa::Observer::new(w.latitude, w.longitude, if w.elevation.is_finite() { w.elevation } else { 0.0 });
    let (mut missing, mut night, mut over, mut mismatch, mut ghi_sum) = (0, 0, 0, 0, 0.0);
    for h in &w.hours {
        if [h.ghi, h.dni, h.dhi, h.temp_air].iter().any(|v| !v.is_finite()) { missing += 1; continue; }
        let sp = spa::solar_position(h.unix_mid as f64, &obs, 69.0);
        let e0 = irr::dni_extra(solarcraft_pv::day_of_year_utc(h.unix_mid as f64) as f64, 1366.1);
        let cz = (90.0 - sp.apparent_elevation).to_radians().cos();
        if sp.apparent_elevation < -2.0 && h.ghi > 5.0 { night += 1; }
        if h.ghi > 1.5 * e0 * cz.max(0.0).powf(1.2) + 100.0 || h.dni > e0 { over += 1; }
        let closure = h.dhi + h.dni * cz.max(0.0);
        if h.ghi > 50.0 && (closure - h.ghi).abs() / h.ghi > 0.15 { mismatch += 1; }
        ghi_sum += h.ghi;
    }
    let mut messages = vec![];
    if w.hours.len() != 8760 && w.hours.len() != 8784 { messages.push(format!("{} hours (expected 8760 or 8784)", w.hours.len())); }
    if missing > 0 { messages.push(format!("{missing} hours with missing values")); }
    if night > 0 { messages.push(format!("{night} night hours with irradiance — check the time zone/offset")); }
    if over > 0 { messages.push(format!("{over} hours exceed physical limits")); }
    if mismatch > 24 { messages.push(format!("{mismatch} hours where GHI ≠ DHI + DNI·cos z by > 15 %")); }
    let ok = missing == 0 && night <= 24 && over == 0 && (w.hours.len() == 8760 || w.hours.len() == 8784);
    QcReport { hours: w.hours.len(), missing_values: missing, night_irradiance: night, over_extraterrestrial: over, component_mismatch: mismatch, annual_ghi_kwh_m2: ghi_sum / 1000.0, ok, messages }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn epw_midpoint_and_timezone() {
        let mut s = String::from("LOCATION,Bogota,CUN,COL,TEST,802220,4.70,-74.13,-5.0,2547.0\n");
        for l in 0..7 { s.push_str(&format!("HEADER{l}\n")); }
        // 1 Jan 2020 hour 13 (12:00–13:00 local, UTC−5) -> midpoint 17:30 UTC
        s.push_str("2020,1,1,13,0,?,15.0,5.0,70,75000,0,0,300,800,500,300,0,0,0,0,0,2.5,0,0,0,0,0,0,0,0,0,0,0,0,0\n");
        let w = parse_epw(&s, "test.epw").unwrap();
        let expected = days_from_civil(2020, 1, 1) * 86400 + 17 * 3600 + 1800;
        assert_eq!(w.hours[0].unix_mid, expected);
        assert_eq!((w.hours[0].ghi, w.hours[0].dni, w.hours[0].dhi, w.hours[0].wind), (800.0, 500.0, 300.0, 2.5));
    }
    #[test]
    fn pvgis_offset_and_error() {
        let j = r#"{"inputs":{"location":{"latitude":4.7,"longitude":-74.1,"elevation":2555,"irradiance_time_offset":0.5},"meteo_data":{"radiation_db":"PVGIS-ERA5","year_min":2005,"year_max":2023,"use_horizon":true,"horizon_db":"DEM-calculated"}},
                   "outputs":{"tmy_hourly":[{"time(UTC)":"20080101:1700","T2m":15.0,"G(h)":600.0,"Gb(n)":400.0,"Gd(h)":250.0,"WS10m":2.0,"SP":75000}]}}"#;
        let w = parse_pvgis_tmy(j, "u").unwrap();
        assert_eq!(w.hours[0].unix_mid, days_from_civil(2008, 1, 1) * 86400 + 17 * 3600 + 1800);
        assert!(w.horizon_applied);
        assert!(parse_pvgis_tmy(r#"{"message":"Location over the sea"}"#, "u").unwrap_err().0.contains("sea"));
    }
}
