// SPDX-License-Identifier: GPL-3.0-or-later
//! Import an equipment workbook into the user's local library.
//!
//! The workbook layout is the one used by the owner's catalog backups: one
//! sheet per category (`Online pvmodules`, `Online pvinverters`,
//! `Online batterysystems`, `Online batteryinverters`) with one header row.
//! Each row becomes a record in SolarCraft's own equipment schema, the same
//! schema used for user-authored and subscription items. SolarCraft ships no
//! records from any such workbook; the user imports their own data.
//!
//! Units are converted explicitly (mm → m, mV/K → V/K, mA/K → A/K, fraction/K
//! → %/K, W → kW where the schema says so). Columns whose meaning has not been
//! verified are kept verbatim under `unverified` and are not used in
//! calculations.
use crate::plant_ops::canonical_equipment_opts;
use crate::{e, Result};
use calamine::{open_workbook_auto, Data, Reader};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Default)]
pub struct SheetReport { pub sheet: String, pub kind: String, pub rows: usize, pub imported: usize, pub skipped: usize, pub first_errors: Vec<String> }

#[derive(Debug, Clone, Serialize, Default)]
pub struct ImportReport { pub file_sha256: String, pub sheets: Vec<SheetReport>, pub not_imported: Vec<String> }

type Row = HashMap<String, Data>;

fn num(r: &Row, k: &str) -> Option<f64> {
    match r.get(k)? {
        Data::Float(f) => Some(*f),
        Data::Int(i) => Some(*i as f64),
        Data::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Data::String(s) => s.trim().replace(',', ".").parse().ok(),
        _ => None,
    }
}
fn text(r: &Row, k: &str) -> String {
    match r.get(k) { Some(Data::String(s)) => s.trim().to_string(), Some(Data::Float(f)) => f.to_string(), Some(Data::Int(i)) => i.to_string(), _ => String::new() }
}
fn flag(r: &Row, k: &str) -> bool {
    match r.get(k) { Some(Data::Bool(b)) => *b, Some(Data::String(s)) => s.eq_ignore_ascii_case("true"), Some(Data::Int(i)) => *i != 0, Some(Data::Float(f)) => *f != 0.0, _ => false }
}
fn raw(r: &Row, k: &str) -> Value {
    match r.get(k) { Some(Data::Float(f)) => json!(f), Some(Data::Int(i)) => json!(i), Some(Data::Bool(b)) => json!(b), Some(Data::String(s)) => json!(s), _ => Value::Null }
}
fn need(r: &Row, k: &str) -> std::result::Result<f64, String> { num(r, k).filter(|v| v.is_finite()).ok_or_else(|| format!("missing {k}")) }

/// Manufacturer from an embedded company object (`company` column, JSON with `name`).
fn manufacturer(r: &Row, companies: &HashMap<String, String>) -> String {
    let c = text(r, "company");
    serde_json::from_str::<Value>(&c).ok().and_then(|v| v["name"].as_str().map(str::to_string))
        .or_else(|| companies.get(&text(r, "companyId")).cloned()).unwrap_or_else(|| "Unknown".into())
}

fn keep(r: &Row, keys: &[&str]) -> Map<String, Value> {
    keys.iter().filter_map(|k| { let v = raw(r, k); (!v.is_null()).then(|| (k.to_string(), v)) }).collect()
}

fn curve(r: &Row) -> Vec<[f64; 2]> {
    [(0.05, "e5"), (0.10, "e10"), (0.20, "e20"), (0.30, "e30"), (0.50, "e50"), (0.75, "e75"), (1.0, "e100")]
        .iter().filter_map(|(x, k)| num(r, k).filter(|v| *v > 0.0 && *v <= 1.0).map(|v| [*x, v])).collect()
}

fn reference(sha: &str, sheet: &str, r: &Row) -> String { format!("workbook:{}:{sheet}:{}@{}", &sha[..12.min(sha.len())], text(r, "id"), text(r, "version")) }

