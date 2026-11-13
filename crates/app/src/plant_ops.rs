// SPDX-License-Identifier: GPL-3.0-or-later
//! Plant operations: equipment authoring, consumption import, result
//! inspection by hour, and design variants with side-by-side comparison.
use crate::{e, open, save, Result};
use serde::Serialize;
use serde_json::Value;
use solarcraft_engine::calendar::local_time;
use solarcraft_engine::SimResult;
use solarcraft_model::equipment::{BatteryInverterSpec, BatterySpec, InverterSpec, Provenance};
use solarcraft_model::plant::{LoadProfile, Variant};
use solarcraft_model::Project;
use solarcraft_store::Store;
use std::path::Path;

/// Equipment kinds shared by the project library, the user library and the catalog.
pub const KINDS: [&str; 4] = ["module", "inverter", "battery", "battery_inverter"];

fn positive(v: f64, name: &str) -> Result<()> { if v.is_finite() && v > 0.0 { Ok(()) } else { Err(e("EQUIPMENT_INVALID", format!("{name} must be positive"))) } }

/// Check an equipment record and return it in canonical form (modules are fitted).
pub fn canonical_equipment(kind: &str, record: &Value) -> Result<Value> {
    let bad = |x: serde_json::Error| e("EQUIPMENT_INVALID", format!("{kind}: {x}"));
    Ok(match kind {
        "module" => {
            let mut r = record.clone();
            if r.get("provenance").is_none() { r["provenance"] = Value::from("user"); }
            if r.get("model_params").is_none() { r["model_params"] = Value::Null; }
            let mut m: solarcraft_model::equipment::ModuleSpec = serde_json::from_value(r).map_err(bad)?;
            for (v, n) in [(m.v_oc, "v_oc"), (m.i_sc, "i_sc"), (m.v_mp, "v_mp"), (m.i_mp, "i_mp"), (m.length_m, "length_m"), (m.width_m, "width_m")] { positive(v, n)?; }
            if m.v_mp >= m.v_oc || m.i_mp >= m.i_sc { return Err(e("EQUIPMENT_INVALID", "module: Vmp must be below Voc and Imp below Isc")); }
            m.fit().map_err(|x| e("MODULE_FIT", x))?;
            serde_json::to_value(m).unwrap_or_default()
        }
        "inverter" => {
            let mut r = record.clone();
            for (k, d) in [("provenance", Value::from("user")), ("night_w", Value::from(1.0))] { if r.get(k).is_none() { r[k] = d; } }
            let i: InverterSpec = serde_json::from_value(r).map_err(bad)?;
            for (v, n) in [(i.pac_max_w, "pac_max_w"), (i.eta_nominal, "eta_nominal"), (i.mppt_v_max, "mppt_v_max"), (i.v_dc_max, "v_dc_max"), (i.i_max_per_mppt, "i_max_per_mppt")] { positive(v, n)?; }
            if i.mppt_v_min >= i.mppt_v_max || i.mppt_v_max > i.v_dc_max { return Err(e("EQUIPMENT_INVALID", "inverter: need mppt_v_min < mppt_v_max ≤ v_dc_max")); }
            if i.mppt_count == 0 || i.eta_nominal > 1.0 { return Err(e("EQUIPMENT_INVALID", "inverter: mppt_count ≥ 1 and efficiency ≤ 1")); }
            if i.efficiency_curve.windows(2).any(|w| w[1][0] <= w[0][0]) || i.efficiency_curve.iter().any(|k| !(0.0..=1.0).contains(&k[1])) { return Err(e("EQUIPMENT_INVALID", "inverter: efficiency curve needs ascending load fractions and efficiencies in 0–1")); }
            serde_json::to_value(i).unwrap_or_default()
        }
        "battery" => {
            let mut r = record.clone();
            if r.get("provenance").is_none() { r["provenance"] = Value::from("user"); }
            let b: BatterySpec = serde_json::from_value(r).map_err(bad)?;
            for (v, n) in [(b.capacity_kwh, "capacity_kwh"), (b.nominal_voltage_v, "nominal_voltage_v"), (b.p_charge_max_kw, "p_charge_max_kw"), (b.p_discharge_max_kw, "p_discharge_max_kw"), (b.roundtrip_efficiency, "roundtrip_efficiency"), (b.cycle_life, "cycle_life")] { positive(v, n)?; }
            if b.roundtrip_efficiency > 1.0 || !(0.0..100.0).contains(&b.soc_min_pct) || b.soc_max_pct > 100.0 || b.soc_min_pct >= b.soc_max_pct { return Err(e("EQUIPMENT_INVALID", "battery: efficiency ≤ 1 and 0 ≤ soc_min < soc_max ≤ 100")); }
            serde_json::to_value(b).unwrap_or_default()
        }
        "battery_inverter" => {
            let mut r = record.clone();
            if r.get("provenance").is_none() { r["provenance"] = Value::from("user"); }
            let b: BatteryInverterSpec = serde_json::from_value(r).map_err(bad)?;
            positive(b.p_ac_max_w, "p_ac_max_w")?;
            if b.efficiency_curve.is_empty() { return Err(e("EQUIPMENT_INVALID", "battery inverter: efficiency curve required")); }
            serde_json::to_value(b).unwrap_or_default()
        }
        _ => return Err(e("INVALID_ARGUMENT", format!("unknown equipment kind '{kind}' (use {})", KINDS.join(", ")))),
    })
}

