// SPDX-License-Identifier: GPL-3.0-or-later
//! Normalisation of catalog records into SolarCraft's equipment and tariff schema.
//! Source units are converted explicitly; `units` records the SolarCraft unit of every
//! normalised value and `issues` what was missing, a sentinel, clamped or unverified.
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub struct Norm { pub record: Option<Value>, pub units: BTreeMap<String, String>, pub issues: Vec<String> }

/// Families that produce records used in calculations.
pub fn technical(f: &str) -> bool { matches!(f, "module" | "pvinverter" | "batterysystem" | "batteryinverter" | "generator" | "feedintariff" | "netmeteringtariff" | "demandtariff") }

fn n(v: &Value, k: &str) -> Option<f64> { v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite()) }
fn s(v: &Value, k: &str) -> String { v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string() }
fn b(v: &Value, k: &str) -> Option<bool> { v.get(k).and_then(Value::as_bool) }

pub const CELL_TYPES: [(i64, &str); 12] = [(0, "mono"), (1, "poly"), (2, "amorphous"), (3, "EFG"), (4, "Apex"), (5, "Ribbon"), (6, "HIT"), (7, "CIS"), (8, "CdTe"), (9, "triple amorphous"), (10, "microcrystalline"), (99, "other")];
pub const BATTERY_TYPES: [(i64, &str); 10] = [(101, "vented lead-acid"), (102, "gel lead-acid"), (103, "AGM"), (201, "LFP"), (202, "lithium nickel oxide"), (203, "lithium nickel cobalt oxide"), (204, "lithium titanate"), (205, "lithium nickel cobalt oxide titanate"), (206, "NMC/graphite"), (301, "saltwater")];
pub const COUPLINGS: [(i64, &str); 4] = [(0, "unknown"), (1, "AC"), (2, "DC on generator side"), (3, "DC on intermediate link")];
fn label(t: &[(i64, &str)], c: Option<i64>) -> String { c.and_then(|c| t.iter().find(|x| x.0 == c).map(|x| x.1.to_string())).unwrap_or_else(|| format!("unknown code {c:?}")) }

fn curve(v: &Value, issues: &mut Vec<String>) -> Vec<[f64; 2]> {
    if let Some(e0) = n(v, "e0").filter(|x| *x != 0.0) { issues.push(format!("e0 = {e0} preserved; curve starts at the 5 % knot")); }
    [(0.05, "e5"), (0.10, "e10"), (0.20, "e20"), (0.30, "e30"), (0.50, "e50"), (0.75, "e75"), (1.0, "e100")].iter()
        .filter_map(|(x, k)| n(v, k).filter(|e| *e > 0.0 && *e <= 1.0).map(|e| [*x, e])).collect()
}

fn company_name(company: Option<&Value>) -> String { company.map(|c| s(c, "name")).filter(|x| !x.is_empty()).unwrap_or_else(|| "Unknown".into()) }

fn base(v: &Value, company: Option<&Value>, sha: &str, ds: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("id".into(), json!(format!("cat-{}-v{}", s(v, "id"), v["version"].as_i64().unwrap_or(0))));
    m.insert("manufacturer".into(), json!(company_name(company)));
    m.insert("model".into(), json!(s(v, "name")));
    m.insert("provenance".into(), json!("catalog"));
    m.insert("datasheet_ref".into(), json!(format!("catalog:{}:{ds}:{}@{}", &sha[..12.min(sha.len())], s(v, "id"), v["version"])));
    m
}