fn module(r: &Row, sha: &str, sheet: &str, companies: &HashMap<String, String>) -> std::result::Result<Value, String> {
    let mut rec = json!({
        "id": format!("imp-{}", text(r, "id")), "manufacturer": manufacturer(r, companies), "model": text(r, "name"), "provenance": "imported",
        "technology": match num(r, "cellType").map(|v| v as i64) { Some(0) | Some(1) => "c-Si", _ => "other" },
        "cells_in_series": need(r, "cellCount")? as u32,
        "length_m": need(r, "height")? / 1000.0, "width_m": need(r, "width")? / 1000.0,
        "v_oc": need(r, "openCircuitVoltage")?, "i_sc": need(r, "shortCircuitCurrent")?, "v_mp": need(r, "mpP_Voltage")?, "i_mp": need(r, "mpP_Current")?,
        "alpha_sc": need(r, "currentCoefficient")? / 1000.0, "beta_voc": need(r, "voltageCoefficient")? / 1000.0, "gamma_pmp": need(r, "outputpowerCoefficient")? * 100.0,
        "max_system_voltage_v": num(r, "maxSysVoltage").filter(|v| *v > 0.0), "bypass_diodes": num(r, "diodeCount").filter(|v| *v > 0.0).map(|v| v as u32),
        "half_cell": flag(r, "isHalfCellModule"), "bifaciality": num(r, "bifacialityFactor").filter(|v| *v > 0.0),
        "thickness_m": num(r, "depthOfPanel").filter(|v| *v > 0.0).map(|v| v / 1000.0), "weight_kg": num(r, "weight").filter(|v| *v > 0.0),
        "datasheet_ref": reference(sha, sheet, r),
        "unverified": keep(r, &["cellType", "iam", "increaseU0", "lossVoltagePerBypassDiode", "standardPartLoadOperation", "standardCellConnection", "onlyTransformerInverter", "widthOfFrame", "specifiedOutputPower"]),
    });
    if let (Some(g), Some(v), Some(i), Some(voc), Some(isc)) = (num(r, "g2"), num(r, "umpp2"), num(r, "impp2"), num(r, "uoc2"), num(r, "isc2")) {
        if g > 0.0 && v > 0.0 && i > 0.0 { rec["low_light"] = json!({"irradiance_w_m2": g, "v_mp": v, "i_mp": i, "v_oc": voc, "i_sc": isc}); }
    }
    Ok(rec)
}

fn inverter(r: &Row, sha: &str, sheet: &str, companies: &HashMap<String, String>) -> std::result::Result<Value, String> {
    let mut inputs = vec![];
    for t in 1..=3 {
        let n = num(r, &format!("mppT{t}_NrOf")).unwrap_or(0.0) as usize;
        for _ in 0..n {
            inputs.push(json!({"v_min": need(r, &format!("mppT{t}_U_MPP_Min"))?, "v_max": need(r, &format!("mppT{t}_U_MPP_Max"))?, "i_max": need(r, &format!("mppT{t}_MaxCurrent"))?,
                "i_sc_max": num(r, &format!("mppT{t}_MaxShortCircuitCurrent")).filter(|v| *v > 0.0), "p_max_w": num(r, &format!("mppT{t}_MaxPower")).filter(|v| *v > 0.0)}));
        }
    }
    if inputs.is_empty() { return Err("no MPP tracker inputs".into()); }
    let f = |k: &str| inputs.iter().filter_map(|i| i[k].as_f64()).collect::<Vec<_>>();
    let c = curve(r);
    let eta = c.iter().map(|k| k[1]).fold(0.0, f64::max);
    if eta <= 0.0 { return Err("no efficiency curve".into()); }
    Ok(json!({
        "id": format!("imp-{}", text(r, "id")), "manufacturer": manufacturer(r, companies), "model": text(r, "name"), "provenance": "imported",
        "pac_max_w": need(r, "maxAC_Power")?, "eta_nominal": eta, "efficiency_curve": c,
        "mppt_v_min": f("v_min").into_iter().fold(f64::INFINITY, f64::min), "mppt_v_max": f("v_max").into_iter().fold(0.0, f64::max),
        "v_dc_max": need(r, "maxInputVoltageDC")?, "mppt_count": inputs.len(), "i_max_per_mppt": f("i_max").into_iter().fold(0.0, f64::max),
        "night_w": num(r, "nightConsumption").unwrap_or(0.0), "standby_w": num(r, "standbyConsumption"),
        "pdc_max_w": num(r, "maxDC_Power").filter(|v| *v > 0.0), "pdc_nominal_w": num(r, "dC_PowerRating").filter(|v| *v > 0.0),
        "v_dc_nominal": num(r, "nominalVoltageDC").filter(|v| *v > 0.0), "phases": num(r, "numberOfPhases").map(|v| v as u32), "transformer": flag(r, "withTransformer"),
        "inputs": inputs, "datasheet_ref": reference(sha, sheet, r),
        "unverified": keep(r, &["changeOfEtaDependentOnVoltage", "feedInFrom", "matchingEtaMPP20", "matchingEtaMPP100", "parallelOperating", "maxCurrentDC", "maxShortCircuitCurrentDC", "dC_InputCount", "aC_PowerRating", "voltageAC", "e0"]),
    }))
}

