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
    call("add_equipment", json!({"file":file, "kind":"generator", "record":{
        "id":"gen-1", "manufacturer":"Fixture", "model":"7 kVA", "rated_va":7000,"max_va":7000,"phases":1,"voltage_v":230,
        "fuel_unit":"L","fuel_curve":[[0.0,0.0],[1.0,2.0]]
    }}));
    workflow(&file, "off_grid_generator", "both");
    call("set_fields", json!({"file":file,"edits":[
        {"pointer":"/economics/horizon_years","value":4}, {"pointer":"/storage/initial_soc_pct","value":15}
    ]}));
    call("set_section", json!({"file":file, "section":"island_plan", "value":{
        "minimum_generator_pct":30, "minimum_run_hours":2, "charge_battery":true, "start_soc_pct":25,
        "stop_soc_pct":70, "fuel_price":1.2, "fuel_escalation_pct":2, "generator_investment":2000
    }}));
    let controlled = app::simulate(&file, |_| true).unwrap();
    assert_eq!(controlled.balance.import_kwh + controlled.balance.export_kwh, 0.0);
    assert!(controlled.balance.generator_starts > 0 && controlled.balance.generator_hours >= controlled.balance.generator_starts * 2 - 1);
    assert!(controlled.balance.generator_charge_kwh > 0.0);
    assert!(controlled.balance.generator_hours < 8760 && controlled.balance.batt_discharge_kwh > 0.0);
    assert!(controlled.balance.generator_to_load_kwh + controlled.balance.generator_charge_kwh <= controlled.balance.generator_kwh + 1e-6);
    assert!(controlled.balance.max_abs_residual_wh < 1e-3);
    let cost = controlled.island_finance.unwrap();
    assert!(cost.cost_per_served_kwh.unwrap() > 0.0);
    assert!((cost.years[0].fuel_cost - controlled.balance.generator_fuel * 1.2).abs() < 1e-6);
    assert_eq!(cost.years.len(), 4);

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


