// SPDX-License-Identifier: GPL-3.0-or-later
//! 3D workflow through the JSON API the shells, CLI and MCP share:
//! scene authoring → module field on a roof face → manual module removal →
//! strings → binding to the electrical array → simulation with per-module
//! shading (bypass diodes) → editing an obstruction changes the yield →
//! undo → viewport and picking → CAD/BIM export and import → reopen.
use serde_json::{json, Value};
use solarcraft_app::api::invoke_json;

fn call(m: &str, p: Value) -> Value { serde_json::from_str(&invoke_json(&json!({"method": m, "params": p}).to_string(), &mut |_| true)).unwrap() }
fn ok(m: &str, p: Value) -> Value { let r = call(m, p); assert_eq!(r["ok"], json!(true), "{m}: {r}"); r["result"].clone() }

fn tree_at(x: f64, y: f64) -> Value {
    json!({"type": "tree", "center": [x, y], "trunk_height": 3.0, "trunk_radius": 0.3, "crown_radius": 3.0, "crown_height": 6.0, "transparency": 0.25, "segments": 16})
}

#[test]
fn scene_workflow() {
    let dir = std::env::temp_dir().join(format!("sc-scene-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("casa3d.solarcraft");
    let _ = std::fs::remove_file(&f);
    let fs = f.to_str().unwrap();
    ok("create_project", json!({"file": fs, "name": "Casa 3D", "latitude": 4.711, "longitude": -74.072}));
    ok("set_mode", json!({"file": fs, "mode": "detailed"}));
    ok("use_demo_weather", json!({"file": fs}));

    // 1. Scene: gable house with a roof window, a chimney and a tree to the south-east.
    let r = ok("scene_edit", json!({"file": fs, "edits": [
        {"op": "add_object", "name": "Casa", "layer": "building", "kind": {"type": "building", "footprint": [[0, 0], [14, 0], [14, 9], [0, 9]], "eave_height": 5.5, "roof": {"type": "gable", "pitch_deg": 25}, "openings": [[[1.0, 0.8], [2.2, 0.8], [2.2, 2.2], [1.0, 2.2]]]}},
        {"op": "add_object", "name": "Chimenea", "kind": {"type": "block", "center": [11.5, 2.5], "base_z": 5.0, "length": 0.7, "width": 0.7, "height": 2.6}},
        {"op": "add_object", "name": "Árbol", "layer": "vegetation", "kind": tree_at(20.0, -9.0)},
    ]}));
    assert_eq!(r["touched"], json!(["bld-1", "blk-2", "tree-3"]));
    let south = r["roof_faces"].as_array().unwrap().iter().find(|f| f["id"] == "bld-1/roof-0").unwrap().clone();
    assert!((south["tilt_deg"].as_f64().unwrap() - 25.0).abs() < 1e-9 && (south["azimuth_deg"].as_f64().unwrap() - 180.0).abs() < 1e-9);
    assert!(r["issues"].as_array().unwrap().iter().all(|i| i["severity"] != "error"), "{}", r["issues"]);

    // Preview changes nothing; an invalid transaction changes nothing either.
    let pv = ok("scene_edit", json!({"file": fs, "preview": true, "edits": [{"op": "remove_object", "id": "blk-2"}]}));
    assert_eq!(pv["counts"]["objects"], json!(2));
    assert_eq!(ok("scene_get", json!({"file": fs}))["counts"]["objects"], json!(3));
    let bad = call("scene_edit", json!({"file": fs, "edits": [{"op": "move", "id": "blk-2", "dx": 1, "dy": 0}, {"op": "remove_object", "id": "nope"}]}));
    assert_eq!(bad["error"]["code"], json!("SCENE_EDIT"));

    // 2. Module field on the south face, bound to the array (takes the array's module size).
    ok("scene_edit", json!({"file": fs, "edits": [{"op": "add_field", "name": "Cubierta sur", "mount": {"type": "roof", "face": "bld-1/roof-0"}}]}));
    let b = ok("scene_bind", json!({"file": fs, "field": "fld-4", "array_id": "array-1"}));
    assert_eq!(b["field"], json!("fld-4"));
    let r = ok("scene_edit", json!({"file": fs, "edits": [{"op": "place_modules", "field": "fld-4"}]}));
    let n = r["counts"]["modules"].as_u64().unwrap() as usize;
    let p = ok("get_project", json!({"file": fs}));
    let mps = p["arrays"][0]["modules_per_string"]["value"].as_u64().unwrap().min(n as u64) as usize;
    let mps = mps.clamp(4, 9);
    // Remove the leftover modules by hand so every string is complete.
    let extra: Vec<String> = (0..n % mps).map(|k| format!("fld-4/m{:04}", n - 1 - k)).collect();
    let r = ok("scene_edit", json!({"file": fs, "edits": [{"op": "remove_modules", "field": "fld-4", "modules": extra}, {"op": "route_strings", "field": "fld-4", "modules_per_string": mps, "inverter_at": [0.0, -0.5, 1.0]}]}));
    let strings = r["counts"]["strings"].as_u64().unwrap();
    assert!(strings >= 1 && r["counts"]["modules"].as_u64().unwrap() == strings * mps as u64);
    let b = ok("scene_bind", json!({"file": fs, "field": "fld-4", "array_id": "array-1"}));
    assert_eq!((b["tilt"].as_f64().unwrap(), b["azimuth"].as_f64().unwrap()), (25.0, 180.0));
    assert_eq!((b["modules_per_string"].as_u64().unwrap(), b["strings"].as_u64().unwrap()), (mps as u64, strings));
    // One generic 5 kW inverter per string keeps each MPPT within its current limit.
    ok("set_fields", json!({"file": fs, "edits": [{"pointer": "/arrays/0/inverter_count", "value": strings}]}));
    let issues = ok("validate", json!({"file": fs}))["issues"].as_array().unwrap().clone();
    assert!(issues.iter().all(|i| i["severity"] != "error" && !i["code"].as_str().unwrap().starts_with("SCENE_FIELD")), "{issues:?}");

    // 3. Simulate: per-module shading with bypass diodes, and the ledger shows it.
    ok("simulate", json!({"file": fs}));
    let ex = ok("explain", json!({"file": fs}));
    let e1 = ex["annual"]["ac_kwh"].as_f64().unwrap();
    assert!(ex["warnings"].as_array().unwrap().iter().any(|w| w["code"] == "SCENE_SHADING"), "{}", ex["warnings"]);
    let steps: Vec<&str> = ex["irradiance_kwh_m2"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
    assert_eq!(steps, vec!["ghi", "poa", "shading", "bypass", "iam", "soiling", "effective"]);
    let sh = ok("scene_shading", json!({"file": fs}));
    let mods = sh["fields"][0]["modules"].as_array().unwrap();
    assert_eq!(mods.len() as u64, strings * mps as u64);
    let worst = mods.iter().map(|m| m["beam_loss_pct"].as_f64().unwrap()).fold(0.0, f64::max);
    let best = mods.iter().map(|m| m["beam_loss_pct"].as_f64().unwrap()).fold(f64::MAX, f64::min);
    assert!(worst > best, "the chimney shades some modules more than others ({best}..{worst})");

    // 4. Move the tree right in front of the roof: more shade, less energy. Undo restores it exactly.
    ok("scene_edit", json!({"file": fs, "edits": [{"op": "move", "id": "tree-3", "dx": -13.0, "dy": 3.0}]}));
    ok("simulate", json!({"file": fs}));
    let e2 = ok("results_summary", json!({"file": fs}))["annual"]["ac_kwh"].as_f64().unwrap();
    assert!(e2 < e1 * 0.995, "tree in front: {e2} vs {e1}");
    ok("undo", json!({"file": fs}));
    ok("simulate", json!({"file": fs}));
    let e3 = ok("results_summary", json!({"file": fs}))["annual"]["ac_kwh"].as_f64().unwrap();
    assert_eq!(e3, e1, "undo restores the scene and the result bit for bit");

    // 5. Viewport: polygons, picking a module, SVG with the shading map.
    let cam = json!({"yaw_deg": 200, "pitch_deg": 40, "width": 800, "height": 600});
    let v = ok("scene_view", json!({"file": fs, "camera": cam, "color_by": "shading"}));
    let module = v["polys"].as_array().unwrap().iter().rev().find(|p| p["kind"] == "module").unwrap().clone();
    let pts = module["points"].as_array().unwrap();
    let (cx, cy) = pts.iter().fold((0.0, 0.0), |a, q| (a.0 + q[0].as_f64().unwrap() / 4.0, a.1 + q[1].as_f64().unwrap() / 4.0));
    let hit = ok("scene_pick", json!({"file": fs, "camera": cam, "x": cx, "y": cy}));
    assert_eq!(hit["hit"], module["id"], "z-buffer pick at the module centre");
    let img = ok("scene_image", json!({"file": fs, "camera": cam, "color_by": "shading"}));
    assert_eq!((img["width"].as_u64(), img["height"].as_u64()), (Some(800), Some(600)));
    assert!(img["png_base64"].as_str().unwrap().starts_with("iVBORw0KGgo"), "PNG signature");
    assert_eq!(hit["field"]["array_id"], json!("array-1"));
    let svg = ok("scene_svg", json!({"file": fs, "camera": cam, "color_by": "string", "output": dir.join("view.svg").to_str().unwrap()}));
    assert!(svg["bytes"].as_u64().unwrap() > 1000);

    // 6. Exchange: every format writes; IFC comes back exactly into a new project.
    for ext in ["glb", "gltf", "obj", "dxf", "ifc", "igz", "json"] {
        let out = dir.join(format!("casa3d.{ext}"));
        let r = ok("scene_export", json!({"file": fs, "output": out.to_str().unwrap()}));
        assert!(r["fidelity"]["max_coordinate_error_m"].as_f64().unwrap() < 1e-5, "{ext}: {}", r["fidelity"]);
    }
    let g = dir.join("copy.solarcraft");
    let _ = std::fs::remove_file(&g);
    ok("create_project", json!({"file": g.to_str().unwrap(), "name": "Copia", "latitude": 4.711, "longitude": -74.072}));
    let imp = ok("scene_import", json!({"file": g.to_str().unwrap(), "source": dir.join("casa3d.ifc").to_str().unwrap()}));
    assert_eq!(imp["fidelity"]["solarcraft_metadata"], json!(true));
    let a = ok("scene_get", json!({"file": fs}));
    let c = ok("scene_get", json!({"file": g.to_str().unwrap()}));
    let geometry = |scene: &Value| scene["scene"]["objects"].as_array().unwrap().iter().map(|object| {
        (object["id"].clone(), object["kind"].clone())
    }).collect::<Vec<_>>();
    assert_eq!(geometry(&a), geometry(&c));
    assert_eq!(a["counts"]["modules"], c["counts"]["modules"]);
    let relinked = ok("scene_import", json!({"file": g.to_str().unwrap(),
        "source": dir.join("casa3d.ifc").to_str().unwrap(), "options": {"relink": true}}));
    assert_eq!(relinked["counts"]["objects"], c["counts"]["objects"]);
    assert_eq!(relinked["counts"]["fields"], c["counts"]["fields"]);
    assert_eq!(relinked["counts"]["modules"], c["counts"]["modules"]);
    let unsupported = call("scene_import", json!({"file": g.to_str().unwrap(), "source": dir.join("model.skp").to_str().unwrap()}));
    assert_eq!(unsupported["error"]["code"], json!("FORMAT_NOT_SUPPORTED"));

    // 7. Reopen: the same inputs give the same result.
    let h1 = ok("results_summary", json!({"file": fs}))["manifest"]["inputs_hash"].clone();
    ok("simulate", json!({"file": fs}));
    assert_eq!(ok("results_summary", json!({"file": fs}))["manifest"]["inputs_hash"], h1);
    // The native roof placement command keeps both roof faces electrically bound.
    let multi = dir.join("multi.solarcraft");
    ok("create_project", json!({"file": multi, "name": "Multi-roof", "latitude": 4.711, "longitude": -74.072}));
    ok("use_demo_weather", json!({"file": multi}));
    ok("scene_edit", json!({"file": multi, "edits": [{"op": "add_object", "name": "House",
        "kind": {"type": "building", "footprint": [[0,0],[15,0],[15,10],[0,10]],
        "eave_height": 5, "roof": {"type": "gable", "pitch_deg": 25}}}]}));
    let south = ok("scene_place_roof", json!({"file": multi, "face": "bld-1/roof-0"}));
    let north = ok("scene_place_roof", json!({"file": multi, "face": "bld-1/roof-1"}));
    assert_ne!(south["array"], north["array"]);
    let expected = [south["array"].clone(), north["array"].clone()];
    let scene = ok("scene_get", json!({"file": multi}));
    for (field, array) in scene["scene"]["fields"].as_array().unwrap().iter().zip(expected) {
        assert_eq!(field["array_id"], array);
    }
    let result = ok("simulate", json!({"file": multi}));
    assert_eq!(result["electrical"].as_array().unwrap().len(), 2);
    let shaded = ok("scene_shading", json!({"file": multi}));
    assert_eq!(shaded["fields"].as_array().unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