fn battery_system(r: &Row, sha: &str, sheet: &str, companies: &HashMap<String, String>) -> std::result::Result<Value, String> {
    let b: Value = serde_json::from_str(&text(r, "battery")).map_err(|_| "battery object missing".to_string())?;
    let g = |k: &str| b[k].as_f64().ok_or_else(|| format!("battery.{k} missing"));
    let per_string = num(r, "numberOfBatteriesPerString").unwrap_or(1.0).max(1.0);
    let strings = num(r, "numberOfBatteryStrings").unwrap_or(1.0).max(1.0);
    let cell_v = g("ratedVoltage")?;
    let series = (g("numberOfCellsSerie")? * per_string) as u32;
    let cap: Vec<[f64; 2]> = [(0.167, "capacityAt0167"), (0.5, "capacityAt05"), (1.0, "capacityAt1"), (5.0, "capacityAt5"), (10.0, "capacityAt10"), (100.0, "capacityAt100")]
        .iter().filter_map(|(h, k)| b[*k].as_f64().filter(|v| *v > 0.0).map(|v| [*h, v])).collect();
    let cyc: Vec<[f64; 2]> = [(20.0, "lifeCycles20"), (40.0, "lifeCycles40"), (60.0, "lifeCycles60"), (80.0, "lifeCycles80")]
        .iter().filter_map(|(d, k)| b[*k].as_f64().filter(|v| *v > 0.0).map(|v| [*d, v])).collect();
    let cells = solarcraft_model::equipment::BatteryCells { chemistry_code: b["batteryType"].as_u64().map(|v| v as u32), cell_voltage_v: cell_v, series, parallel: g("numberOfCellsParallel")?.max(1.0) as u32, strings: strings as u32,
        internal_resistance_ohm: b["internalResistance"].as_f64().unwrap_or(0.0), capacity_ah_by_hours: cap, cycles_by_dod: cyc };
    let kwh = cells.energy_kwh();
    if kwh <= 0.0 { return Err("no capacity".into()); }
    // Chemistry from the cell voltage (lead ≈ 2 V, LFP ≈ 3.2 V, NMC/NCA ≈ 3.6–3.7 V).
    let chem = if cell_v < 2.5 { "lead_acid" } else if cell_v < 3.4 { "lfp" } else if cell_v < 3.9 { "nmc" } else { "other" };
    let lead = chem == "lead_acid";
    let p_ch = need(r, "maxCharge")?;
    let p_dis = need(r, "maxDischarge")?;
    let mut unv = keep(r, &["coupling", "socBoostStart", "socBoostEnd", "socFloatStart", "socFullEnd", "socEqualEnd", "boostDuration", "durationFull", "durationEqualisation", "cycleFull", "cycleEqualisation", "e0"]);
    unv.insert("roundtrip_efficiency_assumed".into(), json!(true));
    Ok(json!({
        "id": format!("imp-{}", text(r, "id")), "manufacturer": manufacturer(r, companies), "model": text(r, "name"), "provenance": "imported",
        "chemistry": chem, "capacity_kwh": kwh, "nominal_voltage_v": cells.pack_voltage(),
        "soc_min_pct": (num(r, "socMin").unwrap_or(0.05) * 100.0).clamp(0.0, 99.0), "soc_max_pct": 100.0,
        "p_charge_max_kw": p_ch / 1000.0, "p_discharge_max_kw": p_dis / 1000.0,
        "roundtrip_efficiency": if lead { 0.85 } else { 0.95 },
        "self_discharge_pct_per_month": b["selfdischarge"].as_f64().unwrap_or(0.02) * 100.0,
        "cycle_life": cells.cycles_at(80.0).unwrap_or(if lead { 1500.0 } else { 6000.0 }),
        "cells": cells,
        "converter": {"p_rated_w": num(r, "nominalPower").unwrap_or(p_dis), "p_charge_max_w": p_ch, "p_discharge_max_w": p_dis, "efficiency_curve": curve(r), "standby_w": 0.0},
        "datasheet_ref": reference(sha, sheet, r), "unverified": unv,
    }))
}