#[test]
fn ev_heating_and_hourly_wiring_feed_the_same_project_ledger() {
    let file = fixture("flex-cables");
    call("use_demo_weather", json!({"file":file}));
    call("set_section", json!({"file":file,"section":"loads","value":{
        "profile":{"kind":"synthetic","profile":"flat"}, "annual_kwh":1752, "critical_fraction":1.0
    }}));
    call("set_section", json!({"file":file,"section":"ev","value":{
        "daily_kwh":8,"charger_kw":3,"efficiency":0.8,"arrival_hour":8,"departure_hour":16,"strategy":"solar_preferred"
    }}));
    call("set_section", json!({"file":file,"section":"heating","value":{
        "setpoint_c":26,"heat_loss_kw_per_k":0.5,"thermal_kw":2,"cutoff_c":-15,"cop_at_0c":2.5,"cop_at_10c":3.8,"resistance_backup_kw":1
    }}));
    let steps = workflow(&file,"grid_ev","roof")["steps"].as_array().unwrap().clone();
    assert!(steps.contains(&json!("ev")) && !steps.contains(&json!("heating")));
    let ev_only = app::simulate(&file, |_| true).unwrap();
    assert!(ev_only.ev.is_some() && ev_only.heating.is_none());
    assert!(app::get_project(&file).unwrap().heating.is_some());
    workflow(&file,"custom","both");

    let circuit = |kind: &str| json!({
        "label":kind, "kind":kind,"metal":"copper","length_m":90,"area_mm2":4,
        "temperature_c":50,"voltage_v":400,"current_a":10,"circuits":1,"target_drop_pct":1.5,
        "ampacity_a":12,"derating":0.7,"candidates":[{"area_mm2":4,"ampacity_a":12},{"area_mm2":10,"ampacity_a":21},{"area_mm2":16,"ampacity_a":30}]
    });
    let rows = vec![circuit("dc_string"), circuit("ac_single")];
    let preview = call("cable_plan",json!({"file":file,"circuits":rows}));
    let sized = &preview["estimates"][0];
    assert_eq!(sized["ampacity_ok"], false);
    assert_eq!(sized["suggested"]["area_mm2"].as_f64(), Some(10.0));
    assert!(app::api::dispatch("cable_plan",&json!({"file":file,"mode":"apply_hourly","circuits":rows}),&mut |_| true).is_err());
    let safe: Vec<_> = rows.into_iter().map(|mut row| {
        row["area_mm2"] = json!(10);
        row["ampacity_a"] = json!(21);
        row
    }).collect();
    call("cable_plan",json!({"file":file,"mode":"apply_hourly","circuits":safe}));
    let run = app::simulate(&file, |_| true).unwrap();
    let ev = run.ev.as_ref().unwrap();
    let heat = run.heating.as_ref().unwrap();
    assert!((ev.charged_kwh - 8.0 * 365.0).abs() < 1e-6 && ev.forecast_surplus_kwh > 1.0);
    assert!((heat.demand_kwh - 3.0 * 8760.0).abs() < 1e-6 && heat.unmet_heat_kwh.abs() < 1e-6);
    assert!((run.balance.load_kwh - (1752.0 + ev.electrical_kwh + heat.pump_electric_kwh + heat.backup_kwh)).abs() < 0.001);
    assert!(run.balance.max_abs_residual_wh < 1e-3);
    assert!(run.hourly.dc_cable_wh.iter().any(|&w| w > 0.0));
    assert!(run.flows.ac_cable_loss.iter().any(|&w| w > 0.0));
    let ratio = |min: f32, max: f32| run.hourly.dc_w.iter().zip(&run.hourly.dc_cable_wh)
        .find_map(|(&dc, &loss)| (dc > min && dc < max).then_some(loss / dc)).unwrap();
    assert!(ratio(2100.0, 3800.0) > ratio(100.0, 400.0) * 3.0);
    let hours = call("results_hours",json!({"file":file,"day_of_year":160}));
    assert!(hours["hours"].as_array().unwrap().iter().any(|h| h["ev_wh"].as_f64().unwrap() > 0.0));
    let report = call("report_preview",json!({"file":file,"report_type":"system_planner"})).to_string();
    assert!(report.contains("Electric vehicle charging") && report.contains("Space heating") && report.contains("Hourly cable losses"));
    std::fs::remove_file(file).ok();
}