/// Add or replace an equipment record in the project library (works offline; same schema as catalog items).
pub fn add_equipment(path: &Path, kind: &str, record: &Value) -> Result<Value> {
    let rec = canonical_equipment(kind, record)?;
    let (mut s, mut p) = open(path)?;
    insert(&mut p, kind, rec.clone())?;
    solarcraft_model::derive::derive(&mut p);
    save(&mut s, &p, &format!("equipment {kind} {}", rec["id"].as_str().unwrap_or("")))?;
    Ok(rec)
}

pub(crate) fn insert(p: &mut Project, kind: &str, rec: Value) -> Result<()> {
    let bad = |x: serde_json::Error| e("EQUIPMENT_INVALID", x);
    let id = rec["id"].as_str().unwrap_or_default().to_string();
    if id.is_empty() { return Err(e("EQUIPMENT_INVALID", "equipment id is required")); }
    let l = &mut p.library;
    match kind {
        "module" => { l.modules.retain(|x| x.id != id); l.modules.push(serde_json::from_value(rec).map_err(bad)?); }
        "inverter" => { l.inverters.retain(|x| x.id != id); l.inverters.push(serde_json::from_value(rec).map_err(bad)?); }
        "battery" => { l.batteries.retain(|x| x.id != id); l.batteries.push(serde_json::from_value(rec).map_err(bad)?); }
        "battery_inverter" => { l.battery_inverters.retain(|x| x.id != id); l.battery_inverters.push(serde_json::from_value(rec).map_err(bad)?); }
        _ => return Err(e("INVALID_ARGUMENT", format!("unknown equipment kind '{kind}'"))),
    }
    Ok(())
}

/// Generic records so a battery design can be started without any catalog.
pub fn add_generic_storage_equipment(path: &Path) -> Result<Vec<String>> {
    let (mut s, mut p) = open(path)?;
    let kw = p.arrays.iter().map(|a| a.target_kwp.value).sum::<f64>().clamp(3.0, 10.0).round();
    let hy = InverterSpec::generic_hybrid(kw);
    let bat = BatterySpec::generic_lfp(10.0);
    let bi = BatteryInverterSpec::generic(5.0);
    let ids = vec![hy.id.clone(), bat.id.clone(), bi.id.clone()];
    if p.library.inverter(&hy.id).is_none() { p.library.inverters.push(hy); }
    if p.library.battery(&bat.id).is_none() { p.library.batteries.push(bat); }
    if p.library.battery_inverter(&bi.id).is_none() { p.library.battery_inverters.push(bi); }
    save(&mut s, &p, "add generic storage equipment")?;
    Ok(ids)
}