fn battery_inverter(r: &Row, sha: &str, sheet: &str, companies: &HashMap<String, String>) -> std::result::Result<Value, String> {
    Ok(json!({
        "id": format!("imp-{}", text(r, "id")), "manufacturer": manufacturer(r, companies), "model": text(r, "name"), "provenance": "imported",
        "p_ac_max_w": need(r, "aC_RatedPower_25C")?, "efficiency_curve": curve(r), "standby_w": num(r, "consumption_Standby").unwrap_or(0.0),
        "datasheet_ref": reference(sha, sheet, r),
        "unverified": keep(r, &["aC_RatedVoltage", "aC_30MinPower", "aC_60MinPower", "aC_MaxGenCurrent", "batt_RatedInputVoltage", "batt_MinInputVoltage", "batt_MaxInputVoltage", "batt_MaxCurrent", "consumption_WithoutLoad", "maxNumberParallel1Phase", "maxNumberParallel2Phase", "maxNumberParallel3Phase", "e0"]),
    }))
}

/// Import supported sheets into the user library. `limit` caps rows per sheet (0 = all).
pub fn import_workbook(lib: &Path, file: &Path, limit: usize) -> Result<ImportReport> {
    let bytes = std::fs::read(file).map_err(|x| e("FILE_READ", format!("{}: {x}", file.display())))?;
    let sha = solarcraft_store::sha256_hex(&bytes);
    let mut wb = open_workbook_auto(file).map_err(|x| e("WORKBOOK", x))?;
    let names = wb.sheet_names().to_vec();
    let read = |wb: &mut calamine::Sheets<_>, name: &str| -> Result<Vec<Row>> {
        let range = wb.worksheet_range(name).map_err(|x| e("WORKBOOK", x))?;
        let mut it = range.rows();
        let header: Vec<String> = it.next().map(|h| h.iter().map(|c| c.to_string()).collect()).unwrap_or_default();
        Ok(it.map(|r| header.iter().cloned().zip(r.iter().cloned()).collect()).collect())
    };
    let mut companies = HashMap::new();
    if let Some(n) = names.iter().find(|n| n.ends_with("companies")) {
        for r in read(&mut wb, n)? { companies.insert(text(&r, "id"), text(&r, "name")); }
    }
    type Mapper = fn(&Row, &str, &str, &HashMap<String, String>) -> std::result::Result<Value, String>;
    let plan: [(&str, &str, Mapper); 4] = [("pvmodules", "module", module), ("pvinverters", "inverter", inverter), ("batterysystems", "battery", battery_system), ("batteryinverters", "battery_inverter", battery_inverter)];
    let mut report = ImportReport { file_sha256: sha.clone(), ..Default::default() };
    for n in &names {
        let lower = n.to_lowercase();
        let Some((_, kind, map)) = plan.iter().find(|(suffix, _, _)| lower.ends_with(suffix)) else {
            if !lower.contains("index") && !lower.contains("companies") && !lower.contains("metadata") && !lower.contains("versions") { report.not_imported.push(n.clone()); }
            continue;
        };
        let rows = read(&mut wb, n)?;
        let mut sr = SheetReport { sheet: n.clone(), kind: kind.to_string(), rows: rows.len(), ..Default::default() };
        let mut batch = vec![];
        for r in rows.iter().take(if limit == 0 { usize::MAX } else { limit }) {
            if flag(r, "isCorrupt") { sr.skipped += 1; continue; }
            let res = map(r, &sha, n, &companies).and_then(|v| canonical_equipment_opts(kind, &v, false).map_err(|x| x.message));
            match res {
                Ok(v) => batch.push((kind.to_string(), v)),
                Err(m) => { sr.skipped += 1; if sr.first_errors.len() < 5 { sr.first_errors.push(format!("{}: {m}", text(r, "name"))); } }
            }
        }
        sr.imported = crate::userlib::put_many(lib, &batch, "imported")?;
        report.sheets.push(sr);
    }
    Ok(report)
}
