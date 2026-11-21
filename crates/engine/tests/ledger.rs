// SPDX-License-Identifier: GPL-3.0-or-later
//! Offline engine tests on synthetic clear-sky weather (no network).
use solarcraft_engine::simulate;
use solarcraft_model::Project;
use solarcraft_pv::{irradiance as irr, spa};
use solarcraft_weather::{Hour, Provenance, Weather};

fn synthetic(lat: f64, lon: f64) -> Weather {
    let obs = spa::Observer::new(lat, lon, 0.0);
    let start = solarcraft_pv::days_from_civil(2025, 1, 1) * 86400;
    let hours = (0..8760).map(|h| {
        let t = start + h * 3600 + 1800;
        let sp = spa::solar_position(t as f64, &obs, 69.0);
        let ghi = irr::clearsky_haurwitz(sp.apparent_zenith) * if h % 7 == 0 { 0.3 } else { 1.0 };
        let (dni, dhi) = irr::erbs(ghi, sp.apparent_zenith, irr::dni_extra(solarcraft_pv::day_of_year_utc(t as f64) as f64, 1366.1));
        Hour { unix_mid: t, unix_start: None, ghi, dni, dhi, temp_air: 15.0 + 10.0 * ((h as f64 / 24.0 - 15.0) * std::f64::consts::TAU / 24.0).sin(), wind: 2.0, pressure: 101325.0 }
    }).collect();
    Weather { latitude: lat, longitude: lon, elevation: 0.0, horizon_applied: false, provenance: Provenance { provider: "synthetic".into(), dataset: "Haurwitz".into(), url: String::new(), retrieved_unix: 0, licence: "test".into(), notes: vec![] }, hours }
}

fn project(lat: f64, lon: f64, kwp: f64) -> Project {
    let mut p = Project::new("t", "t", lat, lon);
    p.set_by_pointer("/arrays/0/target_kwp", serde_json::json!(kwp)).unwrap();
    p.weather = Some(solarcraft_model::WeatherRef { hash: "h".into(), provider: "synthetic".into(), summary: String::new() });
    p
}

#[test]
fn ledger_closes_and_is_consistent() {
    for (lat, lon) in [(52.52, 13.405), (4.711, -74.072), (-33.87, 151.2)] {
        let r = simulate(&project(lat, lon, 10.0), &synthetic(lat, lon), "h", None, |_| true).unwrap();
        assert!(r.manifest.ledger_closure_error_kwh.abs() < 1e-6, "closure {}", r.manifest.ledger_closure_error_kwh);
        let monthly: f64 = r.monthly.iter().map(|m| m.ac_kwh).sum();
        let inverter_months: f64 = r.electrical.iter().flat_map(|u| u.monthly_net_ac_kwh).sum();
        assert!((inverter_months - monthly).abs() < 1e-6);
        let poa: f64 = r.array_irradiation[0].poa_kwh_m2.iter().sum();
        assert!((poa - r.annual.poa_kwh_m2).abs() < 1e-6);
        assert!((monthly - r.annual.ac_kwh).abs() < 1e-6 * r.annual.ac_kwh.max(1.0));
        let hourly: f64 = r.hourly.ac_w.iter().map(|&v| v as f64).sum::<f64>() / 1000.0;
        assert!((hourly - r.annual.ac_kwh).abs() / r.annual.ac_kwh < 1e-4);
        assert!(r.annual.performance_ratio > 0.7 && r.annual.performance_ratio < 0.95, "PR {}", r.annual.performance_ratio);
        // Every loss stage is a non-gain except irradiance level (can be + at high irradiance).
        for s in r.energy_ledger.iter().skip(2) { assert!(s.delta <= 1e-6 || s.key == "night", "{} {}", s.key, s.delta); }
    }
}

