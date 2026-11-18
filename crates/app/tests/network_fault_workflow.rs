// SPDX-License-Identifier: GPL-3.0-or-later
use serde_json::{Value, json};
use solarcraft_app as app;
use std::path::Path;

fn invoke(file: &Path, method: &str, mut args: Value) -> Result<Value, app::AppError> {
    args["file"] = json!(file);
    app::api::dispatch(method, &args, &mut |_| true)
}

fn close(value: &Value, want: f64) {
    let got = value.as_f64().unwrap();
    assert!((got - want).abs() < 1e-8, "{got} != {want}");
}

fn device(model: &str, kind: &str) -> Value {
    json!({"model":model,"reference":"Synthetic device ratings","rated_voltage_v":720,"rating":"ac",
        "breaking_ka":12,"making_peak_ka":20,"documented_peak_ka":8,"peak_reference":"Synthetic terminal bound",
        "direct_curve_max_ka":8,
        "minimum_operating":{"model":model,"kind":kind,"reference":"Synthetic lower curve","points":[[50,0.5],[8000,0.4]]},
        "maximum_clearing":{"model":model,"kind":kind,"reference":"Synthetic upper curve","points":[[50,0.08],[8000,0.04]]}
    })
}

fn link(id: &str, from: &str, to: &str, device: Value) -> Value {
    json!({"id":id, "from":from, "to":to, "minimum_duty":{"r_mohm":50},
        "maximum_duty":{"r_mohm":50}, "device":device})
}

