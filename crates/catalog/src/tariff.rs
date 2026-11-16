// SPDX-License-Identifier: GPL-3.0-or-later
//! Tariff records: schedule decoding and conversion to SolarCraft's billing model.
//!
//! `htNtJson` is Base64 of UTF-16LE JSON (optionally with a BOM). The decoded
//! object carries `WochenTage` (seven row tokens in .NET weekday order, Sunday
//! first), `TarriffZoneDefList` (rows of 288 zone codes = 12 months × 24 hours),
//! `EnergyTariffRanges` (one rate group per zone code, 1-based) and an optional
//! `TimePlusXMinutes`. The hourly adapter picks the row for the weekday and the
//! slot `month0 × 24 + hour`; the minute offset is preserved but not applied.
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use solarcraft_model::plant::{CatalogRef, ExportCompensation, Tariff, ZoneSchedule};

#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError { Base64(String), Utf16(String), Json(String), Shape(String) }
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}

/// Decode a non-empty `htNtJson` payload to its JSON object.
pub fn decode_htnt(encoded: &str) -> Result<Value, DecodeError> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).map_err(|e| DecodeError::Base64(e.to_string()))?;
    if bytes.len() % 2 != 0 { return Err(DecodeError::Utf16("odd byte count".into())); }
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let text = String::from_utf16(&units).map_err(|e| DecodeError::Utf16(e.to_string()))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| DecodeError::Json(e.to_string()))
}