#[test]
fn fleet_and_tank_dispatch_preserve_mobility_heat_and_site_limits() {
    let file = fixture("fleet-tank");
    call("use_demo_weather", json!({"file":file}));
    call("set_section", json!({"file":file,"section":"loads","value":{
        "profile":{"kind":"synthetic","profile":"flat"}, "annual_kwh":8760, "critical_fraction":1.0
    }}));
    let group = |name: &str, weekdays: Vec<u32>| json!({
        "name":name, "vehicles":1, "battery_kwh":30, "initial_soc_pct":55,
        "departure_soc_pct":45,"trip_kwh":4,"weekdays":weekdays,"arrival_hour":9,"departure_hour":23,
        "charge_kw":1.8,"charge_efficiency":0.94,"discharge_kw":1.1,"discharge_efficiency":0.9,"strategy":"solar_preferred"
    });
    call("set_section", json!({"file":file,"section":"fleet","value":{
        "groups":[group("Service",vec![0,1,2,3,4]),group("Weekend",vec![4,5,6])],
        "site_charge_kw":2.0,"site_discharge_kw":1.5
    }}));
    call("set_section", json!({"file":file,"section":"heating","value":{
        "setpoint_c":21,"heat_loss_kw_per_k":0.10,"thermal_kw":3,"cutoff_c":-15,"cop_at_0c":2.5,
        "cop_at_10c":3.8,"resistance_backup_kw":1
    }}));
    call("set_section", json!({"file":file,"section":"heat_store","value":{
        "capacity_kwh":8,"initial_kwh":2,"charge_kw":2,"discharge_kw":2,"standing_loss_pct_per_day":2,
        "daily_hot_water_kwh":2,"hot_water_hour":20
    }}));
    let r = app::simulate(&file, |_| true).unwrap();
    let fleet = r.fleet.as_ref().unwrap();
    assert!(fleet.v2h_kwh > 0.0 && r.balance.ev_to_load_kwh > 0.0);
    for group in &fleet.groups {
        assert!(group.departures > 0 && group.departures_below_target == 0);
        assert!(group.min_soc_pct >= 0.0 && group.max_soc_pct <= 100.0);
        let closure = group.initial_kwh + group.charged_kwh - group.discharged_kwh
            - group.conversion_loss_kwh - group.travelled_kwh - group.final_kwh;
        assert!(closure.abs() < 1e-7);
    }
    assert!(r.hourly.ev_wh.iter().zip(&r.hourly.ev_discharge_wh).all(|(&draw, &give)|
        draw <= 2000.0 && give <= 1500.0 && (draw < 0.01 || give < 0.01)));
    let tank = r.heat_store.as_ref().unwrap();
    assert!(tank.charge_heat_kwh > 0.0 && tank.discharge_heat_kwh > 0.0 && tank.standing_loss_kwh > 0.0);
    assert!((tank.charge_heat_kwh + 2.0 - tank.discharge_heat_kwh - tank.standing_loss_kwh - tank.end_kwh).abs() < 1e-7);
    assert_eq!(tank.hot_water_kwh, 730.0);
    assert!(r.balance.max_abs_residual_wh < 1e-5);
    let hours = call("results_hours", json!({"file":file,"day_of_year":21}));
    assert_eq!(hours["hours"][0]["fleet_soc_pct"].as_array().unwrap().len(), 2);
    let preview = call("report_preview", json!({"file":file,"report_type":"system_planner"})).to_string();
    assert!(preview.contains("EV fleet and bidirectional charging") && preview.contains("Thermal storage"));
    std::fs::remove_file(file).ok();
}