#[test]
fn radial_ac_dc_envelopes_coordination_save_and_report_use_the_same_results() {
    let file = std::env::temp_dir().join(format!("SolarCraft-NetworkFault-{}.solarcraft", std::process::id()));
    app::create_project(&file, "Protection study", 4.71, -74.07, Some(4.0)).unwrap();
    let original = invoke(&file, "network_faults", json!({})).unwrap();
    let dc = original["example"][0].clone();
    let mut ac = json!({
        "name":"AC sources","domain":"ac_three_phase","reference":"Independent synthetic sources on a radial feeder",
        "maximum_voltage_v":700, "buses":["Grid", "Tee", "Inverter", "Load"],
        "links":[link("main","Grid","Tee",device("Main fuse","fuse")),
            link("inverter","Inverter","Tee",device("Inverter breaker","breaker")),
            link("branch","Tee","Load",device("Branch breaker","breaker"))],
        "sources":[
            {"bus":"Grid","name":"Grid","reference":"Synthetic utility supply",
                "minimum":{"type":"voltage","volts":350.0*3_f64.sqrt(),"impedance":{"r_mohm":50}},
                "maximum":{"type":"voltage","volts":400.0*3_f64.sqrt(),"impedance":{"r_mohm":50}}},
            {"bus":"Inverter","name":"Inverter","reference":"Synthetic inverter fault envelope",
                "minimum":{"type":"current_limited","amps":250,"min_terminal_v":0,"max_terminal_v":700,"supported_seconds":0.35},
                "maximum":{"type":"current_limited","amps":500,"min_terminal_v":0,"max_terminal_v":700,"supported_seconds":0.2}}
        ],
        "faults":[{"bus":"Load","maximum_clear_seconds":0.2,"minimum_time_ratio":2}]
    });
    let preview = |studies: Value| invoke(&file, "network_faults", json!({"studies":studies})).unwrap();
    let output = preview(json!([dc,ac]));
    let d = &output["results"][0]["faults"][0];
    assert_eq!(d["status"], "pass");
    close(&d["minimum_ka"], 0.6);
    close(&d["maximum_ka"], 1.35);
    close(&d["devices"][0]["peak_ka"], 3.0);
    let a = &output["results"][1]["faults"][0];
    assert_eq!(a["status"], "pass");
    close(&a["minimum_ka"], 2.5);
    close(&a["maximum_ka"], 3.0);
    close(&a["minimum_support_seconds"], 0.2);
    close(&a["devices"][0]["minimum_ka"], 2.25);
    close(&a["devices"][0]["maximum_ka"], 2.5);
    close(&a["devices"][1]["maximum_ka"], 0.5);
    close(&a["sources"][0]["maximum_state_ka"], 2.5);
    close(&a["sources"][1]["maximum_state_ka"], 0.5);
    assert_eq!(a["coordination"].as_array().unwrap().len(), 2);
    assert_eq!(a["coordination"][0]["primary_link"], "branch");
    assert_eq!(a["coordination"][0]["backup_link"], "main");
    assert_eq!(a["coordination"][0]["status"], "pass");
    assert!(invoke(&file,"network_faults",json!({})).unwrap()["studies"].as_array().unwrap().is_empty());

    let mut single = ac.clone();
    single["sources"].as_array_mut().unwrap().pop();
    // Source and two links in series, with the third link dead-ended.
    for state in ["minimum", "maximum"] { single["sources"][0][state]["impedance"]["x_mohm"] = json!(100); }
    let s = preview(json!([single]));
    let expected = (1.02 + 0.98 * (-3_f64 * 150.0 / 100.0).exp()) * 2_f64.sqrt() * 400.0 / 150_f64.hypot(100.0);
    close(&s["results"][0]["faults"][0]["devices"][0]["peak_ka"], expected);
    assert_eq!(s["results"][0]["faults"][0]["devices"][0]["peak_basis"], "single_source_ac");
    let mut loop_case = single.clone();
    loop_case["domain"] = json!("ac_loop");
    let s = preview(json!([loop_case]));
    let expected_loop = 400.0 * 3_f64.sqrt() / 150_f64.hypot(100.0);
    close(&s["results"][0]["faults"][0]["maximum_ka"], expected_loop);
    let mut split = ac.clone();
    split["faults"][0]["bus"] = json!("Tee");
    let s = preview(json!([split]));
    assert_eq!(s["results"][0]["faults"][0]["status"], "incomplete");
    assert_eq!(s["results"][0]["faults"][0]["staged_clearing_required"], true);

    ac["sources"][1]["maximum"]["max_terminal_v"] = json!(100);
    let bad = preview(json!([ac]));
    let invalid = &bad["results"][0]["faults"][0];
    assert_eq!(invalid["status"], "incomplete");
    assert!(!invalid["outside_source_envelope"].as_array().unwrap().is_empty());
    ac["sources"][1]["maximum"]["max_terminal_v"] = json!(700);

    ac["links"][0]["device"]["minimum_operating"]["points"] = json!([[50,0.02],[8000,0.01]]);
    let bad = preview(json!([ac]));
    assert_eq!(bad["results"][0]["faults"][0]["coordination"][0]["status"], "fail");
    ac["links"][0]["device"] = device("Main fuse","fuse");
    ac["links"][0]["device"]["direct_curve_max_ka"] = json!(1);
    ac["links"][2]["device"]["documented_peak_ka"] = Value::Null;
    let incomplete = preview(json!([ac]));
    let a = &incomplete["results"][0]["faults"][0];
    assert_eq!(a["coordination"][0]["status"], "incomplete");
    assert!(a["devices"][2]["peak_ka"].is_null());

    ac["links"][2]["from"] = json!("Load");
    ac["links"][2]["to"] = json!("Tee");
    assert_eq!(preview(json!([ac]))["results"][0]["faults"][0]["devices"][2]["status"], "fail");
    let mut invalid_topology = dc.clone();
    invalid_topology["links"][0]["to"] = json!("Battery terminals");
    assert!(invoke(&file,"network_faults",json!({"studies":[invalid_topology],"mode":"save"})).is_err());

    invoke(&file,"use_demo_weather",json!({})).unwrap();
    app::simulate(&file, |_| true).unwrap();
    invoke(&file,"network_faults",json!({"studies":[dc],"mode":"save"})).unwrap();
    assert_eq!(invoke(&file,"network_faults",json!({})).unwrap()["studies"][0]["domain"], "dc_pole_to_pole");
    let report = invoke(&file,"report_preview",json!({"report_type":"system_planner"})).unwrap();
    assert!(report.to_string().contains("Network fault studies"));
    assert!(report.to_string().contains("Example DC fuse"));
    app::undo(&file).unwrap();
    assert!(invoke(&file,"network_faults",json!({})).unwrap()["studies"].as_array().unwrap().is_empty());
    std::fs::remove_file(file).unwrap();
}