#[test]
fn clipping_appears_when_inverter_is_undersized() {
    let mut p = project(40.4, -3.7, 10.0);
    p.set_mode(solarcraft_model::DesignMode::Detailed);
    p.library.ensure_inverter(solarcraft_model::equipment::InverterSpec::generic(6.0));
    p.set_by_pointer("/arrays/0/inverter_id", serde_json::json!("generic-inv-6kw")).unwrap();
    let r = simulate(&p, &synthetic(40.4, -3.7), "h", None, |_| true).unwrap();
    let clip = r.energy_ledger.iter().find(|s| s.key == "clipping").unwrap();
    assert!(clip.delta < -50.0, "expected clipping, got {}", clip.delta);
}

#[test]
fn cancellation_and_errors() {
    let w = synthetic(40.4, -3.7);
    assert_eq!(simulate(&project(40.4, -3.7, 5.0), &w, "h", None, |f| f < 0.5).unwrap_err().0, "cancelled");
    let mut p = project(40.4, -3.7, 5.0);
    p.weather = None;
    assert!(simulate(&p, &w, "h", None, |_| true).unwrap_err().0.contains("WEATHER_MISSING"));
}

#[test]
fn deterministic() {
    let w = synthetic(4.711, -74.072);
    let p = project(4.711, -74.072, 5.0);
    let a = simulate(&p, &w, "h", None, |_| true).unwrap();
    let b = simulate(&p, &w, "h", None, |_| true).unwrap();
    assert_eq!(serde_json::to_string(&a.annual).unwrap(), serde_json::to_string(&b.annual).unwrap());
}

fn with_roof(lat: f64, lon: f64, chimney_h: Option<f64>) -> Project {
    let mut p = project(lat, lon, 10.0);
    let obstacles = if chimney_h.is_some() { serde_json::json!([[[4.6, 1.0], [5.4, 1.0], [5.4, 1.8], [4.6, 1.8]]]) } else { serde_json::json!([]) };
    let heights = chimney_h.map_or(serde_json::json!([]), |h| serde_json::json!([h]));
    p.set_by_pointer("/arrays/0/roof", serde_json::json!({
        "face": {"outline": [[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]], "obstacles": obstacles, "obstacle_heights": heights},
        "params": {"module_length": 0.0, "module_width": 0.0, "orientation": "best", "gap_x": 0.02, "gap_y": 0.02, "setback": 0.3, "obstacle_clearance": 0.2, "search_step": 0.05},
        "modules": [], "orientation": "best", "inputs_key": ""})).unwrap();
    p
}

fn shading_loss(r: &solarcraft_engine::SimResult) -> f64 {
    -r.irradiance_ledger.iter().find(|s| s.key == "shading").unwrap().delta_pct
}

#[test]
fn chimney_shading_is_counted_and_grows_with_height() {
    let (lat, lon) = (40.4, -3.7);
    let w = synthetic(lat, lon);
    let none = simulate(&with_roof(lat, lon, None), &w, "h", None, |_| true).unwrap();
    assert!(shading_loss(&none).abs() < 1e-9);
    let low = simulate(&with_roof(lat, lon, Some(0.5)), &w, "h", None, |_| true).unwrap();
    let tall = simulate(&with_roof(lat, lon, Some(2.5)), &w, "h", None, |_| true).unwrap();
    let (l, t) = (shading_loss(&low), shading_loss(&tall));
    println!("chimney shading loss: 0.5 m -> {l:.2} %, 2.5 m -> {t:.2} %");
    assert!(l > 0.0 && t > l && t < 30.0, "low {l} % tall {t} %");
    assert!(tall.manifest.ledger_closure_error_kwh.abs() < 1e-6);
    assert!(tall.warnings.iter().any(|n| n.code == "NEAR_SHADING"));
    let a = &with_roof(lat, lon, Some(2.5)).arrays[0];
    let obs = spa::Observer::new(lat, lon, 0.0);
    let sun: Vec<_> = w.hours.iter().map(|h| spa::solar_position(h.unix_mid as f64, &obs, 69.0)).collect();
    let s = solarcraft_engine::near_shading(a, &sun).unwrap();
    assert!(s.beam_shaded.iter().all(|f| (0.0..=1.0).contains(f)) && s.diffuse_factor > 0.5 && s.diffuse_factor <= 1.0, "svf {}", s.diffuse_factor);
}