pub fn normalize(f: &str, v: &Value, linked_battery: Option<&Value>, company: Option<&Value>, country: Option<&Value>, sha: &str, ds: &str) -> Norm {
    let mut units = BTreeMap::new();
    let mut issues = vec![];
    let mut u = |k: &str, unit: &str| { units.insert(k.to_string(), unit.to_string()); };
    let record = match f {
        "module" => (|| {
            let mut m = base(v, company, sha, ds);
            let cells = n(v, "cellCount")?;
            let voc = n(v, "openCircuitVoltage")?;
            let half = b(v, "isHalfCellModule").unwrap_or(false);
            // Effective series cells: the catalog count, halved for half-cell modules whose count is per half-cell
            // (identified by an implausible Voc per cell below 0.5 V for crystalline cells).
            let series = if half && voc / cells < 0.5 { issues.push(format!("half-cell count {cells} halved to {} series cells (Voc/cell {:.2} V)", cells / 2.0, voc / cells)); cells / 2.0 } else { cells };
            m.insert("technology".into(), json!(match v["cellType"].as_i64() { Some(0) | Some(1) | Some(6) | Some(10) => "c-Si", _ => "thin-film/other" }));
            m.insert("cells_in_series".into(), json!(series.round() as u32));
            m.insert("length_m".into(), json!(n(v, "height")? / 1000.0)); m.insert("width_m".into(), json!(n(v, "width")? / 1000.0));
            for (dst, src) in [("v_oc", "openCircuitVoltage"), ("i_sc", "shortCircuitCurrent"), ("v_mp", "mpP_Voltage"), ("i_mp", "mpP_Current")] { m.insert(dst.into(), json!(n(v, src)?)); }
            m.insert("alpha_sc".into(), json!(n(v, "currentCoefficient")? / 1000.0));
            m.insert("beta_voc".into(), json!(n(v, "voltageCoefficient")? / 1000.0));
            m.insert("gamma_pmp".into(), json!(n(v, "outputpowerCoefficient")? * 100.0));
            m.insert("model_params".into(), Value::Null);
            m.insert("nameplate_w".into(), json!(n(v, "specifiedOutputPower")));
            m.insert("max_system_voltage_v".into(), json!(n(v, "maxSysVoltage").filter(|x| *x > 0.0)));
            m.insert("bypass_diodes".into(), json!(n(v, "diodeCount").filter(|x| *x > 0.0).map(|x| x as u32)));
            m.insert("bypass_diode_voltage_v".into(), json!(n(v, "lossVoltagePerBypassDiode")));
            m.insert("half_cell".into(), json!(half));
            m.insert("voc_initial_increase".into(), json!(n(v, "increaseU0").filter(|x| *x != 0.0)));
            m.insert("requires_transformer_inverter".into(), json!(b(v, "onlyTransformerInverter").unwrap_or(false)));
            m.insert("bifaciality".into(), json!(n(v, "bifacialityFactor").filter(|x| *x > 0.0)));
            m.insert("thickness_m".into(), json!(n(v, "depthOfPanel").filter(|x| *x > 0.0).map(|x| x / 1000.0)));
            m.insert("weight_kg".into(), json!(n(v, "weight").filter(|x| *x > 0.0)));
            m.insert("description".into(), json!(s(v, "description")));
            if let (Some(g), Some(vm), Some(im), Some(vo), Some(is)) = (n(v, "g2"), n(v, "umpp2"), n(v, "impp2"), n(v, "uoc2"), n(v, "isc2")) {
                if g > 0.0 && vm > 0.0 && im > 0.0 { m.insert("low_light".into(), json!({"irradiance_w_m2": g, "v_mp": vm, "i_mp": im, "v_oc": vo, "i_sc": is})); } else { issues.push("second (part-load) point incomplete: impp2 or umpp2 is 0".into()); }
            }
            let mut unv = Map::new();
            for k in ["cellType", "iam", "standardCellConnection", "standardPartLoadOperation", "widthOfFrame", "isAvailable", "isCorrupt", "productType", "dbEntryType"] { if v.get(k).is_some() { unv.insert(k.into(), v[k].clone()); } }
            unv.insert("cell_type".into(), json!(label(&CELL_TYPES, v["cellType"].as_i64())));
            m.insert("unverified".into(), Value::Object(unv));
            Some(Value::Object(m))
        })(),
        "pvinverter" => (|| {
            let mut m = base(v, company, sha, ds);
            let mut inputs = vec![];
            for t in 1..=3 {
                let cnt = n(v, &format!("mppT{t}_NrOf")).unwrap_or(0.0) as usize;
                for _ in 0..cnt {
                    inputs.push(json!({"v_min": n(v, &format!("mppT{t}_U_MPP_Min"))?, "v_max": n(v, &format!("mppT{t}_U_MPP_Max"))?, "i_max": n(v, &format!("mppT{t}_MaxCurrent"))?,
                        "i_sc_max": n(v, &format!("mppT{t}_MaxShortCircuitCurrent")).filter(|x| *x > 0.0), "p_max_w": n(v, &format!("mppT{t}_MaxPower")).filter(|x| *x > 0.0)}));
                }
            }
            if inputs.is_empty() { issues.push("no MPPT group with trackers".into()); return None; }
            let c = curve(v, &mut issues);
            if c.is_empty() { issues.push("no conversion curve".into()); return None; }
            let col = |k: &str| inputs.iter().filter_map(|i| i[k].as_f64()).collect::<Vec<_>>();
            m.insert("pac_max_w".into(), json!(n(v, "maxAC_Power")?));
            m.insert("eta_nominal".into(), json!(c.iter().map(|x| x[1]).fold(0.0, f64::max)));
            m.insert("efficiency_curve".into(), json!(c));
            m.insert("mppt_v_min".into(), json!(col("v_min").into_iter().fold(f64::INFINITY, f64::min)));
            m.insert("mppt_v_max".into(), json!(col("v_max").into_iter().fold(0.0, f64::max)));
            m.insert("v_dc_max".into(), json!(n(v, "maxInputVoltageDC")?));
            m.insert("mppt_count".into(), json!(inputs.len()));
            m.insert("i_max_per_mppt".into(), json!(col("i_max").into_iter().fold(0.0, f64::max)));
            m.insert("night_w".into(), json!(n(v, "nightConsumption").unwrap_or(0.0)));
            m.insert("standby_w".into(), json!(n(v, "standbyConsumption")));
            m.insert("pdc_max_w".into(), json!(n(v, "maxDC_Power").filter(|x| *x > 0.0)));
            m.insert("pdc_nominal_w".into(), json!(n(v, "dC_PowerRating").filter(|x| *x > 0.0)));
            m.insert("v_dc_nominal".into(), json!(n(v, "nominalVoltageDC").filter(|x| *x > 0.0)));
            m.insert("phases".into(), json!(n(v, "numberOfPhases").map(|x| x as u32)));
            m.insert("transformer".into(), json!(b(v, "withTransformer")));
            m.insert("start_power_w".into(), json!(n(v, "feedInFrom").filter(|x| *x > 0.0)));
            m.insert("eta_change_per_100v".into(), json!(n(v, "changeOfEtaDependentOnVoltage").filter(|x| *x != 0.0)));
            m.insert("mpp_matching".into(), match (n(v, "matchingEtaMPP20"), n(v, "matchingEtaMPP100")) { (Some(a), Some(c2)) if a > 0.0 && c2 > 0.0 => json!([a, c2]), _ => Value::Null });
            m.insert("mppt_interconnection".into(), json!(n(v, "parallelOperating").map(|x| x as u32)));
            m.insert("i_dc_max".into(), json!(n(v, "maxCurrentDC").filter(|x| *x > 0.0)));
            m.insert("i_sc_dc_max".into(), json!(n(v, "maxShortCircuitCurrentDC").filter(|x| *x > 0.0)));
            m.insert("dc_connectors".into(), json!(n(v, "dC_InputCount").filter(|x| *x > 0.0).map(|x| x as u32)));
            m.insert("v_ac".into(), json!(n(v, "voltageAC")));
            m.insert("inputs".into(), json!(inputs));
            let mut unv = Map::new();
            for k in ["aC_PowerRating", "e0", "isAvailable", "isCorrupt", "productType", "dbEntryType"] { if v.get(k).is_some() { unv.insert(k.into(), v[k].clone()); } }
            m.insert("unverified".into(), Value::Object(unv));
            Some(Value::Object(m))
        })(),
        "batterysystem" => (|| {
            let mut m = base(v, company, sha, ds);
            let bat = linked_battery?;
            let per_string = n(v, "numberOfBatteriesPerString").unwrap_or(1.0).max(1.0);
            let strings = n(v, "numberOfBatteryStrings").unwrap_or(1.0).max(1.0);
            let cell_v = n(bat, "ratedVoltage")?;
            let cap: Vec<[f64; 2]> = [(0.167, "capacityAt0167"), (0.5, "capacityAt05"), (1.0, "capacityAt1"), (5.0, "capacityAt5"), (10.0, "capacityAt10"), (100.0, "capacityAt100")]
                .iter().filter_map(|(h, k)| n(bat, k).filter(|x| *x > 0.0).map(|x| [*h, x])).collect();
            let mut cyc = vec![];
            for (d, k) in [(20.0, "lifeCycles20"), (40.0, "lifeCycles40"), (60.0, "lifeCycles60"), (80.0, "lifeCycles80")] {
                match n(bat, k) { Some(-1.0) => issues.push(format!("{k} = −1 (point missing)")), Some(x) if x > 0.0 => cyc.push([d, x]), _ => {} }
            }
            let cells = solarcraft_model::equipment::BatteryCells { chemistry_code: bat["batteryType"].as_u64().map(|x| x as u32), cell_voltage_v: cell_v,
                series: (n(bat, "numberOfCellsSerie")?.max(1.0) * per_string) as u32, parallel: n(bat, "numberOfCellsParallel")?.max(1.0) as u32, strings: strings as u32,
                // internalResistance is per catalog battery (block); convert it to one cell.
                internal_resistance_ohm: n(bat, "internalResistance").unwrap_or(0.0) * n(bat, "numberOfCellsParallel")?.max(1.0) / n(bat, "numberOfCellsSerie")?.max(1.0),
                capacity_ah_by_hours: cap, cycles_by_dod: cyc };
            let kwh = cells.energy_kwh();
            if kwh <= 0.0 { issues.push("no capacity at the 10-hour rate".into()); return None; }
            let code = bat["batteryType"].as_i64();
            let chem = match code { Some(101..=199) => "lead_acid", Some(201) => "lfp", Some(202..=206) => "nmc", _ => "other" };
            let lead = chem == "lead_acid";
            let soc_min = n(v, "socMin").unwrap_or(0.05);
            if soc_min < 0.05 { issues.push(format!("socMin {soc_min} below the traced 0.05 strategy floor; using 0.05")); }
            let (p_ch, p_dis) = (n(v, "maxCharge")?, n(v, "maxDischarge")?);
            m.insert("chemistry".into(), json!(chem));
            m.insert("capacity_kwh".into(), json!(kwh));
            m.insert("nominal_voltage_v".into(), json!(cells.pack_voltage()));
            m.insert("soc_min_pct".into(), json!(soc_min.max(0.05) * 100.0));
            m.insert("soc_max_pct".into(), json!(100.0));
            m.insert("p_charge_max_kw".into(), json!(p_ch / 1000.0)); m.insert("p_discharge_max_kw".into(), json!(p_dis / 1000.0));
            m.insert("roundtrip_efficiency".into(), json!(if lead { 0.85 } else { 0.95 }));
            issues.push("round-trip cell efficiency not in source; chemistry default used".into());
            m.insert("self_discharge_pct_per_month".into(), json!(n(bat, "selfdischarge").unwrap_or(0.02) * 100.0));
            m.insert("cycle_life".into(), json!(cells.cycles_at(80.0).unwrap_or(if lead { 1500.0 } else { 6000.0 })));
            m.insert("cells".into(), json!(cells));
            m.insert("converter".into(), json!({"p_rated_w": n(v, "nominalPower").unwrap_or(p_dis), "p_charge_max_w": p_ch, "p_discharge_max_w": p_dis, "efficiency_curve": curve(v, &mut issues), "standby_w": 0.0}));
            let coupling = v["coupling"].as_i64();
            let mut unv = Map::new();
            unv.insert("coupling".into(), json!({"code": coupling, "meaning": label(&COUPLINGS, coupling), "solarcraft": match coupling { Some(1) => "integrated", Some(2) | Some(3) => "dc_hybrid (choose the hybrid inverter)", _ => "choose" }}));
            unv.insert("battery_type".into(), json!(label(&BATTERY_TYPES, code)));
            for k in ["socBoostStart", "socBoostEnd", "socFloatStart", "socFullEnd", "socEqualEnd", "boostDuration", "durationFull", "durationEqualisation", "cycleFull", "cycleEqualisation", "e0"] { if v.get(k).is_some() { unv.insert(k.into(), v[k].clone()); } }
            for k in ["length", "width", "height", "weight"] { if bat.get(k).is_some() { unv.insert(format!("battery.{k}"), bat[k].clone()); } }
            m.insert("unverified".into(), Value::Object(unv));
            Some(Value::Object(m))
        })(),
        "batteryinverter" => (|| {
            let mut m = base(v, company, sha, ds);
            m.insert("p_ac_max_w".into(), json!(n(v, "aC_RatedPower_25C")?));
            m.insert("efficiency_curve".into(), json!(curve(v, &mut issues)));
            m.insert("standby_w".into(), json!(n(v, "consumption_Standby").unwrap_or(0.0)));
            m.insert("battery_v_min".into(), json!(n(v, "batt_MinInputVoltage").filter(|x| *x > 0.0)));
            m.insert("battery_v_max".into(), json!(n(v, "batt_MaxInputVoltage").filter(|x| *x > 0.0)));
            m.insert("battery_i_max".into(), json!(n(v, "batt_MaxCurrent").filter(|x| *x > 0.0)));
            let mut unv = Map::new();
            for k in ["aC_RatedVoltage", "aC_30MinPower", "aC_60MinPower", "aC_MaxGenCurrent", "batt_RatedInputVoltage", "consumption_WithoutLoad", "maxNumberParallel1Phase", "maxNumberParallel2Phase", "maxNumberParallel3Phase", "e0"] { if v.get(k).is_some() { unv.insert(k.into(), v[k].clone()); } }
            m.insert("unverified".into(), Value::Object(unv));
            Some(Value::Object(m))
        })(),
        "generator" => (|| {
            let mut m = base(v, company, sha, ds);
            m.insert("rated_va".into(), json!(n(v, "aC_RatedPower")?)); m.insert("max_va".into(), json!(n(v, "aC_MaxPower")?));
            m.insert("phases".into(), json!(n(v, "numberOfPhases").unwrap_or(1.0) as u32)); m.insert("voltage_v".into(), json!(n(v, "aC_RatedVoltage").unwrap_or(0.0)));
            m.insert("fuel_curve".into(), json!([0.0, 0.25, 0.5, 0.75, 1.0].iter().zip(["fuelConsumptionAt0", "fuelConsumptionAt25", "fuelConsumptionAt50", "fuelConsumptionAt75", "fuelConsumptionAt100"]).filter_map(|(x, k)| n(v, k).map(|y| [*x, y])).collect::<Vec<_>>()));
            m.insert("fuel_unit".into(), json!("source fuel volume per hour (unit unverified)"));
            issues.push("apparent power used as real power (unity power factor)".into());
            Some(Value::Object(m))
        })(),
        "feedintariff" | "netmeteringtariff" | "demandtariff" => {
            let decoded = v["htNtJson"].as_str().filter(|x| !x.trim().is_empty()).map(|h| crate::tariff::decode_htnt(h).map_err(|e| e.to_string()));
            if let Some(Err(e)) = &decoded { issues.push(format!("htNtJson not decoded: {e}")); }
            Some(json!({"tariff_family": f, "name": s(v, "name"), "utility": company_name(company), "city": s(v, "city"),
                "country": country.map(|c| json!({"id": c["id"], "name": c["name"], "iso3": c["isoCode"], "iso2": c["isoCode2"], "es": c["esp"]})),
                "valid_from": v["validFrom"], "customer": match v["typeOfCustomer"].as_i64() { Some(0) => "residential", Some(1) => "commercial", _ => "unknown" },
                "zoning": v["zoning"], "is_net_metering": v["isNetMetering"], "is_demand": v["isDemandTariff"], "is_dynamic": v["isDynamic"], "period": v["periodicDuration"],
                "concepts": crate::tariff::concepts(v), "schedule": decoded.and_then(|d| d.ok()), "description": s(v, "description")}))
        }
        "company" => Some(json!({"name": s(v, "name"), "short_name": v["shortName"], "website": v["website"]})),
        "country" => Some(json!({"name": s(v, "name"), "iso3": s(v, "isoCode"), "iso2": s(v, "isoCode2"), "names": {"de": v["deu"], "fr": v["fra"], "en": v["eng"], "es": v["esp"], "pl": v["pol"], "it": v["ita"], "pt": v["prt"], "cs": v["ces"], "tr": v["tur"]}})),
        "battery" => Some(json!({"component": "battery product (used through battery systems)", "manufacturer": company_name(company), "model": s(v, "name"), "type": label(&BATTERY_TYPES, v["batteryType"].as_i64()),
            "cell_voltage_v": v["ratedVoltage"], "cells_series": v["numberOfCellsSerie"], "cells_parallel": v["numberOfCellsParallel"], "capacity_10h_ah": v["capacityAt10"], "dimensions_m": [v["length"], v["width"], v["height"]], "weight_kg": v["weight"]})),
        "optimizer" => Some(json!({"manufacturer": company_name(company), "model": s(v, "name"), "mode": match v["powerOptimizerMode"].as_i64() { Some(0) => "Full", Some(1) => "Buck", Some(2) => "SubstringBuck", Some(3) => "SubstringFull", _ => "unknown" },
            "rated_dc_w": v["ratedDCOutputPower"], "input_v_max": v["maxInputVoltage"], "mpp_v": [v["u_MPP_Min"], v["u_MPP_Max"]], "module_integrated": v["moduleIntegrated"]})),
        "ev" => Some(json!({"manufacturer": company_name(company), "model": s(v, "name"), "battery_kwh": v["akkuKapazitaet"], "consumption_kwh_100km": v["verbrauchInKwhPro100km"], "charge_w": v["ladeLeistung"], "can_discharge": v["canDischarge"]})),
        _ => None,
    };
    for (k, unit) in [("length_m", "m"), ("width_m", "m"), ("alpha_sc", "A/K"), ("beta_voc", "V/K"), ("gamma_pmp", "%/K"), ("v_oc", "V"), ("i_sc", "A"), ("v_mp", "V"), ("i_mp", "A"),
        ("max_system_voltage_v", "V"), ("pac_max_w", "W"), ("pdc_nominal_w", "W"), ("v_dc_max", "V"), ("start_power_w", "W DC"), ("eta_change_per_100v", "fraction per 100 V"),
        ("capacity_kwh", "kWh"), ("p_charge_max_kw", "kW"), ("p_discharge_max_kw", "kW"), ("soc_min_pct", "%"), ("self_discharge_pct_per_month", "%/month"), ("rated_va", "VA"), ("thickness_m", "m")] {
        if record.as_ref().is_some_and(|r| r.get(k).is_some_and(|x| !x.is_null())) { u(k, unit); }
    }
    Norm { record, units, issues }
}

