// SPDX-License-Identifier: GPL-3.0-or-later
#![recursion_limit = "512"]
//! Catalog import end to end. Backups are built here with the exporter's schema and fictional
//! records shaped like catalog entries, so CI needs no vendor data. Each test changes an imported
//! field and checks the downstream result through the same JSON API the apps use.
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use solarcraft_app::api::invoke_json;
use std::path::{Path, PathBuf};

fn ok(m: &str, p: Value) -> Value {
    let r: Value = serde_json::from_str(&invoke_json(&json!({"method": m, "params": p}).to_string(), &mut |_| true)).unwrap();
    assert_eq!(r["ok"], json!(true), "{m}: {r}");
    r["result"].clone()
}
fn err(m: &str, p: Value) -> Value { serde_json::from_str::<Value>(&invoke_json(&json!({"method": m, "params": p}).to_string(), &mut |_| true)).unwrap() }
fn num(v: &Value, p: &str) -> f64 { v.pointer(p).and_then(Value::as_f64).unwrap_or_else(|| panic!("{p} in {v}")) }

/// Write a backup with the exporter schema: datasets, records (record_json), api_responses, …
fn backup(dir: &Path, name: &str, recs: &[(&str, Value)]) -> PathBuf {
    let p = dir.join(name);
    let _ = std::fs::remove_file(&p);
    let c = Connection::open(&p).unwrap();
    c.execute_batch("CREATE TABLE backup_info(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE datasets(id INTEGER PRIMARY KEY, source TEXT NOT NULL, name TEXT NOT NULL UNIQUE, columns_json TEXT NOT NULL, row_count INTEGER NOT NULL);
        CREATE TABLE records(dataset_id INTEGER NOT NULL, row_number INTEGER NOT NULL, entry_id TEXT, entry_version TEXT, name TEXT, record_json TEXT NOT NULL, PRIMARY KEY(dataset_id, row_number));
        CREATE TABLE local_schema(table_name TEXT, position INTEGER, column_name TEXT, sql_type TEXT);
        CREATE TABLE export_log(source TEXT, route_or_table TEXT, dataset_name TEXT, exported INTEGER, catalog_count INTEGER, status TEXT NOT NULL);
        CREATE TABLE api_responses(id INTEGER PRIMARY KEY, route TEXT NOT NULL, body_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL, body BLOB NOT NULL);
        INSERT INTO backup_info VALUES('exported_at', '2026-11-04T20:53:11+00:00');").unwrap();
    let mut names: Vec<&str> = vec![];
    for r in recs { if !names.contains(&r.0) { names.push(r.0); } }
    for (i, n) in names.iter().enumerate() {
        let rows: Vec<&Value> = recs.iter().filter(|r| r.0 == *n).map(|r| &r.1).collect();
        let cols: Vec<String> = rows[0].as_object().unwrap().keys().cloned().collect();
        c.execute("INSERT INTO datasets VALUES(?1, 'Online', ?2, ?3, ?4)", params![i as i64 + 1, n, serde_json::to_string(&cols).unwrap(), rows.len() as i64]).unwrap();
        for (k, r) in rows.iter().enumerate() {
            c.execute("INSERT INTO records VALUES(?1, ?2, ?3, ?4, ?5, ?6)", params![i as i64 + 1, k as i64 + 1, r["id"].as_str(), r["version"].to_string(), r["name"].as_str(), r.to_string()]).unwrap();
        }
    }
    let body = b"{\"archived\":true}";
    c.execute("INSERT INTO api_responses VALUES(1, 'metadata/test', ?1, ?2, ?3)", params![body.len() as i64, solarcraft_store::sha256_hex(body), &body[..]]).unwrap();
    p
}

fn company() -> Value { json!({"id": "c1", "version": 1, "name": "Fictional Solar GmbH", "shortName": "FS", "website": "https://example.invalid", "dbEntryType": 301, "description": "", "publishedAt": "2024-01-01T00:00:00Z", "graphic": null, "dbUserId": null}) }
fn country() -> Value { json!({"id": "co1", "name": "Colombia", "updatedAt": "2024-01-01", "isoCode": "COL", "isoCode2": "CO", "deu": "Kolumbien", "fra": "Colombie", "eng": "Colombia", "esp": "Colombia", "pol": "Kolumbia", "ita": "Colombia", "prt": "Colômbia", "ces": "Kolumbie", "tur": "Kolombiya"}) }
fn module(version: i64, vmp: f64, voc: f64, height_mm: f64, inc_u0: f64) -> Value {
    json!({"id": "m1", "version": version, "name": format!("FS-450 rev{version}"), "companyId": "c1", "company": company(), "dbUserId": null, "cellType": 0, "cellCount": 72, "diodeCount": 3, "lossVoltagePerBypassDiode": 0.5,
        "isHalfCellModule": false, "standardCellConnection": true, "onlyTransformerInverter": false, "powerOptimizerId": null, "powerOptimizerVersion": null, "powerOptimizer": null,
        "specifiedOutputPower": 450, "mpP_Voltage": vmp, "mpP_Current": 10.87, "openCircuitVoltage": voc, "shortCircuitCurrent": 11.6, "increaseU0": inc_u0, "standardPartLoadOperation": true,
        "g2": 200, "umpp2": vmp - 1.0, "impp2": 2.17, "uoc2": voc - 2.0, "isc2": 2.32, "voltageCoefficient": -134, "currentCoefficient": 5.8, "outputpowerCoefficient": -0.0035, "iam": 0.95,
        "maxSysVoltage": 1000, "bifacialityFactor": null, "width": 1130, "height": height_mm, "depthOfPanel": 35, "widthOfFrame": 11, "weight": 24.5, "productType": 8,
        "isAvailable": true, "isCorrupt": null, "dbEntryType": 108, "description": "fictional", "publishedAt": "2024-02-01T00:00:00Z", "graphic": null})
}
fn inverter(pac: f64, feed_in_from: f64, parallel: i64) -> Value {
    json!({"id": "i1", "version": 1, "name": "FS-INV 5", "companyId": "c1", "company": company(), "dC_PowerRating": 5200, "maxDC_Power": 7500, "nominalVoltageDC": 360, "maxInputVoltageDC": 600, "maxCurrentDC": 52, "maxShortCircuitCurrentDC": 60,
        "dC_InputCount": 4, "aC_PowerRating": pac, "maxAC_Power": pac, "voltageAC": 230, "numberOfPhases": 1, "withTransformer": false, "changeOfEtaDependentOnVoltage": 0.005, "feedInFrom": feed_in_from,
        "standbyConsumption": 2, "nightConsumption": 1, "matchingEtaMPP20": 0.99, "matchingEtaMPP100": 0.999, "parallelOperating": parallel,
        "mppT1_NrOf": 2, "mppT1_MaxCurrent": 26, "mppT1_MaxShortCircuitCurrent": 32, "mppT1_MaxPower": 4000, "mppT1_U_MPP_Min": 120, "mppT1_U_MPP_Max": 500,
        "mppT2_NrOf": 0, "mppT2_MaxCurrent": 0, "mppT2_MaxShortCircuitCurrent": 0, "mppT2_MaxPower": 0, "mppT2_U_MPP_Min": 0, "mppT2_U_MPP_Max": 0,
        "mppT3_NrOf": 0, "mppT3_MaxCurrent": 0, "mppT3_MaxShortCircuitCurrent": 0, "mppT3_MaxPower": 0, "mppT3_U_MPP_Min": 0, "mppT3_U_MPP_Max": 0,
        "e0": 0, "e5": 0.9, "e10": 0.94, "e20": 0.96, "e30": 0.968, "e50": 0.972, "e75": 0.97, "e100": 0.967, "productType": 6, "isAvailable": true, "isCorrupt": null, "dbEntryType": 106, "description": "", "publishedAt": "2024-02-01T00:00:00Z", "graphic": null, "dbUserId": null})
}
fn battery(version: i64, ah10: f64) -> Value {
    json!({"id": "b1", "version": version, "name": "FS-CELL", "companyId": "c1", "company": company(), "batteryType": 201, "ratedVoltage": 3.2, "numberOfCellsSerie": 16, "numberOfCellsParallel": 1, "internalResistance": 0.001,
        "selfdischarge": 0.03, "lifeCycles20": -1, "lifeCycles40": 15000, "lifeCycles60": 9000, "lifeCycles80": 6000, "capacityAt0167": ah10 * 0.9, "capacityAt05": ah10 * 0.95, "capacityAt1": ah10 * 0.99, "capacityAt5": ah10, "capacityAt10": ah10, "capacityAt100": ah10 * 1.01,
        "length": 0.48, "width": 0.46, "height": 0.13, "weight": 46, "productType": 1, "isAvailable": true, "isCorrupt": null, "dbEntryType": 101, "description": "", "publishedAt": "2024-02-01T00:00:00Z", "graphic": null, "dbUserId": null})
}
fn system(id: &str, bver: i64, embedded: Value, per_string: i64, strings: i64, coupling: i64) -> Value {
    json!({"id": id, "version": 1, "name": format!("FS-STORE {id}"), "companyId": "c1", "company": company(), "nominalPower": 5000, "maxCharge": 5000, "maxDischarge": 5000, "coupling": coupling, "socMin": 0.04,
        "socBoostStart": 1, "socBoostEnd": 1, "socFloatStart": 1, "socFullEnd": 1, "socEqualEnd": 1, "boostDuration": 2, "durationFull": 5, "durationEqualisation": 10, "cycleFull": 14, "cycleEqualisation": 180,
        "battery": embedded, "batteryId": "b1", "batteryVersion": bver, "numberOfBatteriesPerString": per_string, "numberOfBatteryStrings": strings,
        "e0": 0.8, "e5": 0.85, "e10": 0.9, "e20": 0.93, "e30": 0.95, "e50": 0.96, "e75": 0.96, "e100": 0.955, "productType": 2, "isAvailable": true, "isCorrupt": null, "dbEntryType": 102, "description": "", "publishedAt": "2024-02-01T00:00:00Z", "graphic": null, "dbUserId": null})
}
fn generator(fuel100: f64) -> Value {
    json!({"id": "g1", "version": 1, "name": "FS-GEN 3", "companyId": "c1", "company": company(), "aC_RatedVoltage": 230, "aC_RatedCurrent": 13, "aC_RatedPower": 3000, "aC_MaxPower": 3300, "numberOfPhases": 1,
        "fuelConsumptionAt0": 0, "fuelConsumptionAt25": 0.4 * fuel100 / 1.3, "fuelConsumptionAt50": 0.7 * fuel100 / 1.3, "fuelConsumptionAt75": 1.0 * fuel100 / 1.3, "fuelConsumptionAt100": fuel100, "productType": 5, "isAvailable": true, "isCorrupt": null, "dbEntryType": 105, "description": "", "publishedAt": "2024-02-01T00:00:00Z", "graphic": null, "dbUserId": null})
}
fn tariff(kind: &str, id: &str, energy: f64, peak: f64, base: f64) -> Value {
    let mut weekday = vec![1u32; 288];
    for m in 0..12 { for h in 17..20 { weekday[m * 24 + h] = 2; } }
    let sched = json!({"$id": "1", "WochenTage": ["b", "a", "a", "a", "a", "a", "b"], "TimePlusXMinutes": 30, "TarriffZoneDefList": [weekday, vec![1u32; 288]],
        "EnergyTariffRanges": [{"$id": "2", "Tarifname": "Off-peak", "RangeValues": [{"$id": "3", "Id": "r1", "Key": 0.0, "Range": 100.0, "Value": energy}]}, {"$id": "4", "Tarifname": "Peak", "RangeValues": [{"$id": "5", "Id": "r2", "Key": 0.0, "Range": 100.0, "Value": peak}]}]});
    let feed = kind == "Online feedintariffs";
    let concepts = if feed { json!([{"id": "tc1", "concept": 0, "tariffUnit": "kWh", "isExpense": false, "tariffId": id, "tariffVersion": 1, "tariffRangeValues": [{"id": "rv1", "key": 0, "range": 0, "value": energy, "tariffConceptId": "tc1"}, {"id": "rv2", "key": 10, "range": 0, "value": energy / 2.0, "tariffConceptId": "tc1"}]}]) }
        else { json!([{"id": "tc0", "concept": 0, "tariffUnit": "Month", "isExpense": true, "tariffId": id, "tariffVersion": 1, "tariffRangeValues": [{"id": "rv0", "key": 0, "range": 0, "value": base, "tariffConceptId": "tc0"}]},
                     {"id": "tc2", "concept": 2, "tariffUnit": "kWh", "isExpense": true, "tariffId": id, "tariffVersion": 1, "tariffRangeValues": [{"id": "rv3", "key": 0, "range": 0, "value": energy, "tariffConceptId": "tc2"}]}]) };
    json!({"id": id, "version": 1, "name": format!("Tariff {id}"), "companyId": "c1", "company": company(), "city": "Bogotá", "countryId": "co1", "country": country(), "payoutDuration": if feed { 20 } else { 0 },
        "validFrom": "2025-01-01T00:00:00", "zoning": false, "isDemandTariff": !feed, "typeOfCustomer": 0, "installationType": "", "isNetMetering": false, "periodicDuration": "Monatlich", "saldoRollover": false,
        "thresholdType_1": "none", "thresholdValue_1": null, "htNtJson": if feed { Value::Null } else { json!(solarcraft_catalog::tariff::encode_htnt(&sched)) }, "minimumPrice": null, "minimumPriceUnitId": null, "isDynamic": false,
        "tariffConcepts": concepts, "dbEntryType": if feed { 201 } else { 203 }, "description": "", "publishedAt": "2024-02-01T00:00:00Z", "graphic": null, "dbUserId": null})
}

struct Fixture { dir: PathBuf }
impl Fixture {
    fn new(tag: &str) -> Self { let d = std::env::temp_dir().join(format!("sc-cat-{tag}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); Fixture { dir: d } }
    fn s(&self, n: &str) -> String { self.dir.join(n).to_str().unwrap().to_string() }
    /// Import a backup into a fresh catalog and return its path.
    fn catalog(&self, n: &str, recs: &[(&str, Value)]) -> String {
        let b = backup(&self.dir, &format!("{n}.backup.sqlite"), recs);
        let c = self.s(&format!("{n}.catalog.sqlite"));
        let _ = std::fs::remove_file(&c);
        ok("catalog_import", json!({"source": b, "catalog": c}));
        c
    }
    /// Project with demo weather, residential load and the catalog module + inverter on a 2-MPPT stringing.
    fn project(&self, n: &str, cat: &str, mods: u32) -> String {
        let f = self.s(&format!("{n}.solarcraft"));
        let _ = std::fs::remove_file(&f);
        ok("create_project", json!({"file": f, "name": n, "latitude": 4.711, "longitude": -74.072}));
        ok("set_mode", json!({"file": f, "mode": "detailed"}));
        ok("use_demo_weather", json!({"file": f}));
        ok("catalog_pin", json!({"catalog": cat, "file": f, "family": "module", "id": "m1"}));
        ok("catalog_pin", json!({"catalog": cat, "file": f, "family": "pvinverter", "id": "i1"}));
        let lib = ok("get_project", json!({"file": f}))["library"].clone();
        let mid = lib["modules"].as_array().unwrap().iter().find(|m| m["id"].as_str().unwrap().starts_with("cat-m1")).unwrap()["id"].clone();
        let iid = lib["inverters"].as_array().unwrap().iter().find(|m| m["id"].as_str().unwrap().starts_with("cat-i1")).unwrap()["id"].clone();
        ok("set_fields", json!({"file": f, "edits": [{"pointer": "/arrays/0/module_id", "value": mid}, {"pointer": "/arrays/0/inverter_id", "value": iid}]}));
        ok("set_section", json!({"file": f, "section": "loads", "value": {"profile": {"kind": "synthetic", "profile": "residential"}, "annual_kwh": 4000, "critical_fraction": 0.3}}));
        ok("set_stringing", json!({"file": f, "inverters": [{"id": "inv-1", "name": "FS", "inverter_id": iid, "mppts": [[{"array_id": "array-1", "modules_per_string": mods, "strings": 1}], [{"array_id": "array-1", "modules_per_string": mods, "strings": 1}]]}]}));
        f
    }
}
fn base_recs() -> Vec<(&'static str, Value)> {
    vec![("Online companies", company()), ("Online countries", country()), ("Online pvmodules", module(1, 41.4, 49.6, 1760.0, 0.0)), ("Online pvinverters", inverter(5000.0, 0.0, 1)),
         ("Online batteries", battery(1, 100.0)), ("Online batterysystems", system("s1", 1, battery(1, 100.0), 1, 1, 1)), ("Online generators", generator(1.3)),
         ("Online demandtariffs", tariff("Online demandtariffs", "t1", 800.0, 1100.0, 12000.0)), ("Online feedintariffs", tariff("Online feedintariffs", "f1", 300.0, 0.0, 0.0))]
}

#[test]
fn identities_revisions_and_joins() {
    let fx = Fixture::new("id");
    let mut recs = base_recs();
    recs.push(("Online pvmodules", module(2, 42.0, 50.0, 1760.0, 0.0)));
    recs.push(("Online batteries", battery(2, 120.0)));
    // s2 links battery v2 but embeds a v1 snapshot; s3 links a missing revision with an embedded snapshot.
    recs.push(("Online batterysystems", system("s2", 2, battery(1, 100.0), 1, 1, 1)));
    recs.push(("Online batterysystems", system("s3", 9, battery(1, 100.0), 1, 1, 1)));
    let b = backup(&fx.dir, "id.backup.sqlite", &recs);
    let c = fx.s("id.catalog.sqlite");
    let rep = ok("catalog_import", json!({"source": b, "catalog": c}));
    assert_eq!(num(&rep, "/audit/records") as usize, recs.len());
    assert_eq!(rep["entities_by_family"]["module"], json!(2));
    assert_eq!(rep["distinct_ids_by_family"]["module"], json!(1), "revisions share the id");
    assert_eq!(rep["joins"]["batterysystem.battery"], json!({"ok": 1, "mismatch": 1, "fallback_embedded": 1}));
    assert_eq!(num(&rep, "/api_sha_mismatches"), 0.0);
    // Linked revision is authoritative: s2 uses battery v2 (120 Ah → 6.144 kWh), not the embedded v1.
    let d2 = ok("catalog_details", json!({"catalog": c, "family": "batterysystem", "id": "s2"}));
    assert!((num(&d2, "/normalized/record/capacity_kwh") - 3.2 * 16.0 * 120.0 / 1000.0).abs() < 1e-9);
    assert_eq!(d2["joins"][1]["status"], json!("mismatch"));
    let d3 = ok("catalog_details", json!({"catalog": c, "family": "batterysystem", "id": "s3"}));
    assert!(d3["joins"].as_array().unwrap().iter().any(|j| j["status"] == json!("fallback_embedded")));
    // Revision pinning: latest by default, explicit revision on request, kept after reopening.
    let f = fx.s("pin.solarcraft");
    ok("create_project", json!({"file": f, "name": "pin", "latitude": 4.7, "longitude": -74.0}));
    let latest = ok("catalog_pin", json!({"catalog": c, "file": f, "family": "module", "id": "m1"}));
    assert_eq!(latest["catalog_ref"]["version"], json!(2));
    let v1 = ok("catalog_pin", json!({"catalog": c, "file": f, "family": "module", "id": "m1", "version": 1}));
    assert_eq!(v1["catalog_ref"]["version"], json!(1));
    assert_eq!(v1["record"]["v_mp"], json!(41.4));
    let p = ok("get_project", json!({"file": f}));
    let refs: Vec<i64> = p["library"]["modules"].as_array().unwrap().iter().filter_map(|m| m["catalog_ref"]["version"].as_i64()).collect();
    assert_eq!(refs, vec![2, 1], "both revisions pinned under scoped ids");
    // Unit conversions: mV/K → V/K, mA/K → A/K, fraction/K → %/K, mm → m; the fit used them.
    let r = &v1["record"];
    assert!((num(r, "/beta_voc") + 0.134).abs() < 1e-12 && (num(r, "/alpha_sc") - 0.0058).abs() < 1e-12 && (num(r, "/gamma_pmp") + 0.35).abs() < 1e-12);
    assert!((num(r, "/length_m") - 1.76).abs() < 1e-12 && (num(r, "/width_m") - 1.13).abs() < 1e-12);
    assert!(r["model_params"]["fit_acceptable"].as_bool().unwrap());
    // Search uses names, companies and countries (localised); history and details carry provenance.
    assert_eq!(ok("catalog_search", json!({"catalog": c, "family": "demandtariff", "query": "kolumbien"}))["hits"].as_array().unwrap().len(), 1);
    assert_eq!(ok("catalog_search", json!({"catalog": c, "query": "fictional fs-450"}))["hits"][0]["versions"], json!(2));
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[test]
fn imported_fields_drive_results() {
    let fx = Fixture::new("drive");
    // Module height (layout): taller modules → fewer fit on the same roof.
    let roof = json!({"outline": [[0,0],[10,0],[10,5],[0,5]], "obstacles": [], "params": {"setback": 0.3}});
    let fit = |h: f64| { let mut r = base_recs(); r[2] = ("Online pvmodules", module(1, 41.4, 49.6, h, 0.0)); let c = fx.catalog(&format!("h{h}"), &r);
        let f = fx.project(&format!("h{h}"), &c, 7); let mut q = roof.clone(); q["file"] = json!(f); ok("set_roof", q)["modules_fit"].as_u64().unwrap() };
    let (short, tall) = (fit(1700.0), fit(2300.0));
    assert!(tall < short, "module height drives layout: {short} → {tall}");

    // Module Voc and increaseU0 (string sizing): 10 modules pass at 49.6 V, fail at 58 V or with +10 % initial Voc.
    let check = |voc: f64, inc: f64| { let mut r = base_recs(); r[2] = ("Online pvmodules", module(1, 41.4, voc, 1760.0, inc)); let c = fx.catalog(&format!("v{voc}{inc}"), &r);
        let f = fx.project(&format!("v{voc}{inc}"), &c, 10); ok("stringing", json!({"file": f}))["issues"].as_array().unwrap().iter().any(|i| i["code"] == json!("STRING_VOC_MAX")) };
    assert!(!check(49.6, 0.0));
    assert!(check(58.0, 0.0), "higher catalog Voc must break the 600 V limit");
    assert!(check(49.6, 0.1), "increaseU0 must raise the checked cold Voc");

    // Inverter maxAC_Power (clipping) and feedInFrom (start threshold) change the AC result.
    let ac = |pac: f64, start: f64| { let mut r = base_recs(); r[3] = ("Online pvinverters", inverter(pac, start, 1)); let c = fx.catalog(&format!("i{pac}-{start}"), &r);
        let f = fx.project(&format!("i{pac}-{start}"), &c, 7); let s = ok("simulate", json!({"file": f})); (num(&s, "/annual/ac_kwh"), s) };
    let (a5, s5) = ac(5000.0, 0.0);
    let (a3, s3) = ac(3000.0, 0.0);
    let (a_start, _) = ac(5000.0, 800.0);
    assert!(a3 < a5 - 100.0 && num(&s3, "/balance/clipped_kwh") > num(&s5, "/balance/clipped_kwh"), "maxAC_Power clips: {a5} → {a3}");
    assert!(a_start < a5, "feedInFrom start threshold removes low-power hours: {a5} → {a_start}");

    // MPPT interconnection: parallelOperating 0 forbids it, 1 allows it and the trackers combine.
    let ic = |parallel: i64| { let mut r = base_recs(); r[3] = ("Online pvinverters", inverter(5000.0, 0.0, parallel)); let c = fx.catalog(&format!("ic{parallel}"), &r);
        let f = fx.project(&format!("ic{parallel}"), &c, 7); let mut p = ok("stringing", json!({"file": f})); let mut u = p["inverters"][0]["unit"].take();
        u["interconnected"] = json!([[0, 1]]); let st = ok("set_stringing", json!({"file": f, "inverters": [u]})); (f, st) };
    let (_, st0) = ic(0);
    assert!(st0["issues"].as_array().unwrap().iter().any(|i| i["code"] == json!("MPPT_INTERCONNECT")));
    let (f1, st1) = ic(1);
    assert!(st1["issues"].as_array().unwrap().iter().all(|i| i["code"] != json!("MPPT_INTERCONNECT")));
    let s1 = ok("simulate", json!({"file": f1}));
    assert!(s1["electrical"][0]["mppts"][0]["strings"].as_str().unwrap().contains("interconnected"));
    assert_eq!(s1["electrical"][0]["mppts"].as_array().unwrap().len(), 1);

    // Battery series/parallel scaling and the integrated converter in dispatch.
    let store = |per: i64, strings: i64| { let mut r = base_recs(); r[5] = ("Online batterysystems", system("s1", 1, battery(1, 100.0), per, strings, 1)); let c = fx.catalog(&format!("b{per}{strings}"), &r);
        let f = fx.project(&format!("b{per}{strings}"), &c, 7); let pin = ok("catalog_pin", json!({"catalog": c, "file": f, "family": "batterysystem", "id": "s1"}));
        ok("set_section", json!({"file": f, "section": "storage", "value": {"battery_id": pin["record"]["id"], "units": 1, "coupling": {"kind": "integrated"}, "model": "detailed", "soc_min_pct": 5, "soc_max_pct": 100, "initial_soc_pct": 50,
            "backup_reserve_pct": 0, "grid_services_pct": 0, "charge_priority": ["backup", "self_consumption", "grid_services"], "grid_charge_backup": false, "grid_service": {"kind": "none"}, "outages": [{"start_hour": 171 * 24 + 18, "hours": 10}]}}));
        (pin["record"].clone(), ok("simulate", json!({"file": f})), c, f) };
    let (r11, s11, _, _) = store(1, 1);
    let (r21, _, _, _) = store(2, 1);
    let (r12, s12, _, _) = store(1, 2);
    assert!((num(&r21, "/capacity_kwh") - 2.0 * num(&r11, "/capacity_kwh")).abs() < 1e-9 && (num(&r21, "/nominal_voltage_v") - 2.0 * num(&r11, "/nominal_voltage_v")).abs() < 1e-9, "series per string doubles energy and voltage");
    assert!((num(&r12, "/capacity_kwh") - 2.0 * num(&r11, "/capacity_kwh")).abs() < 1e-9 && (num(&r12, "/nominal_voltage_v") - num(&r11, "/nominal_voltage_v")).abs() < 1e-9, "parallel strings double energy at equal voltage");
    assert_eq!(num(&r11, "/soc_min_pct"), 5.0, "socMin 0.04 clamped to the traced 0.05 floor");
    assert_eq!(num(&r11, "/cycle_life"), 6000.0, "cycle life at 80 % DoD; −1 sentinel skipped");
    assert!(num(&s12, "/balance/batt_discharge_kwh") > num(&s11, "/balance/batt_discharge_kwh"), "more capacity, more discharge");
    assert!(num(&s11, "/balance/max_abs_residual_wh") < 1e-3);

    // Generator fuel curve during an outage (critical load beyond the battery).
    let gen = |fuel100: f64| { let mut r = base_recs(); r[6] = ("Online generators", generator(fuel100)); let c = fx.catalog(&format!("g{fuel100}"), &r);
        let f = fx.project(&format!("g{fuel100}"), &c, 7); let g = ok("catalog_pin", json!({"catalog": c, "file": f, "family": "generator", "id": "g1"}));
        ok("set_section", json!({"file": f, "section": "loads", "value": {"profile": {"kind": "synthetic", "profile": "residential"}, "annual_kwh": 6000, "critical_fraction": 1.0}}));
        ok("add_generic_storage_equipment", json!({"file": f}));
        let mut st = json!({"battery_id": "generic-bat-lfp-10kwh", "units": 1, "coupling": {"kind": "ac_coupled", "battery_inverter_id": "generic-batinv-5kw"}, "model": "simple", "soc_min_pct": 5, "soc_max_pct": 15, "initial_soc_pct": 10,
            "backup_reserve_pct": 0, "grid_services_pct": 0, "charge_priority": ["backup", "self_consumption", "grid_services"], "grid_charge_backup": false, "grid_service": {"kind": "none"}, "outages": [{"start_hour": 100 * 24 + 17, "hours": 40}]});
        st["generator_id"] = g["record"]["id"].clone();
        ok("set_section", json!({"file": f, "section": "storage", "value": st}));
        ok("simulate", json!({"file": f})) };
    let (g13, g20) = (gen(1.3), gen(2.0));
    assert!(num(&g13, "/balance/generator_kwh") > 1.0, "generator supplies the outage");
    assert!(num(&g20, "/balance/generator_fuel") > num(&g13, "/balance/generator_fuel"), "fuel curve drives fuel use");
    assert!(num(&g13, "/balance/max_abs_residual_wh") < 1e-3 && num(&g13, "/balance/unserved_kwh") < num(&g13, "/balance/generator_kwh"));

    // Tariffs: decoded schedule bills peak hours; energy and feed-in values drive the cash flow.
    let bill = |energy: f64, peak: f64, feed: f64| { let mut r = base_recs(); r[7] = ("Online demandtariffs", tariff("Online demandtariffs", "t1", energy, peak, 12000.0)); r[8] = ("Online feedintariffs", tariff("Online feedintariffs", "f1", feed, 0.0, 0.0));
        let c = fx.catalog(&format!("t{energy}-{peak}-{feed}"), &r); let f = fx.project(&format!("t{energy}-{peak}-{feed}"), &c, 7);
        let a = ok("catalog_apply_tariff", json!({"catalog": c, "file": f, "import": {"family": "demandtariff", "id": "t1"}, "export": {"family": "feedintariff", "id": "f1"}}));
        (a, ok("simulate", json!({"file": f}))) };
    let (a, base) = bill(800.0, 1100.0, 300.0);
    let t = &a["tariff"];
    assert_eq!(t["zones"]["weekday_rows"], json!([1, 0, 0, 0, 0, 0, 1]));
    assert_eq!(t["zones"]["time_plus_minutes"], json!(30));
    assert_eq!(t["fixed_per_month"], json!(12000.0));
    assert_eq!(t["export"]["price"], json!(300.0), "feed-in tier for a 5 kWp system (key 0 < 5 < 10)");
    assert!(a["applied"][0]["not_applied"].as_array().unwrap().is_empty() || a["applied"][0]["not_applied"].to_string().contains("payout"));
    let (_, peak_up) = bill(800.0, 2000.0, 300.0);
    let (_, feed_up) = bill(800.0, 1100.0, 600.0);
    assert!(num(&peak_up, "/finance/bill_without/energy") > num(&base, "/finance/bill_without/energy"), "peak zone price drives the bill");
    assert!(num(&feed_up, "/finance/npv") > num(&base, "/finance/npv"), "feed-in rate drives the cash flow");
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[test]
fn zone_billing_boundaries() {
    use solarcraft_engine::finance::import_price;
    let fx = Fixture::new("zb");
    let c = fx.catalog("zb", &base_recs());
    let f = fx.project("zb", &c, 7);
    let t: solarcraft_model::plant::Tariff = serde_json::from_value(ok("catalog_apply_tariff", json!({"catalog": c, "file": f, "import": {"family": "demandtariff", "id": "t1"}}))["tariff"].clone()).unwrap();
    // Reference year: 1 January is a Wednesday. Hour 16 off-peak, 17–19 peak, 20 off-peak (row 'a' on weekdays).
    for (h, p) in [(16, 800.0), (17, 1100.0), (19, 1100.0), (20, 800.0)] { assert_eq!(import_price(&t, h), p, "Wednesday {h}:00"); }
    // Saturday 4 January and Sunday 5 January use row 'b' (all off-peak).
    assert_eq!(import_price(&t, 3 * 24 + 18), 800.0);
    assert_eq!(import_price(&t, 4 * 24 + 18), 800.0);
    // Monday 6 January peak again; the last slot of December (month0 11 × 24 + 23).
    assert_eq!(import_price(&t, 5 * 24 + 18), 1100.0);
    assert_eq!(import_price(&t, 8759), 800.0);
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// Real records from a backup by (dataset, predicate on the parsed record).
fn real(src: &str, dataset: &str, pick: impl Fn(&Value) -> bool) -> Value {
    let c = Connection::open_with_flags(src, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut st = c.prepare("SELECT r.record_json FROM records r JOIN datasets d ON d.id = r.dataset_id WHERE d.name = ?1").unwrap();
    let rows: Vec<String> = st.query_map([dataset], |r| r.get(0)).unwrap().filter_map(|x| x.ok()).collect();
    rows.iter().map(|j| serde_json::from_str::<Value>(j).unwrap()).find(|v| pick(v)).unwrap_or_else(|| panic!("no record in {dataset}"))
}

/// Against the owner's real backup (not in the repository):
/// `SOLARCRAFT_PVSOL_BACKUP=… [SOLARCRAFT_PVSOL_FIELDS=…tsv] cargo test --release -p solarcraft-app --test catalog_import -- --ignored --nocapture`
#[test]
#[ignore]
fn real_backup() {
    let Ok(src) = std::env::var("SOLARCRAFT_PVSOL_BACKUP") else { return };
    let a = ok("catalog_audit", json!({"source": src}));
    assert_eq!((a["records"].clone(), a["top_level_field_occurrences"].clone(), a["storage_columns"].clone(), a["api_responses"].clone(), a["local_schema_rows"].clone()), (json!(165166), json!(426), json!(28), json!(217), json!(0)));
    assert_eq!(a["datasets"].as_array().unwrap().len(), 16);
    assert_eq!((a["nonempty_htnt"].clone(), a["htnt_decoded"].clone()), (json!(60), json!(60)));
    let fx = Fixture::new("real");
    let cat = fx.s("real.catalog.sqlite");
    let rep = ok("catalog_import", json!({"source": src, "catalog": cat}));
    assert_eq!(rep["paths_observed"], json!(763), "426 top-level + 337 nested paths");
    for (k, v) in rep["joins"].as_object().unwrap() { assert!(v.as_object().unwrap().keys().all(|s| s.starts_with("ok")), "{k}: {v}"); }
    eprintln!("import: {}", serde_json::to_string(&json!({"entities": rep["entities_by_family"], "distinct": rep["distinct_ids_by_family"], "changelog": rep["changelog_events"], "releases": rep["releases"], "api": rep["api_responses"], "joins": rep["joins"]})).unwrap());
    if let Ok(tsv) = std::env::var("SOLARCRAFT_PVSOL_FIELDS") {
        let cov = ok("catalog_coverage", json!({"tsv": tsv, "catalog": cat}));
        assert_eq!(cov["entries"], json!(1181));
        eprintln!("coverage: {}", cov["by_source_and_role"]);
    }

    // Real records (with their real company and country) rebuilt into small backups with one field changed.
    let module = real(&src, "Online pvmodules", |m| m["specifiedOutputPower"].as_f64().is_some_and(|p| (440.0..460.0).contains(&p)) && m["impp2"].as_f64().unwrap_or(0.0) > 0.0 && m["cellType"] == json!(0) && m["isHalfCellModule"] == json!(false));
    let inverter = real(&src, "Online pvinverters", |i| i["mppT1_NrOf"] == json!(2) && i["mppT2_NrOf"] == json!(0) && i["maxAC_Power"].as_f64().is_some_and(|p| (4500.0..=6000.0).contains(&p)) && i["maxInputVoltageDC"].as_f64().unwrap_or(0.0) >= 600.0 && i["numberOfPhases"] == json!(1) && i["mppT1_U_MPP_Min"].as_f64().unwrap_or(999.0) <= 150.0);
    let system = real(&src, "Online batterysystems", |b| { let bat = &b["battery"]; let kwh = bat["ratedVoltage"].as_f64().unwrap_or(0.0) * bat["numberOfCellsSerie"].as_f64().unwrap_or(0.0) * bat["numberOfCellsParallel"].as_f64().unwrap_or(0.0) * bat["capacityAt10"].as_f64().unwrap_or(0.0) * b["numberOfBatteriesPerString"].as_f64().unwrap_or(0.0) / 1000.0;
        b["coupling"] == json!(1) && bat["batteryType"] == json!(201) && b["numberOfBatteryStrings"] == json!(1) && (4.0..8.0).contains(&kwh) && b["maxDischarge"].as_f64().unwrap_or(0.0) >= 2500.0 });
    let battery = real(&src, "Online batteries", |b| b["id"] == system["batteryId"] && b["version"] == system["batteryVersion"]);
    let generator = real(&src, "Online generators", |_| true);
    let tariff = real(&src, "Online netmeteringtariffs", |t| t["htNtJson"].as_str().is_some_and(|h| solarcraft_catalog::tariff::decode_htnt(h).is_ok_and(|d| d["EnergyTariffRanges"].as_array().is_some_and(|r| r.len() == 2))));
    let companies: Vec<Value> = [&module, &inverter, &system, &battery, &generator, &tariff].iter().map(|r| { let mut c = r["company"].clone(); c.as_object_mut().map(|o| o.remove("dbEntryType")); real(&src, "Online companies", |x| x["id"] == r["companyId"] && x["version"] == c["version"]) }).collect();
    let country = real(&src, "Online countries", |c| c["id"] == tariff["countryId"]);
    let build = |name: &str, edit: &dyn Fn(&mut Vec<(&'static str, Value)>)| -> String {
        let mut recs: Vec<(&'static str, Value)> = companies.iter().map(|c| ("Online companies", c.clone())).collect();
        recs.dedup_by(|a, b| a.1["id"] == b.1["id"] && a.1["version"] == b.1["version"]);
        recs.extend([("Online countries", country.clone()), ("Online pvmodules", module.clone()), ("Online pvinverters", inverter.clone()), ("Online batteries", battery.clone()), ("Online batterysystems", system.clone()), ("Online generators", generator.clone()), ("Online netmeteringtariffs", tariff.clone())]);
        edit(&mut recs);
        fx.catalog(name, &recs)
    };
    let setup = |c: &str, n: &str| -> String {
        let f = fx.s(&format!("{n}.solarcraft"));
        let _ = std::fs::remove_file(&f);
        ok("create_project", json!({"file": f, "name": n, "latitude": 4.711, "longitude": -74.072}));
        ok("set_mode", json!({"file": f, "mode": "detailed"}));
        ok("use_demo_weather", json!({"file": f}));
        let m = ok("catalog_pin", json!({"catalog": c, "file": f, "family": "module", "id": module["id"]}));
        let i = ok("catalog_pin", json!({"catalog": c, "file": f, "family": "pvinverter", "id": inverter["id"]}));
        ok("set_fields", json!({"file": f, "edits": [{"pointer": "/arrays/0/module_id", "value": m["record"]["id"]}, {"pointer": "/arrays/0/inverter_id", "value": i["record"]["id"]}]}));
        ok("set_section", json!({"file": f, "section": "loads", "value": {"profile": {"kind": "synthetic", "profile": "residential"}, "annual_kwh": 4000, "critical_fraction": 0.3}}));
        ok("set_stringing", json!({"file": f, "inverters": [{"id": "inv-1", "name": "inv", "inverter_id": i["record"]["id"], "mppts": [[{"array_id": "array-1", "modules_per_string": 7, "strings": 1}], [{"array_id": "array-1", "modules_per_string": 7, "strings": 1}]]}]}));
        let b = ok("catalog_pin", json!({"catalog": c, "file": f, "family": "batterysystem", "id": system["id"]}));
        ok("set_section", json!({"file": f, "section": "storage", "value": {"battery_id": b["record"]["id"], "units": 1, "coupling": {"kind": "integrated"}, "model": "detailed", "soc_min_pct": b["record"]["soc_min_pct"], "soc_max_pct": 100, "initial_soc_pct": 50,
            "backup_reserve_pct": 20, "grid_services_pct": 0, "charge_priority": ["backup", "self_consumption", "grid_services"], "grid_charge_backup": false, "grid_service": {"kind": "none"}, "outages": []}}));
        ok("catalog_apply_tariff", json!({"catalog": c, "file": f, "import": {"family": "netmeteringtariff", "id": tariff["id"]}}));
        f
    };
    let c0 = build("base", &|_| {});
    let f0 = setup(&c0, "base");
    let s0 = ok("simulate", json!({"file": f0}));
    eprintln!("real records: module {} / inverter {} / battery system {} / tariff {} → {:.0} kWh AC, battery out {:.0} kWh, bill with plant {:.2}",
        module["name"], inverter["name"], system["name"], tariff["name"], num(&s0, "/annual/ac_kwh"), num(&s0, "/balance/batt_discharge_kwh"), num(&s0, "/finance/bill_with/total"));
    assert!(num(&s0, "/balance/max_abs_residual_wh") < 1e-3);
    // Module maxSysVoltage lowered below the string's cold Voc → the string is rejected.
    let c1 = build("vsys", &|r| { let v = r.iter_mut().find(|x| x.0 == "Online pvmodules").unwrap(); v.1["maxSysVoltage"] = json!(250); });
    let f1 = setup(&c1, "vsys");
    assert!(ok("stringing", json!({"file": f1}))["issues"].as_array().unwrap().iter().any(|i| i["code"] == json!("MODULE_SYSTEM_VOLTAGE")), "module maxSysVoltage drives string sizing");
    assert!(ok("stringing", json!({"file": f0}))["issues"].as_array().unwrap().iter().all(|i| i["code"] != json!("MODULE_SYSTEM_VOLTAGE")));
    // Inverter maxAC_Power −40 % → more clipping, less AC.
    let c2 = build("pac", &|r| { let v = r.iter_mut().find(|x| x.0 == "Online pvinverters").unwrap(); let p = v.1["maxAC_Power"].as_f64().unwrap(); v.1["maxAC_Power"] = json!(p * 0.6); });
    let s2 = ok("simulate", json!({"file": setup(&c2, "pac")}));
    assert!(num(&s2, "/annual/ac_kwh") < num(&s0, "/annual/ac_kwh"), "maxAC_Power drives clipping");
    // Battery strings ×2 → twice the energy, more discharge.
    let c3 = build("strings", &|r| { let v = r.iter_mut().find(|x| x.0 == "Online batterysystems").unwrap(); v.1["numberOfBatteryStrings"] = json!(2); });
    let s3 = ok("simulate", json!({"file": setup(&c3, "strings")}));
    assert!(num(&s3, "/balance/batt_discharge_kwh") > num(&s0, "/balance/batt_discharge_kwh") + 50.0, "battery strings drive dispatch");
    // Zone 2 price ×3 in the encoded schedule → higher bill without the plant.
    let c4 = build("zone", &|r| { let v = r.iter_mut().find(|x| x.0 == "Online netmeteringtariffs").unwrap();
        let mut d = solarcraft_catalog::tariff::decode_htnt(v.1["htNtJson"].as_str().unwrap()).unwrap();
        let p = d["EnergyTariffRanges"][1]["RangeValues"][0]["Value"].as_f64().unwrap(); d["EnergyTariffRanges"][1]["RangeValues"][0]["Value"] = json!(p * 3.0);
        v.1["htNtJson"] = json!(solarcraft_catalog::tariff::encode_htnt(&d)); });
    let s4 = ok("simulate", json!({"file": setup(&c4, "zone")}));
    assert!(num(&s4, "/finance/bill_without/energy") > num(&s0, "/finance/bill_without/energy"), "decoded zone price drives billing");
    eprintln!("downstream: maxSysVoltage→MODULE_SYSTEM_VOLTAGE; maxAC {:.0}→{:.0} kWh; strings discharge {:.0}→{:.0} kWh; zone bill {:.2}→{:.2}",
        num(&s0, "/annual/ac_kwh"), num(&s2, "/annual/ac_kwh"), num(&s0, "/balance/batt_discharge_kwh"), num(&s3, "/balance/batt_discharge_kwh"), num(&s0, "/finance/bill_without/energy"), num(&s4, "/finance/bill_without/energy"));
    let _ = err;
    std::fs::remove_dir_all(&fx.dir).ok();
}


#[test]
fn imports_accumulate_and_reimport_is_idempotent() {
    let fixture = Fixture::new("accumulate");
    let mut second_module = module(1, 41.4, 49.6, 1760.0, 0.0);
    second_module["id"] = json!("m2");
    let mut conflicting_module = module(1, 41.4, 49.6, 1760.0, 0.0);
    conflicting_module["name"] = json!("Alternative description");
    let a = backup(&fixture.dir, "a.sqlite", &[("Online pvmodules", module(1, 41.4, 49.6, 1760.0, 0.0))]);
    let b = backup(&fixture.dir, "b.sqlite", &[("Online pvmodules", second_module), ("Online pvmodules", conflicting_module)]);
    let catalog = fixture.s("merged.sqlite");
    for source in [&a, &b, &a] {
        ok("catalog_import", json!({"source": source, "catalog": catalog}));
    }
    let hits = ok("catalog_search", json!({"catalog": catalog, "family": "module"}));
    assert_eq!(hits["hits"].as_array().unwrap().len(), 2);
    let first = ok("catalog_details", json!({"catalog": catalog, "family": "module", "id": "m1"}));
    assert_eq!(first["raw"]["name"], "FS-450 rev1");
    let db = Connection::open(catalog).unwrap();
    let count = |table: &str| db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("source"), 2);
    assert_eq!(count("api_response"), 2);
    assert_eq!(count("backup_record"), 3);
}

#[test]
fn switching_to_flat_tariff_clears_schedule_and_charges() {
    let fixture = Fixture::new("flat-rate");
    let mut records = base_recs();
    let mut flat = tariff("Online demandtariffs", "flat", 500.0, 0.0, 0.0);
    flat["htNtJson"] = Value::Null;
    flat["tariffConcepts"].as_array_mut().unwrap().remove(0);
    records.push(("Online demandtariffs", flat));
    let catalog = fixture.catalog("rates", &records);
    let project = fixture.project("rates", &catalog, 7);
    ok("catalog_apply_tariff", json!({"catalog": catalog, "file": project, "import": {"family": "demandtariff", "id": "t1"}}));
    let result = ok("catalog_apply_tariff", json!({"catalog": catalog, "file": project, "import": {"family": "demandtariff", "id": "flat"}}));
    assert!(result["tariff"]["zones"].is_null());
    assert_eq!(num(&result, "/tariff/fixed_per_month"), 0.0);
    let tariff = serde_json::from_value(result["tariff"].clone()).unwrap();
    assert_eq!(solarcraft_engine::finance::import_price(&tariff, 18), 500.0);
}

#[test]
fn pinned_tariff_tiers_follow_load_and_installed_stringing() {
    let fixture = Fixture::new("tariff-run-tier");
    let mut records = base_recs();
    records[7].1["htNtJson"] = Value::Null;
    records[7].1["tariffConcepts"][1]["tariffRangeValues"] = json!([
        {"key": 0, "value": 800}, {"key": 4500, "value": 600}
    ]);
    records[8].1["tariffConcepts"][0]["tariffRangeValues"][1]["key"] = json!(7);
    let catalog = fixture.catalog("tiers", &records);
    let file = fixture.project("tiers", &catalog, 7);
    ok("catalog_apply_tariff", json!({"catalog": catalog, "file": file,
        "import": {"family": "demandtariff", "id": "t1"}, "export": {"family": "feedintariff", "id": "f1"}}));
    let rate = |result: &Value, bill: &str, money: &str, volume: &str| {
        num(result, &format!("/finance/{bill}/{money}")) / num(result, &format!("/finance/{bill}/{volume}"))
    };
    let before = ok("simulate", json!({"file": file}));
    assert!((rate(&before, "bill_without", "energy", "import_kwh") - 800.0).abs() < 1e-6);
    assert!((rate(&before, "bill_with", "export_credit", "export_kwh") - 300.0).abs() < 1e-6);

    ok("set_fields", json!({"file": file, "edits": [{"pointer": "/loads/annual_kwh", "value": 5000}]}));
    let mut assignments = ok("get_project", json!({"file": file}))["electrical"]["inverters"].clone();
    assignments[0]["mppts"][0][0]["modules_per_string"] = json!(9);
    assignments[0]["mppts"][1][0]["modules_per_string"] = json!(9);
    ok("set_stringing", json!({"file": file, "inverters": assignments}));
    let after = ok("simulate", json!({"file": file}));
    assert!((rate(&after, "bill_without", "energy", "import_kwh") - 600.0).abs() < 1e-6);
    assert!((rate(&after, "bill_with", "export_credit", "export_kwh") - 150.0).abs() < 1e-6);
    let _ = std::fs::remove_dir_all(&fixture.dir);
}