/// Encode an object the same way (tests and round trips).
pub fn encode_htnt(v: &Value) -> String {
    let s = v.to_string();
    let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Build the zone schedule from a decoded object. Each zone's price is its first rate row
/// (tiers beyond the first are kept in the record and reported).
pub fn zone_schedule(decoded: &Value, raw: &str) -> Result<ZoneSchedule, DecodeError> {
    let tokens: Vec<String> = decoded["WochenTage"].as_array().ok_or(DecodeError::Shape("WochenTage missing".into()))?.iter().map(|t| t.as_str().unwrap_or("").to_string()).collect();
    if tokens.len() != 7 { return Err(DecodeError::Shape(format!("{} weekday tokens", tokens.len()))); }
    let rows: Vec<Vec<u32>> = decoded["TarriffZoneDefList"].as_array().ok_or(DecodeError::Shape("TarriffZoneDefList missing".into()))?
        .iter().map(|r| r.as_array().map(|a| a.iter().map(|x| x.as_u64().unwrap_or(0) as u32).collect()).unwrap_or_default()).collect();
    if rows.is_empty() || rows.iter().any(|r| r.len() != 288) { return Err(DecodeError::Shape("zone rows must have 288 slots".into())); }
    let mut weekday_rows = [0usize; 7];
    for (i, t) in tokens.iter().enumerate() {
        // Tokens are row letters: "a" → row 0, "b" → row 1, …
        let c = t.trim().chars().next().ok_or(DecodeError::Shape("empty weekday token".into()))?;
        let r = (c.to_ascii_lowercase() as u32).checked_sub('a' as u32).ok_or(DecodeError::Shape(format!("weekday token '{t}'")))? as usize;
        if r >= rows.len() { return Err(DecodeError::Shape(format!("weekday token '{t}' has no row"))); }
        weekday_rows[i] = r;
    }
    let ranges = decoded["EnergyTariffRanges"].as_array().cloned().unwrap_or_default();
    let zone_prices: Vec<f64> = ranges.iter().map(|g| g["RangeValues"].as_array().and_then(|v| v.first()).and_then(|v| v["Value"].as_f64()).unwrap_or(0.0)).collect();
    let zone_names: Vec<String> = ranges.iter().map(|g| g["Tarifname"].as_str().unwrap_or("").trim().to_string()).collect();
    let max_code = rows.iter().flatten().copied().max().unwrap_or(0) as usize;
    if max_code > zone_prices.len() { return Err(DecodeError::Shape(format!("zone code {max_code} but {} rate groups", zone_prices.len()))); }
    Ok(ZoneSchedule { weekday_rows, rows, zone_prices, zone_names, time_plus_minutes: decoded["TimePlusXMinutes"].as_i64(), raw_encoded: raw.to_string() })
}

/// Tariff family from the dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TariffFamily { FeedIn, NetMetering, Demand }

/// Context that selects tiers: annual consumption (from-grid bands) and installed power (feed-in bands).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TariffContext { pub annual_consumption_kwh: f64, pub installed_kwp: f64 }

#[derive(Debug, Clone, Serialize)]
pub struct Concept { pub concept: i64, pub unit: String, pub is_expense: bool, pub tiers: Vec<[f64; 3]> }

pub fn concepts(rec: &Value) -> Vec<Concept> {
    rec["tariffConcepts"].as_array().cloned().unwrap_or_default().iter().map(|c| Concept {
        concept: c["concept"].as_i64().unwrap_or(-1), unit: c["tariffUnit"].as_str().unwrap_or("").to_string(), is_expense: c["isExpense"].as_bool().unwrap_or(false),
        tiers: c["tariffRangeValues"].as_array().cloned().unwrap_or_default().iter().map(|t| [t["key"].as_f64().unwrap_or(0.0), t["range"].as_f64().unwrap_or(0.0), t["value"].as_f64().unwrap_or(0.0)]).collect(),
    }).collect()
}

/// Rate from tiers keyed by a breakpoint: the reached band, or (zoning) the quantity-weighted mix of bands.
/// Tiers are sorted by key (lower bound, inclusive).
pub fn tier_rate(tiers: &[[f64; 3]], quantity: f64, zoning: bool) -> Option<f64> {
    let mut t: Vec<[f64; 3]> = tiers.to_vec();
    t.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let first = t.first()?;
    if t.len() == 1 || quantity <= first[0] { return Some(first[2]); }
    if !zoning { return t.iter().rev().find(|x| quantity >= x[0]).map(|x| x[2]); }
    let mut cost = 0.0;
    for (i, x) in t.iter().enumerate() {
        let hi = t.get(i + 1).map_or(quantity, |n| n[0].min(quantity));
        if hi > x[0] { cost += (hi - x[0]) * x[2]; }
    }
    let base = first[0].min(quantity);
    Some(if quantity - base > 0.0 { (cost + base * first[2]) / quantity } else { first[2] })
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Conversion { pub notes: Vec<String>, pub unsupported: Vec<String> }

/// Apply a catalog tariff to a SolarCraft tariff: from-grid families set import, fixed and demand
/// charges (and the zone schedule); production families set export compensation.
pub fn apply(t: &mut Tariff, family: TariffFamily, rec: &Value, ctx: TariffContext, cref: CatalogRef) -> Conversion {
    let mut cv = Conversion::default();
    let zoning = rec["zoning"].as_bool().unwrap_or(false);
    let unit_scale = |u: &str| -> Option<f64> { match u.to_ascii_lowercase().as_str() { "month" => Some(1.0), "year" | "jahr" => Some(1.0 / 12.0), _ => None } };
    for c in concepts(rec) {
        let u = c.unit.to_ascii_lowercase();
        match (family, c.concept) {
            (TariffFamily::FeedIn, 0) => match tier_rate(&c.tiers, ctx.installed_kwp, zoning) {
                Some(r) if u.starts_with("kwh") => { t.export = ExportCompensation::Fixed { price: r }; cv.notes.push(format!("feed-in {r} per kWh (tier by installed {} kWp, zoning {zoning})", ctx.installed_kwp)); }
                _ => cv.unsupported.push(format!("feed-in concept with unit '{}'", c.unit)),
            },
            (TariffFamily::FeedIn, k) => cv.unsupported.push(format!("production concept {k} ({}) — {} rows preserved", if k == 1 { "Consumption" } else { "Generation" }, c.tiers.len())),
            (_, 0) => match (unit_scale(&u), tier_rate(&c.tiers, ctx.annual_consumption_kwh, zoning)) {
                (Some(s), Some(r)) => { t.fixed_per_month = r * s; cv.notes.push(format!("base charge {r} per {}", c.unit)); }
                _ => cv.unsupported.push(format!("base concept with unit '{}'", c.unit)),
            },
            (_, 1) => match tier_rate(&c.tiers, 0.0, false) {
                Some(r) if u.contains("kw") && !u.contains("kwh") => { t.demand_charge_per_kw_month = r; cv.notes.push(format!("demand charge {r} per kW·month")); }
                _ => cv.unsupported.push(format!("demand concept with unit '{}'", c.unit)),
            },
            (_, 2) => match tier_rate(&c.tiers, ctx.annual_consumption_kwh, zoning) {
                Some(r) if u.starts_with("kwh") => { t.import_price = r; cv.notes.push(format!("energy {r} per kWh (tier by {} kWh/yr, zoning {zoning})", ctx.annual_consumption_kwh)); }
                _ => cv.unsupported.push(format!("energy concept with unit '{}'", c.unit)),
            },
            (_, k) => cv.unsupported.push(format!("concept {k} with unit '{}'", c.unit)),
        }
    }
    if family != TariffFamily::FeedIn {
        if let Some(raw) = rec["htNtJson"].as_str().filter(|s| !s.trim().is_empty()) {
            match decode_htnt(raw).and_then(|d| zone_schedule(&d, raw)) {
                Ok(z) => { cv.notes.push(format!("time-of-use schedule: {} rows, {} zones ({})", z.rows.len(), z.zone_prices.len(), z.zone_names.join(", "))); t.zones = Some(z); }
                Err(e) => cv.unsupported.push(format!("schedule not decoded: {e}")),
            }
        }
        if family == TariffFamily::NetMetering { t.export = ExportCompensation::NetMetering { surplus_price: 0.0 }; cv.notes.push("net metering: monthly offset at import price; surplus price 0 (source has no surplus rate)".into()); }
        t.name = rec["name"].as_str().unwrap_or("").to_string();
        t.catalog_ref = Some(cref);
    }
    for (k, why) in [("minimumPrice", "minimum price rule"), ("thresholdValue_1", "threshold rule"), ("saldoRollover", "rollover rule"), ("payoutDuration", "payout duration")] {
        if !rec[k].is_null() && rec[k] != json!(0) && rec[k] != json!(false) { cv.unsupported.push(format!("{why} ({k} = {}) preserved, not applied", rec[k])); }
    }
    if rec["isDynamic"].as_bool() == Some(true) { cv.unsupported.push("dynamic tariff: no price series in the source".into()); }
    cv
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sched() -> Value {
        let mut weekday = vec![1u32; 288];
        for m in 0..12 { for h in 17..20 { weekday[m * 24 + h] = 2; } }
        json!({"$id": "1", "WochenTage": ["b", "a", "a", "a", "a", "a", "b"], "TimePlusXMinutes": 30,
               "TarriffZoneDefList": [weekday, vec![1u32; 288]],
               "EnergyTariffRanges": [{"Tarifname": "Off-peak", "RangeValues": [{"Id": "x", "Key": 0.0, "Range": 100.0, "Value": 0.45}]}, {"Tarifname": "Peak", "RangeValues": [{"Id": "y", "Key": 0.0, "Range": 100.0, "Value": 1.75}]}]})
    }
    #[test]
    fn decode_round_trip_and_adapter() {
        let raw = encode_htnt(&sched());
        let d = decode_htnt(&raw).unwrap();
        assert_eq!(d, sched());
        let z = zone_schedule(&d, &raw).unwrap();
        assert_eq!(z.time_plus_minutes, Some(30));
        // Monday (weekday 0 → .NET 1 → row 'a' = 1) 18:00 in June (month0 5): peak; 16:00 and 20:00 are off-peak.
        assert_eq!(z.zone(0, 5, 18), Some(2));
        assert_eq!(z.price(0, 5, 18), Some(1.75));
        assert_eq!(z.price(0, 5, 16), Some(0.45));
        assert_eq!(z.price(0, 5, 20), Some(0.45));
        // Saturday (weekday 5 → .NET 6 → 'b' = row 1) and Sunday (6 → .NET 0 → 'b'): off-peak at 18:00.
        assert_eq!(z.price(5, 5, 18), Some(0.45));
        assert_eq!(z.price(6, 5, 18), Some(0.45));
        // Slot = month0 × 24 + hour: December 23:00 is the last slot.
        assert_eq!(z.zone(1, 11, 23), Some(1));
        assert!(decode_htnt("@@").is_err());
    }
    #[test]
    fn tiers() {
        let t = [[0.0, 0.0, 0.30], [1000.0, 0.0, 0.20]];
        assert_eq!(tier_rate(&t, 500.0, false), Some(0.30));
        assert_eq!(tier_rate(&t, 1000.0, false), Some(0.20)); // lower bound inclusive
        assert!((tier_rate(&t, 2000.0, true).unwrap() - 0.25).abs() < 1e-12); // weighted
    }
}