fn dispatch_fixture(coupling: solarcraft_model::plant::Coupling) -> Project {
    use solarcraft_model::equipment::{BatteryInverterSpec, BatterySpec, Converter};
    use solarcraft_model::plant::{BatteryModel, EnergyUse, GridService, Storage};

    let mut project = Project::new("dispatch", "Dispatch", 4.7, -74.0);
    let mut battery = BatterySpec::generic_lfp(20.0);
    battery.roundtrip_efficiency = 1.0;
    battery.self_discharge_pct_per_month = 0.0;
    battery.converter = Some(Converter {
        p_rated_w: 5000.0,
        p_charge_max_w: 2000.0,
        p_discharge_max_w: 5000.0,
        efficiency_curve: vec![[0.0, 1.0], [1.0, 1.0]],
        standby_w: 0.0,
    });
    let battery_id = battery.id.clone();
    project.library.batteries.push(battery);
    let mut inverter = BatteryInverterSpec::generic(5.0);
    inverter.id = "ac".into();
    inverter.standby_w = 0.0;
    inverter.efficiency_curve = vec![[0.0, 1.0], [1.0, 1.0]];
    project.library.battery_inverters.push(inverter);
    project.storage = Some(Storage {
        battery_id, units: 1, coupling, model: BatteryModel::Simple,
        soc_min_pct: 5.0, soc_max_pct: 100.0, initial_soc_pct: 100.0,
        backup_reserve_pct: 0.0, grid_services_pct: 50.0,
        charge_priority: vec![EnergyUse::Backup, EnergyUse::SelfConsumption, EnergyUse::GridServices],
        grid_charge_backup: false,
        grid_service: GridService::ExportWindow { start_hour: 0, end_hour: 24, power_kw: 4.0 },
        outages: vec![], generator_id: None, timed_charge: None, timed_discharge: None,
    });
    project
}

#[test]
fn overlapping_discharge_shares_one_converter_limit() {
    use solarcraft_engine::energy::dispatch;
    use solarcraft_model::plant::Coupling;
    let project = dispatch_fixture(Coupling::AcCoupled { battery_inverter_id: "ac".into() });
    let result = dispatch(&project, &[], &[4000.0], &[1], 1.0).unwrap();
    assert_eq!(result.flows.batt_discharge[0], 5000.0);
    assert_eq!(result.flows.export[0], 1000.0);
    assert!(result.balance.max_abs_residual_wh < 1e-6);
}

#[test]
fn integrated_converter_keeps_its_charge_rating() {
    use solarcraft_engine::energy::{dispatch, UnitInput};
    use solarcraft_model::equipment::InverterSpec;
    use solarcraft_model::plant::Coupling;
    let mut project = dispatch_fixture(Coupling::Integrated);
    project.storage.as_mut().unwrap().initial_soc_pct = 5.0;
    let mut inverter = InverterSpec::generic(5.0);
    inverter.efficiency_curve = vec![[0.0, 1.0], [1.0, 1.0]];
    let unit = UnitInput { id: "pv", spec: &inverter, dc_w: vec![5000.0], v_dc: vec![350.0] };
    let result = dispatch(&project, &[unit], &[0.0], &[1], 1.0).unwrap();
    assert_eq!(result.flows.batt_charge[0], 2000.0);
    assert_eq!(result.flows.export[0], 3000.0);
    assert!(result.balance.max_abs_residual_wh < 1e-6);
}

