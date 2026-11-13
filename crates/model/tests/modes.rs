// SPDX-License-Identifier: GPL-3.0-or-later
use serde_json::json;
use solarcraft_model::{validate::validate, DesignMode, Origin, Project};

#[test]
fn easy_mode_derives_a_valid_layout() {
    let p = Project::new("p1", "Bogotá house", 4.711, -74.072);
    let a = &p.arrays[0];
    assert_eq!(a.tilt.value, 5.0);
    assert!(a.modules_per_string.value > 0 && a.strings.value > 0);
    assert!((a.target_kwp.value - 10.0).abs() < 0.5, "kWp {}", a.target_kwp.value);
    let issues: Vec<_> = validate(&p).into_iter().filter(|i| i.code != "WEATHER_MISSING").collect();
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn switching_modes_never_loses_data() {
    let mut p = Project::new("p1", "Berlin", 52.52, 13.405);
    p.set_mode(DesignMode::Detailed);
    p.set_by_pointer("/arrays/0/strings", json!(3)).unwrap();
    p.set_by_pointer("/arrays/0/modules_per_string", json!(12)).unwrap();
    assert_eq!(p.arrays[0].strings.origin, Origin::User);
    assert!((p.arrays[0].target_kwp.value - 36.0 * 0.39996).abs() < 0.01);
    let before = p.clone();
    p.set_mode(DesignMode::Easy);
    p.set_mode(DesignMode::Detailed);
    assert_eq!(p, before, "mode round trip must be lossless");
    // Editing the kWp target in Easy mode re-derives the layout.
    p.set_mode(DesignMode::Easy);
    p.set_by_pointer("/arrays/0/target_kwp", json!(6.0)).unwrap();
    assert_eq!(p.arrays[0].strings.origin, Origin::Derived);
    assert!((p.arrays[0].target_kwp.value - 6.0).abs() < 1e-9);
}

#[test]
fn user_values_survive_rederivation() {
    let mut p = Project::new("p1", "Madrid", 40.4168, -3.7038);
    p.set_by_pointer("/arrays/0/tilt", json!(22.5)).unwrap();
    p.set_by_pointer("/site/latitude", json!(41.0)).unwrap();
    assert_eq!(p.arrays[0].tilt.value, 22.5);
    assert!(p.set_by_pointer("/arrays/0/nope", json!(1)).is_err());
    assert!(p.set_by_pointer("/arrays/0/strings", json!("x")).is_err());
}

#[test]
fn string_too_long_is_an_error() {
    let mut p = Project::new("p1", "Cold site", 60.0, 10.0);
    p.set_mode(DesignMode::Detailed);
    p.set_by_pointer("/arrays/0/modules_per_string", json!(30)).unwrap();
    assert!(validate(&p).iter().any(|i| i.code == "STRING_VOC_MAX"));
}

#[test]
fn easy_mode_sweep_has_no_errors() {
    let mut failures = vec![];
    for lat in [-55.0, -33.9, -12.0, 0.0, 4.711, 19.4, 40.4, 52.5, 69.6] {
        for kwp in [1.0, 3.0, 5.0, 8.0, 10.0, 15.0, 25.0, 40.0, 60.0, 100.0, 200.0, 400.0] {
            let mut p = Project::new("s", "sweep", lat, 10.0);
            p.set_by_pointer("/arrays/0/target_kwp", json!(kwp)).unwrap();
            for i in validate(&p) {
                if i.severity == solarcraft_model::validate::Severity::Error && i.code != "WEATHER_MISSING" {
                    failures.push(format!("lat {lat} kWp {kwp}: {} {}", i.code, i.message));
                }
            }
            let got = p.arrays[0].target_kwp.value;
            if (got - kwp).abs() / kwp > 0.08 { failures.push(format!("lat {lat} kWp {kwp}: derived {got:.2} kWp")); }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn roof_layout_drives_easy_mode_size_and_detailed_capacity() {
    let mut p = Project::new("r", "Roof", 40.4, -3.7);
    let roof = json!({
        "face": {"outline": [[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]], "obstacles": []},
        "params": {"module_length": 0.0, "module_width": 0.0, "orientation": "best", "gap_x": 0.02, "gap_y": 0.02, "setback": 0.4, "obstacle_clearance": 0.3, "search_step": 0.05},
        "modules": [], "orientation": "best", "inputs_key": ""
    });
    p.set_by_pointer("/arrays/0/roof", roof).unwrap();
    let r = p.arrays[0].roof.as_ref().unwrap();
    // Generic module 2.0 × 1.0 m: portrait floor(9.22/1.02)=9 × floor(5.22/2.02)=2 = 18; landscape floor(9.22/2.02)=4 × floor(5.22/1.02)=5 = 20.
    assert_eq!(r.modules.len(), 20);
    let pmp = p.library.modules[0].pmp_stc();
    let n = (p.arrays[0].modules_per_string.value * p.arrays[0].strings.value) as usize;
    assert!((18..=20).contains(&n), "derived layout uses {n} modules");
    assert!((p.arrays[0].target_kwp.value - n as f64 * pmp / 1000.0).abs() < 1e-3); // rounded to watts
    // Detailed mode: more modules than fit is an error.
    p.set_mode(DesignMode::Detailed);
    p.set_by_pointer("/arrays/0/modules_per_string", json!(12)).unwrap();
    p.set_by_pointer("/arrays/0/strings", json!(2)).unwrap();
    assert!(validate(&p).iter().any(|i| i.code == "ROOF_CAPACITY"));
}

#[test]
fn user_size_larger_than_roof_is_capped_with_warning() {
    // Reproduces the shell self-test failure: user asks 4 kWp, roof fits fewer modules.
    let mut p = Project::new("c", "Cap", 6.2442, -75.5812);
    p.set_by_pointer("/arrays/0/target_kwp", json!(4.0)).unwrap();
    p.set_by_pointer("/arrays/0/roof", json!({"face": {"outline": [[0.0, 0.0], [11.0, 0.0], [8.0, 5.0], [3.0, 5.0]], "obstacles": [[[5.0, 2.0], [5.8, 2.0], [5.8, 2.8], [5.0, 2.8]]]},
        "params": {"module_length": 0.0, "module_width": 0.0, "orientation": "best", "gap_x": 0.02, "gap_y": 0.02, "setback": 0.4, "obstacle_clearance": 0.3, "search_step": 0.05}, "modules": [], "orientation": "best", "inputs_key": ""})).unwrap();
    let fit = p.arrays[0].roof.as_ref().unwrap().modules.len() as u32;
    let used = p.arrays[0].modules_per_string.value * p.arrays[0].strings.value;
    assert!(used <= fit && used > 0, "uses {used}, fits {fit}");
    let issues = validate(&p);
    assert!(!issues.iter().any(|i| i.code == "ROOF_CAPACITY"), "{issues:?}");
    assert!(issues.iter().any(|i| i.code == "ROOF_LIMITS_SIZE"));
}
