// SPDX-License-Identifier: GPL-3.0-or-later
//! The designer workflow from start to report, driven through the same JSON
//! API the native shells and the MCP server use. Every claim is checked
//! against the simulation output; nothing is read from sample data.
//!
//! Scenario (Bogotá, offline): user-authored module, hybrid inverter and
//! battery; a roof with a chimney; manual stringing on two MPPTs; a DC-coupled
//! battery with a backup reserve; residential consumption; a time-of-use
//! tariff; two variants; report; reopen and rerun.
use serde_json::{json, Value};
use solarcraft_app::api::invoke_json;

fn call(m: &str, p: Value) -> Value {
    let r: Value = serde_json::from_str(&invoke_json(&json!({"method": m, "params": p}).to_string(), &mut |_| true)).unwrap();
    r
}
fn ok(m: &str, p: Value) -> Value {
    let r = call(m, p);
    assert_eq!(r["ok"], json!(true), "{m}: {r}");
    r["result"].clone()
}
fn num(v: &Value, ptr: &str) -> f64 { v.pointer(ptr).and_then(Value::as_f64).unwrap_or_else(|| panic!("{ptr} missing in {v}")) }

const MODULE: &str = r#"{"id":"user-450","manufacturer":"Datasheet entry","model":"450 W mono half-cell","technology":"mono-c-Si","cells_in_series":72,
  "length_m":1.76,"width_m":1.13,"v_oc":49.6,"i_sc":11.6,"v_mp":41.4,"i_mp":10.87,"alpha_sc":0.0058,"beta_voc":-0.134,"gamma_pmp":-0.35}"#;
const HYBRID: &str = r#"{"id":"user-hybrid-5k","manufacturer":"Datasheet entry","model":"hybrid 5 kW","pac_max_w":5000,"eta_nominal":0.97,"mppt_v_min":120,"mppt_v_max":500,
  "v_dc_max":600,"mppt_count":2,"i_max_per_mppt":26,"night_w":2,"pdc_max_w":7500,
  "efficiency_curve":[[0.05,0.90],[0.1,0.94],[0.2,0.96],[0.3,0.968],[0.5,0.972],[0.75,0.97],[1.0,0.967]],
  "battery_port":{"v_min":150,"v_max":500,"p_charge_max_w":5000,"p_discharge_max_w":5000,"dc_dc_efficiency":0.98}}"#;
const BATTERY: &str = r#"{"id":"user-bat-10","manufacturer":"Datasheet entry","model":"LFP 10 kWh","chemistry":"lfp","capacity_kwh":10.0,"nominal_voltage_v":204.8,
  "soc_min_pct":5,"soc_max_pct":100,"p_charge_max_kw":5,"p_discharge_max_kw":5,"roundtrip_efficiency":0.95,"self_discharge_pct_per_month":2,"cycle_life":6000}"#;

fn storage(reserve: f64, outages: Value) -> Value {
    json!({"battery_id": "user-bat-10", "units": 1, "coupling": {"kind": "dc_hybrid", "unit_id": "inv-1"}, "model": "detailed",
           "soc_min_pct": 5, "soc_max_pct": 100, "initial_soc_pct": 50, "backup_reserve_pct": reserve, "grid_services_pct": 0,
           "charge_priority": ["backup", "self_consumption", "grid_services"], "grid_charge_backup": false, "grid_service": {"kind": "none"}, "outages": outages})
}
fn stringing(groups: Value) -> Value { json!({"manual": true, "inverters": [{"id": "inv-1", "name": "Hybrid 5 kW", "inverter_id": "user-hybrid-5k", "mppts": groups}]}) }

