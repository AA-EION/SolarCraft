// SPDX-License-Identifier: GPL-3.0-or-later
use serde_json::{json, Value};
use solarcraft_app as app;
use std::path::Path;

fn call(method: &str, args: Value) -> Value {
    app::api::dispatch(method, &args, &mut |_| true).unwrap_or_else(|e| panic!("{method}: {}", e.message))
}

fn fixture(suffix: &str) -> std::path::PathBuf {
    let file = std::env::temp_dir().join(format!("sc-system-{}-{suffix}.solarcraft", std::process::id()));
    let _ = std::fs::remove_file(&file);
    call("create_project", json!({"file":file, "name":"Planning fixture", "latitude":4.71, "longitude":-74.07, "target_kwp":4}));
    file
}

fn workflow(file: &Path, system: &str, tools: &str) -> Value {
    call("workflow_set", json!({"file":file, "value":{"system":system,"tools":tools}}))
}

#[test]
fn workflows_preserve_inputs_and_run_without_grid_exchange_when_islanded() {
    let file = fixture("island");
    call("use_demo_weather", json!({"file":file}));
    let ids = call("add_generic_storage_equipment", json!({"file":file}))["ids"].clone();
    call("set_section", json!({"file":file,"section":"loads","value":{
        "profile":{"kind":"synthetic","profile":"flat"}, "annual_kwh":32_000, "critical_fraction":0.2
    }}));
    let steps = workflow(&file, "grid_export", "roof")["steps"].as_array().unwrap().clone();
    assert!(steps.contains(&json!("roof")) && !steps.contains(&json!("scene")));
    assert!(!steps.contains(&json!("storage")) && !steps.contains(&json!("loads")));
    assert_eq!(app::get_project(&file).unwrap().loads.annual_kwh, 32_000.0);
    workflow(&file, "grid_consumption", "both");
    let grid = app::simulate(&file, |_| true).unwrap();
    assert!(grid.balance.import_kwh > 0.0 && grid.finance.is_some());

    let steps = workflow(&file, "off_grid_generator", "scene")["steps"].as_array().unwrap().clone();
    assert!(!steps.contains(&json!("roof")) && !steps.contains(&json!("economics")));
    assert!(steps.contains(&json!("scene")) && steps.contains(&json!("storage")));
    let issues = call("validate", json!({"file":file}));
    assert!(issues.to_string().contains("WORKFLOW_STORAGE_REQUIRED"));
    call("set_section", json!({"file":file,"section":"storage","value":{
        "battery_id":ids[1], "units":1, "coupling":{"kind":"ac_coupled", "battery_inverter_id":ids[2]},
        "model":"simple", "soc_min_pct":10, "soc_max_pct":95, "initial_soc_pct":50,
        "backup_reserve_pct":20, "grid_services_pct":10, "charge_priority":["backup","self_consumption","grid_services"],
        "grid_charge_backup":true, "grid_service":{"kind":"peak_shaving","import_limit_kw":3},
        "outages":[{"start_hour":0,"hours":3}]
    }}));
    assert!(call("validate", json!({"file":file})).to_string().contains("ISLAND_GENERATOR_REQUIRED"));
    call("set_fields", json!({"file":file,"edits":[{"pointer":"/economics/horizon_years","value":0}]}));
    call("add_equipment", json!({"file":file, "kind":"generator", "record":{
        "id":"gen-1", "manufacturer":"Fixture", "model":"1 kVA", "rated_va":1000,"max_va":1000,"phases":1,"voltage_v":230,
        "fuel_unit":"L","fuel_curve":[[0.0,0.0],[1.0,0.4]]
    }}));
    call("set_fields", json!({"file":file,"edits":[{"pointer":"/storage/generator_id","value":"gen-1"}]}));
    let island = app::simulate(&file, |_| true).unwrap();
    assert_eq!(island.balance.import_kwh, 0.0);
    assert_eq!(island.balance.export_kwh, 0.0);
    assert_eq!(island.balance.shed_noncritical_kwh, 0.0);
    assert!(island.balance.unserved_kwh > 0.0 && island.balance.generator_kwh > 0.0 && island.balance.generator_fuel > 0.0);
    assert!((island.balance.load_served_kwh + island.balance.unserved_kwh - island.balance.load_kwh).abs() < 1e-6);
    assert!(island.finance.is_none() && island.manifest.ledger_closure_error_kwh.abs() < 1e-6);
    assert!(app::latest_results(&file).unwrap().is_some());
    assert!(island.balance.max_abs_residual_wh < 1e-3);
    let report = call("report_preview", json!({"file":file, "report_type":"system_planner"})).to_string();
    assert!(report.contains("Monthly off-grid supply") && report.contains("Generator fuel"));
    assert!(!report.contains("Outage hours"));

    workflow(&file, "off_grid", "both");
    let no_generator = app::simulate(&file, |_| true).unwrap();
    assert_eq!(no_generator.balance.generator_kwh, 0.0);
    assert!(no_generator.balance.unserved_kwh > island.balance.unserved_kwh);
    workflow(&file, "custom", "both");
    let stored = app::get_project(&file).unwrap().storage.unwrap();
    assert_eq!(stored.generator_id.as_deref(), Some("gen-1"));
    assert!(stored.grid_charge_backup && stored.outages.len() == 1);
    std::fs::remove_file(file).ok();
}