#[test]
fn hybrid_ac_meters_distinguish_active_battery_output_from_idle_draw() {
    use solarcraft_engine::energy::{dispatch, UnitInput};
    use solarcraft_model::equipment::InverterSpec;
    use solarcraft_model::plant::{Coupling, GridService};

    let mut project = dispatch_fixture(Coupling::DcHybrid { unit_id: "pv".into() });
    project.storage.as_mut().unwrap().grid_service = GridService::None;
    let mut inverter = InverterSpec::generic_hybrid(5.0);
    inverter.night_w = 5.0;
    inverter.standby_w = Some(3.0);
    inverter.efficiency_curve = vec![[0.0, 1.0], [1.0, 1.0]];
    inverter.battery_port.as_mut().unwrap().dc_dc_efficiency = 1.0;
    let unit = UnitInput { id: "pv", spec: &inverter, dc_w: vec![0.0, 0.0, 50.0], v_dc: vec![0.0, 0.0, 350.0] };
    let result = dispatch(&project, &[unit], &[100.0, 0.0, 0.0], &[0, 1, 12], 1.0).unwrap();
    assert_eq!(result.flows.inverter_ac_gross, [100.0, 0.0, 0.0]);
    assert_eq!(result.flows.inverter_ac_net, [100.0, -5.0, -3.0]);
    assert_eq!(result.flows.import, [0.0, 5.0, 3.0]);
    assert_eq!(result.flows.grid_to_load, [0.0; 3]);
    assert!((result.balance.standby_kwh - 0.008).abs() < 1e-12);
    assert!(result.balance.max_abs_residual_wh < 1e-6);
}

#[test]
fn customer_load_sources_and_unserved_demand_reconcile() {
    use solarcraft_engine::energy::dispatch;
    use solarcraft_model::equipment::{GeneratorSpec, Provenance};
    use solarcraft_model::plant::{Coupling, GridService, Outage};
    let mut project = dispatch_fixture(Coupling::Integrated);
    let generator = GeneratorSpec { id: "generator".into(), manufacturer: "Fixture".into(), model: "750 W".into(), provenance: Provenance::User,
        rated_va: 750.0, max_va: 750.0, phases: 1, voltage_v: 230.0, fuel_curve: vec![[0.0, 0.0], [1.0, 0.5]],
        fuel_unit: "L".into(), catalog_ref: None, datasheet_ref: String::new() };
    let storage = project.storage.as_mut().unwrap();
    storage.initial_soc_pct = storage.soc_min_pct;
    storage.grid_service = GridService::None;
    storage.generator_id = Some(generator.id.clone());
    storage.outages = vec![Outage { start_hour: 1, hours: 1 }];
    project.library.generators.push(generator);
    project.loads.critical_fraction = 0.5;
    let result = dispatch(&project, &[], &[2000.0, 800.0], &[1, 2], 1.0).unwrap();
    let b = result.balance;
    assert_eq!(result.flows.generator_to_load, [750.0, 0.0]);
    assert_eq!(result.flows.grid_to_load, [0.0, 800.0]);
    assert_eq!(result.flows.battery_to_load, [0.0, 0.0]);
    let demand = b.pv_to_load_kwh + b.battery_to_load_kwh + b.grid_to_load_kwh + b.generator_to_load_kwh
        + b.unserved_kwh + b.shed_noncritical_kwh;
    assert!((demand - b.load_kwh).abs() < 1e-12);
    assert!((b.autarky - 750.0 / 2800.0).abs() < 1e-12);
    assert!(b.max_abs_residual_wh < 1e-6);
}

#[test]
fn timed_battery_operations_respect_windows_targets_and_converter_limits() {
    use solarcraft_engine::energy::dispatch;
    use solarcraft_model::plant::{Coupling, GridService, TimedBatteryOperation};
    let mut p = dispatch_fixture(Coupling::Integrated);
    let st = p.storage.as_mut().unwrap();
    st.initial_soc_pct = 20.0;
    st.grid_services_pct = 0.0;
    st.grid_service = GridService::None;
    st.timed_charge = Some(TimedBatteryOperation { target_soc_pct: 30.0, power_kw: 4.0, periods: vec![[22*3600, 2*3600]] });
    st.timed_discharge = Some(TimedBatteryOperation { target_soc_pct: 20.0, power_kw: 3.0, periods: vec![[12*3600, 13*3600]] });
    let r = dispatch(&p, &[], &[1000.0; 4], &[22, 23, 3, 12], 1.0).unwrap();
    assert_eq!(r.flows.batt_charge_grid[0], 2000.0);
    assert_eq!(r.flows.batt_discharge[0], 0.0);
    assert_eq!(r.flows.batt_charge_grid[1], 0.0);
    assert_eq!(r.flows.batt_discharge[3], 1000.0); // at the target, normal load dispatch resumes
    assert!(r.balance.max_abs_residual_wh < 1e-6);
}