#[test]
fn designer_workflow() {
    let dir = std::env::temp_dir().join(format!("sc-flow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("casa.solarcraft");
    let _ = std::fs::remove_file(&f);
    let fs = f.to_str().unwrap();
    let lib = dir.join("library.sqlite");
    let libs = lib.to_str().unwrap();

    // 1. Project, location, weather, grid, consumption.
    ok("create_project", json!({"file": fs, "name": "Casa Usaquén", "latitude": 4.711, "longitude": -74.072}));
    ok("set_mode", json!({"file": fs, "mode": "detailed"}));
    ok("use_demo_weather", json!({"file": fs}));
    ok("set_section", json!({"file": fs, "section": "grid", "value": {"feed_in": "surplus", "export_limit_kw": 3.0, "phases": 1, "voltage_v": 120}}));
    ok("set_section", json!({"file": fs, "section": "loads", "value": {"profile": {"kind": "synthetic", "profile": "residential"}, "annual_kwh": 4200, "critical_fraction": 0.3}}));

    // 2. Equipment authored offline: saved to the user library, then pinned into the project.
    for (kind, rec) in [("module", MODULE), ("inverter", HYBRID), ("battery", BATTERY)] {
        ok("library_save", json!({"library": libs, "kind": kind, "record": serde_json::from_str::<Value>(rec).unwrap()}));
        ok("library_pin", json!({"library": libs, "file": fs, "kind": kind, "id": serde_json::from_str::<Value>(rec).unwrap()["id"]}));
    }
    let items = ok("library_list", json!({"library": libs, "query": "datasheet"}));
    assert_eq!(items["items"].as_array().unwrap().len(), 3);
    let fitted = ok("library_get", json!({"library": libs, "kind": "module", "id": "user-450"}));
    assert!(fitted["record"]["model_params"]["fit_acceptable"].as_bool().unwrap(), "module fit: {fitted}");

    // 3. Roof with a chimney, module and inverter selection.
    ok("set_fields", json!({"file": fs, "edits": [{"pointer": "/arrays/0/module_id", "value": "user-450"}, {"pointer": "/arrays/0/tilt", "value": 15}, {"pointer": "/arrays/0/azimuth", "value": 180},
                                                  {"pointer": "/arrays/0/inverter_id", "value": "user-hybrid-5k"}]}));
    let roof = ok("set_roof", json!({"file": fs, "outline": [[0,0],[11,0],[11,6],[0,6]], "obstacles": [[[5,4.2],[5.8,4.2],[5.8,5],[5,5]]], "obstacle_heights": [1.5], "params": {"setback": 0.4, "obstacle_clearance": 0.3}}));
    let fit = roof["modules_fit"].as_u64().unwrap();
    assert!(fit >= 14, "roof fits {fit}");

    // Invalid stringing: one 14-module string exceeds the inverter's 600 V at −10 °C.
    let bad = ok("set_stringing", json!({"file": fs, "inverters": stringing(json!([[{"array_id": "array-1", "modules_per_string": 14, "strings": 1}], []]))["inverters"]}));
    let codes: Vec<&str> = bad["issues"].as_array().unwrap().iter().map(|i| i["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"STRING_VOC_MAX"), "{codes:?}");
    let refused = call("simulate", json!({"file": fs}));
    assert_eq!(refused["error"]["code"], json!("SIMULATION"));
    assert!(refused["error"]["message"].as_str().unwrap().contains("STRING_VOC_MAX"));
    // Valid: 2 × 7 modules, one string per MPPT.
    let good = ok("set_stringing", json!({"file": fs, "inverters": stringing(json!([[{"array_id": "array-1", "modules_per_string": 7, "strings": 1}], [{"array_id": "array-1", "modules_per_string": 7, "strings": 1}]]))["inverters"]}));
    assert!(good["issues"].as_array().unwrap().iter().all(|i| i["severity"] != "error"), "{good}");

    // 4. Battery on the hybrid inverter's DC port with a 30 % backup reserve; time-of-use tariff.
    ok("set_section", json!({"file": fs, "section": "storage", "value": storage(30.0, json!([]))}));
    ok("set_section", json!({"file": fs, "section": "tariff", "value": {"name": "TOU", "currency": "COP", "import_price": 800, "tou": [{"name": "Peak", "price": 1100, "start_hour": 18, "end_hour": 22, "weekdays": true, "weekends": true, "months": []}],
        "export": {"kind": "fixed", "price": 300}, "fixed_per_month": 12000, "demand_charge_per_kw_month": 0, "import_escalation_pct": 4, "export_escalation_pct": 2}}));
    ok("set_section", json!({"file": fs, "section": "economics", "value": {"horizon_years": 20, "discount_rate_pct": 9, "cost_fixed": 3000000, "cost_per_kwp": 3500000, "cost_per_battery_kwh": 1800000,
        "om_pct_of_investment": 1, "om_escalation_pct": 4, "inverter_replacement_year": 13, "inverter_replacement_cost": 4000000, "battery_replacement_year": 12, "battery_replacement_cost": 0, "degradation_pct_per_year": 0.4, "subsidy": 0, "loan": null}}));
    assert!(ok("validate", json!({"file": fs}))["issues"].as_array().unwrap().iter().all(|i| i["severity"] != "error"));

    // 5. Simulate and inspect.
    let r = ok("simulate", json!({"file": fs}));
    let b = &r["balance"];
    eprintln!("balance: {}", serde_json::to_string_pretty(b).unwrap());
    eprintln!("finance: {}", r["finance"]);
    assert!(num(b, "/max_abs_residual_wh") < 1e-3, "energy balance must close every hour");
    for k in ["/pv_ac_kwh", "/load_kwh", "/import_kwh", "/export_kwh", "/batt_charge_kwh", "/batt_discharge_kwh", "/pv_to_load_kwh"] { assert!(num(b, k) > 1.0, "{k} = {}", num(b, k)); }
    assert!(num(&r, "/annual/dc_kwp") > 6.0 && num(&r, "/annual/dc_kwp") < 6.5);
    let mppts = &r["electrical"][0]["mppts"];
    assert_eq!(mppts.as_array().unwrap().len(), 2);
    let e = ok("explain", json!({"file": fs}));
    let shading = e["irradiance_kwh_m2"].as_array().unwrap().iter().find(|s| s["key"] == "shading").unwrap()["delta"].as_f64().unwrap();
    assert!(shading < 0.0, "chimney shading must reduce irradiation");
    // Hours behind the totals.
    let day = ok("results_hours", json!({"file": fs, "day_of_year": 172}));
    assert_eq!(day["hours"].as_array().unwrap().len(), 24);
    for h in day["hours"].as_array().unwrap() {
        let accounts = num(h, "/backup_wh") + num(h, "/self_wh") + num(h, "/grid_services_wh");
        let stored = num(h, "/soc_pct") / 100.0 * 10000.0 - 500.0;
        assert!((accounts - stored).abs() < 1.0, "allocations {accounts} ≠ stored {stored}");
        assert!(num(h, "/backup_wh") <= 0.30 * 9500.0 + 1e-3);
    }
    let top = ok("results_hours", json!({"file": fs, "top": "import", "count": 5}));
    assert!(num(&top["hours"][0], "/import_wh") >= num(&top["hours"][4], "/import_wh"));
    let base = r.clone();

    // 6. Variant A saved; raise the backup reserve to 60 % → less battery energy for self-consumption.
    ok("create_variant", json!({"file": fs, "name": "Reserve 30 %"}));
    ok("set_section", json!({"file": fs, "section": "storage", "value": storage(60.0, json!([]))}));
    let r60 = ok("simulate", json!({"file": fs}));
    eprintln!("reserve 60: discharge {} import {} (30 %: {} / {})", r60["balance"]["batt_discharge_kwh"], r60["balance"]["import_kwh"], b["batt_discharge_kwh"], b["import_kwh"]);
    assert!(num(&r60, "/balance/batt_discharge_kwh") < num(b, "/batt_discharge_kwh") - 50.0, "larger reserve must reduce normal discharge");
    assert!(num(&r60, "/balance/import_kwh") > num(b, "/import_kwh"));
    assert!(num(&r60, "/balance/max_abs_residual_wh") < 1e-3);
    let d60 = ok("results_hours", json!({"file": fs, "top": "-soc", "count": 50}));
    for h in d60["hours"].as_array().unwrap() { assert!(num(h, "/backup_wh") <= 0.60 * 9500.0 + 1e-3 && num(h, "/self_wh") <= 0.40 * 9500.0 + 1e-3); }

    // Outage on day 172, 03:00–08:00 (before sunrise, after the evening has drained the
    // self-consumption share). With a 60 % reserve the critical load is covered from the
    // backup allocation; with no reserve the battery is empty and load goes unserved.
    let outage = json!([{"start_hour": 171 * 24 + 3, "hours": 5}]);
    ok("set_section", json!({"file": fs, "section": "storage", "value": storage(60.0, outage.clone())}));
    let ro = ok("simulate", json!({"file": fs}));
    let od = ok("results_hours", json!({"file": fs, "day_of_year": 172}));
    let in_outage: Vec<&Value> = od["hours"].as_array().unwrap().iter().filter(|h| h["outage"] == json!(true)).collect();
    assert_eq!(in_outage.len(), 5);
    assert!(in_outage.iter().all(|h| num(h, "/import_wh") == 0.0 && num(h, "/export_wh") == 0.0));
    assert!(num(&ro, "/balance/unserved_kwh") < 0.01, "60 % reserve covers the critical load: unserved {}", ro["balance"]["unserved_kwh"]);
    let first = in_outage[0];
    let lowest = in_outage.iter().map(|h| num(h, "/backup_wh")).fold(f64::INFINITY, f64::min);
    assert!(num(first, "/self_wh") < 1.0 && lowest < num(first, "/backup_wh") + num(first, "/batt_discharge_wh") - 300.0, "backup reserve supplies the outage once self-consumption energy is gone: {first}, lowest backup {lowest}");
    ok("set_section", json!({"file": fs, "section": "storage", "value": storage(0.0, outage)}));
    let r0 = ok("simulate", json!({"file": fs}));
    eprintln!("outage unserved: reserve 60 % {:.3} kWh, reserve 0 % {:.3} kWh", num(&ro, "/balance/unserved_kwh"), num(&r0, "/balance/unserved_kwh"));
    assert!(num(&r0, "/balance/unserved_kwh") >= num(&ro, "/balance/unserved_kwh"));
    // Outside outages the backup allocation is never used for self-consumption.
    ok("set_section", json!({"file": fs, "section": "storage", "value": storage(60.0, json!([]))}));
    ok("simulate", json!({"file": fs}));
    let low = ok("results_hours", json!({"file": fs, "top": "-soc", "count": 200}));
    let cap = 0.60 * 9500.0;
    for h in low["hours"].as_array().unwrap().iter().filter(|h| num(h, "/index") > 48.0) {
        assert!(num(h, "/backup_wh") > cap * 0.97, "backup allocation drained outside an outage: {h}");
    }

    // 7. Moving the chimney away from the modules raises yield.
    ok("set_section", json!({"file": fs, "section": "storage", "value": storage(30.0, json!([]))}));
    ok("create_variant", json!({"file": fs, "name": "Chimney moved"}));
    let before = num(&ok("simulate", json!({"file": fs})), "/annual/ac_kwh");
    ok("set_roof", json!({"file": fs, "outline": [[0,0],[11,0],[11,6],[0,6]], "obstacles": [[[10.2,5.4],[10.8,5.4],[10.8,5.9],[10.2,5.9]]], "obstacle_heights": [1.5], "params": {"setback": 0.4, "obstacle_clearance": 0.3}}));
    let after = num(&ok("simulate", json!({"file": fs})), "/annual/ac_kwh");
    eprintln!("chimney moved: {before:.1} → {after:.1} kWh");
    assert!(after > before + 5.0, "yield {before} → {after}");

    // 8. Stringing change: both strings in parallel on MPPT 1 vs one per MPPT.
    let one_per = num(&ok("simulate", json!({"file": fs})), "/balance/pv_dc_kwh");
    ok("set_stringing", json!({"file": fs, "inverters": stringing(json!([[{"array_id": "array-1", "modules_per_string": 7, "strings": 1}, {"array_id": "array-1", "modules_per_string": 6, "strings": 1}], []]))["inverters"]}));
    let mixed = ok("simulate", json!({"file": fs}));
    eprintln!("stringing: 7|7 split {one_per:.1} kWh DC vs 7+6 parallel {:.1} (mismatch {})", num(&mixed, "/balance/pv_dc_kwh"), mixed["electrical"][0]["mppts"][0]["mismatch_kwh"]);
    assert!(num(&mixed, "/electrical/0/mppts/0/mismatch_kwh") > 20.0, "unequal parallel strings must show mismatch");
    ok("set_stringing", json!({"file": fs, "inverters": stringing(json!([[{"array_id": "array-1", "modules_per_string": 7, "strings": 1}], [{"array_id": "array-1", "modules_per_string": 7, "strings": 1}]]))["inverters"]}));

    // 9. Tariff change → cash flow changes.
    let cf1 = ok("simulate", json!({"file": fs}));
    let npv1 = num(&cf1, "/finance/npv");
    let p = ok("get_project", json!({"file": fs}));
    let mut t = p["tariff"].clone();
    t["import_price"] = json!(1000);
    ok("set_section", json!({"file": fs, "section": "tariff", "value": t}));
    let cf2 = ok("simulate", json!({"file": fs}));
    eprintln!("NPV {npv1:.0} → {:.0} with a higher import price", num(&cf2, "/finance/npv"));
    assert!(num(&cf2, "/finance/npv") > npv1 + 1e5);
    let years = ok("cash_flow", json!({"file": fs}));
    assert_eq!(years["years"].as_array().unwrap().len(), 21);

    // 10. Compare variants.
    let cmp = ok("compare_variants", json!({"file": fs}));
    let rows = cmp["variants"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r["error"].is_null()), "{cmp}");
    assert!(rows[0]["inputs_hash"] != rows[1]["inputs_hash"]);

    // 11. Report from the actual results.
    let out = dir.join("informe.pdf");
    ok("export_report", json!({"file": fs, "output": out.to_str().unwrap(), "format": "pdf", "lang": "es"}));
    assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF"));
    let docx = dir.join("informe.docx");
    ok("export_report", json!({"file": fs, "output": docx.to_str().unwrap(), "format": "docx", "lang": "en"}));
    // Trace printed values back to the run.
    let view = ok("report_preview", json!({"file": fs, "lang": "en"}));
    let latest = ok("results_summary", json!({"file": fs}));
    let fmt0 = |x: f64| { let s = format!("{:.0}", x.abs()); let mut o = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push(','); } o.push(c); } if x < 0.0 { format!("-{o}") } else { o } };
    let row = |title: &str, label: &str| -> String { view["sections"].as_array().unwrap().iter().find(|s| s["title"] == title).unwrap()["rows"].as_array().unwrap().iter().find(|r| r[0] == label).unwrap_or_else(|| panic!("{title}/{label}"))[1].as_str().unwrap().to_string() };
    assert_eq!(row("Financial analysis", "Net present value"), format!("{} COP", fmt0(num(&latest, "/finance/npv"))));
    assert_eq!(row("Energy balance", "Grid import"), format!("{} kWh", fmt0(num(&latest, "/balance/import_kwh"))));
    assert_eq!(view["kpis"][1]["value"].as_str().unwrap(), fmt0(num(&latest, "/annual/ac_kwh")));
    assert!(view["run_rows"][0][1].as_str().map(|h| latest["manifest"]["inputs_hash"].as_str().unwrap().starts_with(h)).unwrap());

    // 12. Close, reopen, rerun offline: identical inputs → identical outputs.
    let first = ok("results_summary", json!({"file": fs}));
    let again = ok("simulate", json!({"file": fs}));
    assert_eq!(first["manifest"]["inputs_hash"], again["manifest"]["inputs_hash"]);
    assert_eq!(first["annual"], again["annual"]);
    assert_eq!(first["balance"], again["balance"]);
    assert_eq!(first["finance"], again["finance"]);
    let _ = base;
    std::fs::remove_dir_all(&dir).ok();
}