#[test]
fn isolated_dispatch_and_linked_cables_use_the_assigned_equipment() {
    let file = fixture("clusters-routed");
    call("use_demo_weather", json!({"file":file}));
    let unit = call("stringing", json!({"file":file}))["inverters"][0]["unit"]["id"].clone();
    let equipment = call("add_generic_storage_equipment", json!({"file":file}))["ids"].clone();
    let storage = json!({
        "battery_id":equipment[1],"units":1,"coupling":{"kind":"ac_coupled","battery_inverter_id":equipment[2]},
        "model":"simple","soc_min_pct":10,"soc_max_pct":95,"initial_soc_pct":50,"backup_reserve_pct":0,"grid_services_pct":0,
        "charge_priority":["backup","self_consumption","grid_services"],"grid_charge_backup":false,
        "grid_service":{"kind":"none"},"outages":[]
    });
    call("set_section", json!({"file":file,"section":"loads","value":{
        "profile":{"kind":"synthetic","profile":"flat"},"annual_kwh":4000,"critical_fraction":1
    }}));
    let circuit = |kind: &str, binding: Value, protection: f64| json!({
        "label":kind,"kind":kind,"metal":"copper","length_m":40,"area_mm2":10,"temperature_c":45,
        "voltage_v":230,"current_a":1,"circuits":1,"target_drop_pct":3,
        "ampacity_a":45,"derating":0.8,"sizing_factor":1.25,"protection_a":protection,"binding":binding,
        "candidates":[{"area_mm2":6,"ampacity_a":36},{"area_mm2":10,"ampacity_a":45}]
    });
    let dc = circuit("dc_main", json!({"kind":"mppt","unit_id":unit,"input":0}), 30.0);
    let ac = circuit("ac_single", json!({"kind":"inverter_ac","unit_id":unit}), 25.0);
    let preview = call("cable_plan", json!({"file":file,"circuits":[dc,ac]}));
    assert!(preview["circuits"][0]["current_a"].as_f64().unwrap() > 1.0);
    assert!(preview["circuits"][0]["design_current_a"].as_f64().unwrap() > preview["circuits"][0]["current_a"].as_f64().unwrap());
    assert!(preview["estimates"][1]["suggested"].is_object());
    let mut inadequate = ac.clone();
    inadequate["protection_a"] = json!(5.0);
    assert!(app::api::dispatch("cable_plan", &json!({"file":file,"mode":"apply_routed","circuits":[inadequate]}), &mut |_| true).is_err());
    call("cable_plan", json!({"file":file,"mode":"apply_routed","circuits":[dc,ac]}));
    workflow(&file,"off_grid","roof");
    call("set_section", json!({"file":file,"section":"island_clusters","value":[
        {"name":"PV island","inverter_units":[unit],"load_share_pct":25,"storage":storage},
        {"name":"Remote island","inverter_units":[],"load_share_pct":75,"storage":storage}
    ]}));
    let r = app::simulate(&file, |_| true).unwrap();
    assert_eq!(r.clusters.len(), 2);
    assert!(r.clusters[0].balance.curtailed_kwh > 0.0 && r.clusters[1].balance.unserved_kwh > 0.0);
    assert!(r.clusters[1].balance.pv_ac_kwh.abs() < 1e-10 && r.clusters[1].balance.generator_kwh == 0.0);
    assert!((r.balance.load_kwh-4000.0).abs() < 1e-6 && r.balance.import_kwh == 0.0 && r.balance.export_kwh == 0.0);
    assert!((r.balance.unserved_kwh - r.clusters.iter().map(|c| c.balance.unserved_kwh).sum::<f64>()).abs() < 1e-6);
    assert!(r.island_finance.as_ref().unwrap().investment > 10_000.0);
    assert!(r.routed_cables.iter().all(|c| c.loss_kwh > 0.0) && r.balance.max_abs_residual_wh < 1e-5);
    let hours = call("results_hours", json!({"file":file,"day_of_year":160}));
    assert!(hours["hours"].as_array().unwrap().iter().any(|h| h["routed_cable_wh"][0].as_f64().unwrap() > 0.0));
    call("add_equipment", json!({"file":file,"kind":"generator","record":{
        "id":"remote-generator","manufacturer":"Fixture","model":"Remote","rated_va":1500,"max_va":1500,
        "phases":1,"voltage_v":230,"fuel_unit":"L","fuel_curve":[[0.0,0.0],[1.0,0.75]]
    }}));
    workflow(&file,"off_grid_generator","roof");
    let mut generator = app::get_project(&file).unwrap().island_clusters;
    generator[1].storage.generator_id = Some("remote-generator".into());
    generator[1].controls = Some(serde_json::from_value(json!({"minimum_generator_pct":30,"minimum_run_hours":2,
        "charge_battery":true,"start_soc_pct":20,"stop_soc_pct":50,"fuel_price":1.6,
        "fuel_escalation_pct":4,"generator_investment":1500})).unwrap());
    call("set_section", json!({"file":file,"section":"island_clusters","value":generator}));
    let equipped = app::simulate(&file, |_| true).unwrap();
    assert_eq!(equipped.clusters[0].balance.generator_kwh, 0.0);
    assert!(equipped.clusters[1].balance.generator_charge_kwh > 0.0);
    let fuel_cost = equipped.island_finance.unwrap().years[0].fuel_cost;
    assert!((fuel_cost - equipped.clusters[1].balance.generator_fuel * 1.6).abs() < 1e-6);
    let preview = call("report_preview", json!({"file":file,"report_type":"system_planner"})).to_string();
    assert!(preview.contains("Independent islands") && preview.contains("Hourly cable losses"));
    let mut invalid = app::get_project(&file).unwrap().island_clusters;
    invalid[1].inverter_units = invalid[0].inverter_units.clone();
    call("set_section", json!({"file":file,"section":"island_clusters","value":invalid}));
    assert!(call("validate", json!({"file":file})).to_string().contains("ISLAND_CLUSTER_INVERTER"));
    std::fs::remove_file(file).ok();
}