/// Import a metered consumption series: one value per line (kWh per interval, or kW
/// average with `unit = "kw"`), optionally `timestamp,value`. 8760/17520/35040/105120/525600 rows.
pub fn import_load_series(path: &Path, file: &Path, unit: &str) -> Result<Value> {
    let text = std::fs::read_to_string(file).map_err(|x| e("FILE_READ", format!("{}: {x}", file.display())))?;
    let mut vals = vec![];
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') { continue; }
        let last = t.rsplit([',', ';', '\t']).next().unwrap_or(t).trim();
        match last.replace(',', ".").parse::<f64>() {
            Ok(v) if v.is_finite() && v >= 0.0 => vals.push(v),
            Ok(_) => return Err(e("LOAD_PARSE", format!("line {}: negative or invalid value", i + 1))),
            Err(_) if vals.is_empty() => continue, // header
            Err(_) => return Err(e("LOAD_PARSE", format!("line {}: '{last}' is not a number", i + 1))),
        }
    }
    let per_hour = match vals.len() { 8760 => 1.0, 17520 => 2.0, 35040 => 4.0, 105120 => 12.0, 525600 => 60.0, n => return Err(e("LOAD_PARSE", format!("{n} values; expected one 365-day year at 60/30/15/5/1-minute steps"))) };
    if unit.eq_ignore_ascii_case("kw") { for v in &mut vals { *v /= per_hour; } }
    let total: f64 = vals.iter().sum();
    let (mut s, mut p) = open(path)?;
    let bytes = serde_json::to_vec(&vals).map_err(|x| e("ENCODE", x))?;
    let hash = s.put_blob("load", "application/vnd.solarcraft.load+json", &bytes).map_err(|x| e("PROJECT_SAVE", x))?;
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    p.loads.profile = LoadProfile::Series { blob: hash, name: name.clone(), step_minutes: (60.0 / per_hour) as u32, values: vals.len() };
    p.loads.annual_kwh = total;
    save(&mut s, &p, &format!("load series {name}"))?;
    Ok(serde_json::json!({"name": name, "values": vals.len(), "step_minutes": 60.0 / per_hour, "annual_kwh": total}))
}

pub(crate) fn load_series(s: &Store, p: &Project) -> Result<Option<Vec<f64>>> {
    if let LoadProfile::Series { blob, .. } = &p.loads.profile {
        let b = s.get_blob(blob).map_err(|x| e("PROJECT_LOAD", x))?.ok_or_else(|| e("LOAD_MISSING", "load series blob missing"))?;
        return Ok(Some(serde_json::from_slice(&b).map_err(|x| e("LOAD_DECODE", x))?));
    }
    Ok(None)
}

#[derive(Debug, Clone, Serialize)]
pub struct HourRow {
    pub index: usize, pub month: u32, pub day_of_year: u32, pub hour: u32, pub weekday: u32, pub unix_mid: i64,
    pub poa_w_m2: f32, pub t_cell_c: f32, pub pv_dc_wh: f32, pub pv_ac_wh: f32, pub clipped_wh: f32, pub load_wh: f32, pub pv_to_load_wh: f32,
    pub batt_charge_wh: f32, pub batt_discharge_wh: f32, pub soc_pct: f32, pub backup_wh: f32, pub self_wh: f32, pub grid_services_wh: f32,
    pub import_wh: f32, pub export_wh: f32, pub curtailed_wh: f32, pub unserved_wh: f32, pub outage: bool, pub residual_wh: f32,
}

pub fn hour_row(r: &SimResult, k: usize) -> HourRow {
    let fl = &r.flows;
    let g = |v: &Vec<f32>| v.get(k).copied().unwrap_or(0.0);
    let lh = r.local_hour.get(k).copied().unwrap_or(k as u32);
    let lt = local_time(lh);
    HourRow { index: k, month: lt.month, day_of_year: lt.day_of_year + 1, hour: lt.hour, weekday: lt.weekday, unix_mid: r.hourly.unix_mid.get(k).copied().unwrap_or(0),
        poa_w_m2: g(&r.hourly.poa_w_m2), t_cell_c: g(&r.hourly.t_cell_c), pv_dc_wh: g(&fl.pv_dc), pv_ac_wh: g(&fl.pv_ac), clipped_wh: g(&fl.clipped), load_wh: g(&fl.load), pv_to_load_wh: g(&fl.pv_to_load),
        batt_charge_wh: g(&fl.batt_charge), batt_discharge_wh: g(&fl.batt_discharge), soc_pct: g(&fl.soc_pct), backup_wh: g(&fl.backup_wh), self_wh: g(&fl.self_wh), grid_services_wh: g(&fl.grid_services_wh),
        import_wh: g(&fl.import), export_wh: g(&fl.export), curtailed_wh: g(&fl.curtailed), unserved_wh: g(&fl.unserved), outage: fl.outage.get(k).copied().unwrap_or(false), residual_wh: g(&fl.residual) }
}