#[test]
fn separate_roofs_and_cable_estimates_round_trip_and_apply_atomically() {
    let file = fixture("design");
    let second = call("add_array", json!({"file":file,"name":"West roof"}));
    assert_eq!(second["array"], 1);
    let rectangle = json!([[0,0],[10,0],[10,6],[0,6]]);
    let west = json!([[0,0],[9,0],[9,6],[4,6],[4,8],[0,8]]);
    let obstacle = json!([[[1.1,2.1],[2.0,2.1],[1.55,2.9]]]);
    call("set_roof", json!({"file":file,"array":0,"outline":rectangle}));
    call("set_roof", json!({"file":file,"array":1,"outline":west,"obstacles":obstacle,"obstacle_heights":[1.6]}));
    let p = call("get_project", json!({"file":file}));
    assert_eq!(p["arrays"][0]["roof"]["face"]["outline"], json!([[0.0,0.0],[10.0,0.0],[10.0,6.0],[0.0,6.0]]));
    assert_eq!(p["arrays"][1]["roof"]["face"]["obstacles"], obstacle);
    call("set_roof", json!({"file":file,"array":1,"outline":null}));
    assert!(app::get_project(&file).unwrap().arrays[0].roof.is_some());
    call("undo", json!({"file":file}));
    assert_eq!(app::get_project(&file).unwrap().arrays[1].roof.as_ref().unwrap().face.obstacle_heights, [1.6]);

    let dc = json!({"label":"DC string A", "kind":"dc_string", "metal":"copper","length_m":25.0,
        "extra_m":10.0, "area_mm2":4.0,"temperature_c":20.0,"voltage_v":400.0,"current_a":10.0,"circuits":1,"target_drop_pct":1.5});
    let ac = json!({"label":"AC feeder", "kind":"ac_three", "metal":"aluminium","length_m":25.0,
        "area_mm2":10.0,"temperature_c":20.0,"voltage_v":400.0,"current_a":8.0,"circuits":2,"target_drop_pct":1.0});
    let plan = call("cable_plan", json!({"file":file,"mode":"preview","circuits":[dc,ac]}));
    let near = |actual: &Value, expected: f64| assert!((actual.as_f64().unwrap() - expected).abs() < 1e-8);
    near(&plan["estimates"][0]["drop_v"], 2.58615);
    near(&plan["estimates"][0]["loss_w"], 25.8615);
    near(&plan["estimates"][1]["conductor_m"], 150.0);
    near(&plan["estimates"][1]["loss_w"], 27.13344);
    assert!(app::get_project(&file).unwrap().cable_plan.is_empty());
    let previous = app::get_project(&file).unwrap();
    let key = solarcraft_engine::inputs_hash(&previous, "fixture");
    call("cable_plan", json!({"file":file,"mode":"save","circuits":[dc]}));
    let saved = app::get_project(&file).unwrap();
    assert_eq!(saved.losses.dc_wiring_pct.value, 1.5);
    assert_eq!(solarcraft_engine::inputs_hash(&saved, "fixture"), key);
    let applied = call("cable_plan", json!({"file":file,"mode":"apply"}));
    let actual = app::get_project(&file).unwrap();
    near(&applied["dc_loss_pct"], actual.losses.dc_wiring_pct.value);
    assert_eq!(actual.losses.ac_wiring_pct.value, 0.5);
    assert_eq!(actual.cable_plan.len(), 1);
    call("undo", json!({"file":file}));
    assert_eq!(app::get_project(&file).unwrap().losses.dc_wiring_pct.value, 1.5);
    std::fs::remove_file(file).ok();
}
