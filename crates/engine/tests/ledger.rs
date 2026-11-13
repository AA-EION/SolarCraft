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
        Hour { unix_mid: t, ghi, dni, dhi, temp_air: 15.0 + 10.0 * ((h as f64 / 24.0 - 15.0) * std::f64::consts::TAU / 24.0).sin(), wind: 2.0, pressure: 101325.0 }
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