/// Hours behind the totals: a range (`start`, `count`), one local day (`day_of_year` 1–365),
/// or the `top` hours by a channel (`import`, `export`, `clipped`, `curtailed`, `batt_discharge`, `unserved`, `-soc`).
pub fn results_hours(path: &Path, q: &Value) -> Result<Value> {
    let r = crate::latest_results(path)?.ok_or_else(|| e("NO_RESULTS", "No results yet. Run the simulation first."))?;
    let n = r.hourly.unix_mid.len();
    let idx: Vec<usize> = if let Some(d) = q.get("day_of_year").and_then(Value::as_u64) {
        let mut v: Vec<usize> = (0..n).filter(|&k| r.local_hour.get(k).is_some_and(|&h| h / 24 + 1 == d as u32)).collect();
        v.sort_by_key(|&k| r.local_hour[k]);
        v
    } else if let Some(ch) = q.get("top").and_then(Value::as_str) {
        let (neg, name) = ch.strip_prefix('-').map_or((false, ch), |c| (true, c));
        let fl = &r.flows;
        let col = match name { "import" => &fl.import, "export" => &fl.export, "clipped" => &fl.clipped, "curtailed" => &fl.curtailed, "batt_discharge" => &fl.batt_discharge, "batt_charge" => &fl.batt_charge, "unserved" => &fl.unserved, "soc" => &fl.soc_pct, "load" => &fl.load, "pv_ac" => &fl.pv_ac,
            _ => return Err(e("INVALID_ARGUMENT", format!("unknown channel '{name}'"))) };
        let mut v: Vec<usize> = (0..n).collect();
        v.sort_by(|&a, &b| { let (x, y) = (col[a], col[b]); if neg { x.total_cmp(&y) } else { y.total_cmp(&x) } });
        v.truncate(q.get("count").and_then(Value::as_u64).unwrap_or(24) as usize);
        v
    } else {
        let start = q.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
        let count = q.get("count").and_then(Value::as_u64).unwrap_or(24).min(744) as usize;
        (start.min(n)..(start + count).min(n)).collect()
    };
    let rows: Vec<HourRow> = idx.iter().map(|&k| hour_row(&r, k)).collect();
    let sum = |f: fn(&HourRow) -> f32| rows.iter().map(|x| f(x) as f64).sum::<f64>() / 1000.0;
    Ok(serde_json::json!({"hours": rows, "totals_kwh": {"pv_ac": sum(|x| x.pv_ac_wh), "load": sum(|x| x.load_wh), "import": sum(|x| x.import_wh), "export": sum(|x| x.export_wh),
        "batt_charge": sum(|x| x.batt_charge_wh), "batt_discharge": sum(|x| x.batt_discharge_wh), "clipped": sum(|x| x.clipped_wh), "curtailed": sum(|x| x.curtailed_wh)},
        "inputs_hash": r.manifest.inputs_hash}))
}

// ---------------------------------------------------------------- variants

pub fn create_variant(path: &Path, name: &str) -> Result<Variant> {
    let (mut s, mut p) = open(path)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let id = format!("var-{}", p.variants.len() + 1 + p.variants.iter().filter_map(|v| v.id.strip_prefix("var-").and_then(|x| x.parse::<usize>().ok())).max().unwrap_or(0));
    let v = Variant { id, name: name.to_string(), created_unix: now, design: p.design_snapshot() };
    p.variants.push(v.clone());
    save(&mut s, &p, &format!("variant {name}"))?;
    Ok(v)
}

/// Make a saved variant the active design (the current design is kept as a variant first if `keep_current`).
pub fn apply_variant(path: &Path, id: &str, keep_current: bool) -> Result<Project> {
    if keep_current { create_variant(path, "Before switching")?; }
    let (mut s, mut p) = open(path)?;
    let v = p.variants.iter().find(|v| v.id == id).cloned().ok_or_else(|| e("NOT_FOUND", format!("no variant '{id}'")))?;
    p.apply_snapshot(&v.design).map_err(|m| e("VARIANT_INVALID", m))?;
    save(&mut s, &p, &format!("apply variant {}", v.name))?;
    Ok(p)
}