#[test]
fn timed_discharge_exports_only_to_the_target_and_grid_limit() {
    use solarcraft_engine::energy::dispatch;
    use solarcraft_model::plant::{Coupling, GridService, TimedBatteryOperation};
    let mut p = dispatch_fixture(Coupling::Integrated);
    let st = p.storage.as_mut().unwrap();
    st.initial_soc_pct = 50.0;
    st.grid_services_pct = 0.0;
    st.grid_service = GridService::None;
    st.timed_discharge = Some(TimedBatteryOperation { target_soc_pct: 40.0, power_kw: 4.0, periods: vec![[0, 3600]] });
    p.grid.export_limit_kw = Some(1.0);
    let r = dispatch(&p, &[], &[0.0], &[0], 1.0).unwrap();
    assert_eq!(r.flows.export[0], 1000.0);
    assert_eq!(r.flows.batt_discharge[0], 1000.0);
    assert!(r.balance.max_abs_residual_wh < 1e-6);
}


#[test]
fn hybrid_timed_charge_at_night_respects_port_and_reconciles() {
    use solarcraft_engine::energy::{dispatch, UnitInput};
    use solarcraft_model::equipment::InverterSpec;
    use solarcraft_model::plant::{Coupling, GridService, TimedBatteryOperation};
    let mut p = dispatch_fixture(Coupling::DcHybrid { unit_id: "pv".into() });
    let st = p.storage.as_mut().unwrap();
    st.initial_soc_pct = 20.0;
    st.grid_services_pct = 0.0;
    st.grid_service = GridService::None;
    st.timed_charge = Some(TimedBatteryOperation { target_soc_pct: 80.0, power_kw: 4.0, periods: vec![[0, 3600]] });
    let mut inverter = InverterSpec::generic_hybrid(5.0);
    inverter.night_w = 0.0;
    inverter.efficiency_curve = vec![[0.0, 1.0], [1.0, 1.0]];
    let port = inverter.battery_port.as_mut().unwrap();
    port.p_charge_max_w = 1000.0;
    port.dc_dc_efficiency = 1.0;
    let unit = UnitInput { id: "pv", spec: &inverter, dc_w: vec![0.0], v_dc: vec![0.0] };
    let r = dispatch(&p, &[unit], &[500.0], &[0], 1.0).unwrap();
    assert_eq!(r.flows.batt_charge_grid[0], 1000.0);
    assert_eq!(r.flows.batt_discharge[0], 0.0);
    assert_eq!(r.flows.import[0], 1500.0);
    assert!(r.balance.max_abs_residual_wh < 1e-6);
}

fn multiple_battery_fixture() -> Project {
    use solarcraft_model::plant::{Coupling, GridService};
    let mut p = dispatch_fixture(Coupling::Integrated);
    let primary = p.storage.as_mut().unwrap();
    primary.initial_soc_pct = 50.0;
    primary.grid_services_pct = 0.0;
    primary.grid_service = GridService::None;

    let mut second = primary.clone();
    let mut spec = p.library.battery(&second.battery_id).unwrap().clone();
    spec.id = "second-ac-bank".into();
    spec.capacity_kwh = 10.0;
    spec.roundtrip_efficiency = 0.81;
    let converter = spec.converter.as_mut().unwrap();
    converter.p_rated_w = 2500.0;
    converter.p_charge_max_w = 750.0;
    second.battery_id = spec.id.clone();
    p.library.batteries.push(spec);
    p.additional_storage.push(second);
    p
}

