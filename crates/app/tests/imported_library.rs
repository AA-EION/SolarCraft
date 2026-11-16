// SPDX-License-Identifier: GPL-3.0-or-later
//! Checks against a user library imported from the owner's own equipment workbook.
//! The data is not part of the repository, so the test is ignored by default:
//!
//! ```text
//! SOLARCRAFT_IMPORT_LIB=/path/library.sqlite cargo test --release -p solarcraft-app --test imported_library -- --ignored --nocapture
//! ```
use serde_json::{json, Value};
use solarcraft_app::api::invoke_json;

fn ok(m: &str, p: Value) -> Value {
    let r: Value = serde_json::from_str(&invoke_json(&json!({"method": m, "params": p}).to_string(), &mut |_| true)).unwrap();
    assert_eq!(r["ok"], json!(true), "{m}: {r}");
    r["result"].clone()
}

#[test]
#[ignore]
fn imported_records_fit_and_simulate() {
    let Ok(lib) = std::env::var("SOLARCRAFT_IMPORT_LIB") else { return };
    // 1. Module fits: datasheet STC values and temperature coefficients, then the second (low-light) point as an independent check.
    let mods = ok("library_list", json!({"library": lib, "kind": "module", "limit": 3000}))["items"].as_array().unwrap().clone();
    let (mut fitted, mut acceptable, mut failed) = (0, 0, 0);
    let mut ll_err = vec![];
    for it in &mods {
        match solarcraft_app::plant_ops::canonical_equipment("module", &it["record"]) {
            Ok(m) => {
                fitted += 1;
                if m["model_params"]["fit_acceptable"] == json!(true) { acceptable += 1; }
                if let Some(e) = m["model_params"]["low_light_pmp_rel_err"].as_f64() { ll_err.push(e * 100.0); }
            }
            Err(_) => failed += 1,
        }
    }
    ll_err.sort_by(|a, b| a.total_cmp(b));
    let q = |p: f64| ll_err.get(((ll_err.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(f64::NAN);
    eprintln!("modules sampled {}: fitted {fitted}, acceptable {acceptable}, failed {failed}", mods.len());
    eprintln!("low-light Pmp error (model vs datasheet second point), %: p05 {:.2} p25 {:.2} median {:.2} p75 {:.2} p95 {:.2} (n={})", q(0.05), q(0.25), q(0.5), q(0.75), q(0.95), ll_err.len());
    assert!(fitted as f64 >= 0.9 * mods.len() as f64);

    // 2. A project built only from imported records: module, string inverter with per-input limits, integrated battery system.
    let dir = std::env::temp_dir().join(format!("sc-imp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("imp.solarcraft");
    let fs = f.to_str().unwrap();
    let pick = |kind: &str, query: &str| -> String { ok("library_list", json!({"library": lib, "kind": kind, "query": query, "limit": 50}))["items"].as_array().unwrap()
        .iter().find(|i| kind != "module" || solarcraft_app::plant_ops::canonical_equipment("module", &i["record"]).is_ok()).unwrap()["id"].as_str().unwrap().to_string() };
    let module = pick("module", "half_cell\":true");
    let inv = ok("library_list", json!({"library": lib, "kind": "inverter", "limit": 2000}))["items"].as_array().unwrap().iter()
        .find(|i| { let r = &i["record"]; r["pac_max_w"].as_f64().is_some_and(|p| (4500.0..=6000.0).contains(&p)) && r["inputs"].as_array().is_some_and(|a| a.len() == 2) && r["v_dc_max"].as_f64().unwrap_or(0.0) >= 600.0 })
        .unwrap()["id"].as_str().unwrap().to_string();
    let bat = ok("library_list", json!({"library": lib, "kind": "battery", "query": "lfp", "limit": 2000}))["items"].as_array().unwrap().iter()
        .find(|i| i["record"]["capacity_kwh"].as_f64().is_some_and(|k| (5.0..15.0).contains(&k))).unwrap()["id"].as_str().unwrap().to_string();
    ok("create_project", json!({"file": fs, "name": "Imported", "latitude": 4.711, "longitude": -74.072}));
    ok("set_mode", json!({"file": fs, "mode": "detailed"}));
    ok("use_demo_weather", json!({"file": fs}));
    for (k, id) in [("module", &module), ("inverter", &inv), ("battery", &bat)] { ok("library_pin", json!({"library": lib, "file": fs, "kind": k, "id": id})); }
    ok("set_fields", json!({"file": fs, "edits": [{"pointer": "/arrays/0/module_id", "value": module}, {"pointer": "/arrays/0/inverter_id", "value": inv}, {"pointer": "/arrays/0/target_kwp", "value": 5.5}]}));
    ok("set_section", json!({"file": fs, "section": "loads", "value": {"profile": {"kind": "synthetic", "profile": "residential"}, "annual_kwh": 4000, "critical_fraction": 0.3}}));
    ok("set_section", json!({"file": fs, "section": "storage", "value": {"battery_id": bat, "units": 1, "coupling": {"kind": "integrated"}, "model": "detailed", "soc_min_pct": 10, "soc_max_pct": 100, "initial_soc_pct": 50,
        "backup_reserve_pct": 20, "grid_services_pct": 0, "charge_priority": ["backup", "self_consumption", "grid_services"], "grid_charge_backup": false, "grid_service": {"kind": "none"}, "outages": []}}));
    let st = ok("stringing", json!({"file": fs}));
    eprintln!("imported: module {module}, inverter {inv}, battery {bat}; issues {}", st["issues"]);
    let r = ok("simulate", json!({"file": fs}));
    eprintln!("annual {} kWh, battery out {} kWh, residual {} Wh", r["annual"]["ac_kwh"], r["balance"]["batt_discharge_kwh"], r["balance"]["max_abs_residual_wh"]);
    assert!(r["balance"]["batt_discharge_kwh"].as_f64().unwrap() > 100.0 && r["balance"]["max_abs_residual_wh"].as_f64().unwrap() < 1e-3);
    std::fs::remove_dir_all(&dir).ok();
}