pub fn delete_variant(path: &Path, id: &str) -> Result<()> {
    let (mut s, mut p) = open(path)?;
    let before = p.variants.len();
    p.variants.retain(|v| v.id != id);
    if p.variants.len() == before { return Err(e("NOT_FOUND", format!("no variant '{id}'"))); }
    save(&mut s, &p, &format!("delete variant {id}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantMetrics {
    pub id: String, pub name: String, pub dc_kwp: f64, pub battery_kwh: f64, pub ac_kwh: f64, pub specific_yield: f64, pub performance_ratio: f64,
    pub self_consumption_pct: f64, pub autarky_pct: f64, pub import_kwh: f64, pub export_kwh: f64, pub clipped_kwh: f64,
    pub investment: f64, pub npv: f64, pub irr_pct: Option<f64>, pub payback_years: Option<f64>, pub lcoe: f64, pub currency: String, pub inputs_hash: String, pub error: Option<String>,
}

/// Simulate the current design and every saved variant with their own pinned inputs.
pub fn compare_variants(path: &Path) -> Result<Vec<VariantMetrics>> {
    let (s, p) = open(path)?;
    let mut designs: Vec<(String, String, Project)> = vec![("current".into(), "Current design".into(), p.clone())];
    for v in &p.variants {
        let mut q = p.clone();
        q.apply_snapshot(&v.design).map_err(|m| e("VARIANT_INVALID", m))?;
        designs.push((v.id.clone(), v.name.clone(), q));
    }
    let mut out = vec![];
    for (id, name, q) in designs {
        let res = (|| -> Result<SimResult> {
            let (w, hash) = crate::load_weather(&s, &q)?;
            let series = load_series(&s, &q)?;
            solarcraft_engine::simulate(&q, &w, &hash, series.as_deref(), |_| true).map_err(|x| e("SIMULATION", x))
        })();
        let bkwh = q.storage.as_ref().and_then(|st| q.library.battery(&st.battery_id).map(|b| b.capacity_kwh * st.units as f64)).unwrap_or(0.0);
        out.push(match res {
            Ok(r) => { let f = r.finance.clone().unwrap_or_default(); VariantMetrics { id, name, dc_kwp: r.annual.dc_kwp, battery_kwh: bkwh, ac_kwh: r.annual.ac_kwh, specific_yield: r.annual.specific_yield_kwh_kwp, performance_ratio: r.annual.performance_ratio,
                self_consumption_pct: r.balance.self_consumption_rate * 100.0, autarky_pct: r.balance.autarky * 100.0, import_kwh: r.balance.import_kwh, export_kwh: r.balance.export_kwh, clipped_kwh: r.balance.clipped_kwh,
                investment: f.investment, npv: f.npv, irr_pct: f.irr_pct, payback_years: f.payback_years, lcoe: f.lcoe_per_kwh, currency: f.currency, inputs_hash: r.manifest.inputs_hash, error: None } }
            Err(x) => VariantMetrics { id, name, dc_kwp: 0.0, battery_kwh: bkwh, ac_kwh: 0.0, specific_yield: 0.0, performance_ratio: 0.0, self_consumption_pct: 0.0, autarky_pct: 0.0, import_kwh: 0.0, export_kwh: 0.0, clipped_kwh: 0.0,
                investment: 0.0, npv: 0.0, irr_pct: None, payback_years: None, lcoe: 0.0, currency: String::new(), inputs_hash: String::new(), error: Some(x.to_string()) },
        });
    }
    Ok(out)
}

/// Electrical design as simulated (manual or derived) with per-MPPT checks.
pub fn stringing(path: &Path) -> Result<Value> {
    let (_, p) = open(path)?;
    let e = solarcraft_model::derive::electrical(&p);
    let issues: Vec<_> = solarcraft_model::validate::validate(&p).into_iter().filter(|i| i.pointer.starts_with("/electrical") || i.code.starts_with("STRING") || i.code.starts_with("MPPT") || i.code.starts_with("INVERTER") || i.code == "DC_AC_RATIO" || i.code == "ROOF_CAPACITY" || i.code == "ARRAY_UNASSIGNED").collect();
    let inverters: Vec<Value> = e.inverters.iter().map(|u| {
        let inv = p.library.inverter(&u.inverter_id);
        serde_json::json!({"unit": u, "inverter": inv, "has_battery_port": inv.is_some_and(|i| i.battery_port.is_some())})
    }).collect();
    Ok(serde_json::json!({"manual": e.manual, "inverters": inverters, "issues": issues, "provenance": Provenance::User}))
}