#[test]
fn battery_groups_use_nominal_shares_and_close_independent_soc_ledgers() {
    use solarcraft_engine::energy::{dispatch, UnitInput};
    use solarcraft_model::equipment::InverterSpec;
    let p = multiple_battery_fixture();
    let mut inverter = InverterSpec::generic(5.0);
    inverter.night_w = 0.0;
    inverter.efficiency_curve = vec![[0.0, 1.0], [1.0, 1.0]];
    let unit = UnitInput { id: "pv", spec: &inverter, dc_w: vec![3000.0, 0.0], v_dc: vec![0.0; 2] };
    let r = dispatch(&p, &[unit], &[0.0, 3000.0], &[12, 23], 1.0).unwrap();
    let [primary, second] = r.flows.battery_banks.as_slice() else { panic!("expected two battery meters") };
    assert_eq!(primary.charge_wh, [2000.0, 0.0]);
    assert_eq!(second.charge_wh, [750.0, 0.0]);
    assert_eq!(primary.discharge_wh, [0.0, 2000.0]);
    assert_eq!(second.discharge_wh, [0.0, 1000.0]);
    assert_eq!(primary.soc_pct, [60.0, 50.0]);
    assert!((second.soc_pct[1] - 45.63889).abs() < 1e-5);
    assert_eq!(r.flows.export, [250.0, 0.0]);
    assert_eq!(r.flows.import, [0.0, 0.0]);
    assert!(r.balance.batt_conversion_loss_kwh > 0.18);
    assert!(r.balance.max_abs_residual_wh < 1e-6);
}

#[test]
fn battery_groups_share_export_headroom_and_keep_scheduled_grid_charges_separate() {
    use solarcraft_engine::energy::dispatch;
    use solarcraft_model::plant::TimedBatteryOperation;
    let mut p = multiple_battery_fixture();
    for bank in p.storage.iter_mut().chain(&mut p.additional_storage) {
        bank.timed_discharge = Some(TimedBatteryOperation {
            target_soc_pct: 40.0, power_kw: 1.0, periods: vec![[12 * 3600, 13 * 3600]],
        });
        bank.timed_charge = Some(TimedBatteryOperation {
            target_soc_pct: 70.0, power_kw: 1.0, periods: vec![[22 * 3600, 2 * 3600]],
        });
    }
    p.grid.export_limit_kw = Some(1.2);
    let r = dispatch(&p, &[], &[0.0; 2], &[12, 23], 1.0).unwrap();
    let [primary, second] = r.flows.battery_banks.as_slice() else { panic!("expected two battery meters") };
    assert_eq!(primary.discharge_wh, [1000.0, 0.0]);
    assert_eq!(second.discharge_wh, [200.0, 0.0]);
    assert_eq!(primary.charge_grid_wh, [0.0, 1000.0]);
    assert_eq!(second.charge_grid_wh, [0.0, 750.0]);
    assert_eq!(r.flows.export, [1200.0, 0.0]);
    assert_eq!(r.flows.import, [0.0, 1750.0]);
    assert!(r.balance.max_abs_residual_wh < 1e-6);
}

#[test]
fn parallel_integrated_units_scale_converter_power() {
    use solarcraft_engine::energy::dispatch;
    use solarcraft_model::plant::{Coupling, TimedBatteryOperation};
    let mut p = dispatch_fixture(Coupling::Integrated);
    let bank = p.storage.as_mut().unwrap();
    bank.units = 2;
    bank.initial_soc_pct = 30.0;
    bank.timed_charge = Some(TimedBatteryOperation {
        target_soc_pct: 70.0, power_kw: 5.0, periods: vec![[0, 3600]],
    });
    let r = dispatch(&p, &[], &[0.0], &[0], 1.0).unwrap();
    assert_eq!(r.flows.batt_charge_grid, [4000.0]);
    assert_eq!(r.flows.soc_pct, [40.0]);
    assert!(r.balance.max_abs_residual_wh < 1e-6);
}