/// One-line summary for search results.
pub fn summary(rec: &Value) -> Value {
    if let Some(p) = rec["v_mp"].as_f64() { return json!(format!("{:.0} W, {:.2} × {:.2} m", p * rec["i_mp"].as_f64().unwrap_or(0.0), rec["length_m"].as_f64().unwrap_or(0.0), rec["width_m"].as_f64().unwrap_or(0.0))); }
    if let Some(p) = rec["pac_max_w"].as_f64() { return json!(format!("{:.1} kW AC, {} MPPT", p / 1000.0, rec["mppt_count"])); }
    if let Some(k) = rec["capacity_kwh"].as_f64() { return json!(format!("{:.1} kWh, {:.1} kW", k, rec["p_discharge_max_kw"].as_f64().unwrap_or(0.0))); }
    if let Some(p) = rec["p_ac_max_w"].as_f64() { return json!(format!("{:.1} kW battery inverter", p / 1000.0)); }
    if let Some(p) = rec["rated_va"].as_f64() { return json!(format!("{:.1} kVA generator", p / 1000.0)); }
    if rec["tariff_family"].is_string() { return json!(format!("{} · {} · from {}", rec["utility"].as_str().unwrap_or(""), rec["country"]["name"].as_str().unwrap_or(""), rec["valid_from"].as_str().unwrap_or(""))); }
    Value::Null
}
