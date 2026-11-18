// SPDX-License-Identifier: GPL-3.0-or-later
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use solarcraft_app::api::invoke_json;

fn call(method: &str, params: Value) -> Value {
    let response = invoke_json(&json!({"method": method, "params": params}).to_string(), &mut |_| true);
    serde_json::from_str(&response).unwrap()
}

fn ok(method: &str, params: Value) -> Value {
    let r = call(method, params);
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

#[test]
fn photo_preview_apply_undo_and_reports_are_portable() {
    let dir = std::env::temp_dir().join(format!("sc-photo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("project.solarcraft");
    let source = dir.join("roof.png");
    let _ = std::fs::remove_file(&project);
    image::RgbImage::from_fn(256, 192, |x, y| image::Rgb([x as u8, y as u8, 50])).save(&source).unwrap();
    ok("create_project", json!({"file": project, "name": "Photo layout test", "latitude": 4.71, "longitude": -74.07, "target_kwp": 4}));
    ok("use_demo_weather", json!({"file": project}));
    let image = ok("photo_source", json!({"source": source}));
    let control = json!({"corners": [[0.12,0.87],[0.92,0.74],[0.73,0.19],[0.28,0.24]], "width_m": 9.0, "depth_m": 5.0});
    let calibrated = solarcraft_geometry::photo::Calibration::new(&serde_json::from_value(control.clone()).unwrap()).unwrap();
    let keepout = [[0.55, 0.55], [1.25, 0.55], [1.25, 1.25], [0.55, 1.25]].map(|point| calibrated.image_at(point).unwrap());
    let outline = [[0.2, 0.1], [8.9, 0.1], [8.9, 4.8], [6.5, 4.8], [6.5, 4.3], [0.2, 4.3]]
        .map(|point| calibrated.image_at(point).unwrap());
    let request = json!({"file": project, "source": source, "options": {
        "control": control, "source_hash": image["source_hash"], "outline": outline, "obstacles": [keepout], "obstacle_heights": [0.8]
    }});
    let preview = ok("photo_layout", request.clone());
    assert_eq!(preview["preview"], true);
    assert!(preview["roof"]["modules"].as_array().unwrap().len() >= 8);
    assert_eq!(preview["roof"]["face"]["obstacle_heights"], json!([0.8]));
    assert!(ok("get_project", json!({"file": project}))["arrays"][0]["roof"].is_null());
    let png = image::load_from_memory(&STANDARD.decode(preview["png_base64"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!((png.width(), png.height()), (1000, 556));
    let top_left = png.to_rgb8().get_pixel(0, 0).0;
    assert!((65..=77).contains(&top_left[0]) && (42..=51).contains(&top_left[1]));
    assert!(png.to_rgb8().pixels().any(|p| p.0 == [28, 230, 205]));

    let mut apply = request;
    apply["preview"] = json!(false);
    let mut changed = apply.clone();
    changed["options"]["source_hash"] = json!("wrong");
    assert_eq!(call("photo_layout", changed)["error"]["code"], "PHOTO_PLAN");
    ok("photo_layout", apply.clone());
    ok("undo", json!({"file": project}));
    assert!(ok("get_project", json!({"file": project}))["roof_photos"].is_null());
    ok("photo_layout", apply);
    std::fs::remove_file(&source).unwrap();
    let saved = ok("photo_saved", json!({"file": project}));
    assert_eq!(saved["source_hash"], image["source_hash"]);
    assert_eq!(saved["outline"].as_array().unwrap().len(), 6);
    for (actual, expected) in saved["obstacles"][0].as_array().unwrap().iter().zip(keepout) {
        for (c, expected) in actual.as_array().unwrap().iter().zip(expected) {
            assert!((c.as_f64().unwrap() - expected).abs() < 1e-9);
        }
    }
    let resumed = ok("photo_layout", json!({"file": project, "preview": false, "options": {
        "control": saved["control"], "source_hash": saved["source_hash"],
        "outline": saved["outline"], "obstacles": saved["obstacles"], "obstacle_heights": [1.5]
    }}));
    assert_eq!(resumed["roof"]["face"]["obstacle_heights"], json!([1.5]));
    ok("undo", json!({"file": project}));
    assert_eq!(ok("photo_saved", json!({"file": project}))["obstacle_heights"], json!([0.8]));
    ok("set_fields", json!({"file": project, "edits": [
        {"pointer": "/library/modules/0/half_cell", "value": true},
        {"pointer": "/library/modules/0/bypass_diodes", "value": 3}
    ]}));
    let run = solarcraft_app::simulate(&project, &mut |_| true).unwrap();
    let trace = &run.module_shading[0];
    assert!(trace.nonlinear && trace.half_cell);
    assert!(trace.modules.iter().any(|m| m.beam_area_loss_pct > 0.0));
    assert!(run.manifest.ledger_closure_error_kwh.abs() < 1e-4);
    let preview = ok("report_preview", json!({"file": project, "report_type": "system_planner"}));
    assert!(preview["contents"].as_array().unwrap().iter().any(|section| section["id"] == "photo_plan"));
    assert!(preview["contents"].as_array().unwrap().iter().any(|section| section["id"] == "module_shading"));
    assert!(preview.to_string().contains("Parallel half-cells"));
    assert!(!preview.to_string().contains("is not yet modelled"));
    let schedule = ok("design_schedule", json!({"file": project}));
    assert_eq!(schedule["equipment"].as_array().unwrap().iter().find(|row| row["kind"] == "module").unwrap()["quantity"],
        trace.modules.len() as u64);
    let filtered = ok("report_preview", json!({"file": project, "report_type": "system_planner", "include_figures": true,
        "exclude_sections": ["module_shading"]}));
    assert!(!filtered["contents"].as_array().unwrap().iter().any(|section| section["id"] == "module_shading"));
    let photo_section = filtered["contents"].as_array().unwrap().iter().find(|section| section["id"] == "photo_plan").unwrap();
    let figure = STANDARD.decode(photo_section["blocks"][0]["png_base64"].as_str().unwrap()).unwrap();
    assert!(image::load_from_memory(&figure).unwrap().width() > 100);
    for format in ["pdf", "docx"] {
        let output = dir.join(format!("photo.{format}"));
        let r = ok("export_report", json!({"file": project, "output": output, "format": format, "report_type": "system_planner"}));
        assert!(r["bytes"].as_u64().unwrap() > 10_000);
    }
    let filtered_output = dir.join("filtered.docx");
    ok("export_report", json!({"file": project, "output": filtered_output, "format": "docx",
        "report_type": "system_planner", "exclude_sections": ["module_shading", "photo_plan"]}));
    let filtered_size = std::fs::metadata(filtered_output).unwrap().len();
    assert!(filtered_size < std::fs::metadata(dir.join("photo.docx")).unwrap().len());
    std::fs::remove_dir_all(dir).ok();
}
