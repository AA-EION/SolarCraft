// SPDX-License-Identifier: GPL-3.0-or-later
//! An MCP client models a roof in 3D, places and strings modules, renders,
//! exports to IFC and IngeTrazo, and re-imports — all inside the workspace.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

struct Client { child: std::process::Child, out: BufReader<std::process::ChildStdout>, id: u64 }
impl Client {
    fn start(ws: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_solarcraft-mcp")).args(["--workspace", ws.to_str().unwrap()])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Client { child, out, id: 0 }
    }
    fn send(&mut self, v: Value) { let s = self.child.stdin.as_mut().unwrap(); writeln!(s, "{v}").unwrap(); s.flush().unwrap(); }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "server closed");
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == json!(id) { return v; }
        }
    }
    fn call(&mut self, name: &str, args: Value) -> Value { self.request("tools/call", json!({"name": name, "arguments": args})) }
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let r = self.call(name, args);
        assert!(r["result"]["isError"] != json!(true) && r["error"].is_null(), "{name}: {r}");
        r["result"]["structuredContent"].clone()
    }
}

#[test]
fn client_models_a_roof_in_3d() {
    let ws = std::env::temp_dir().join(format!("sc-mcp-scene-{}", std::process::id()));
    std::fs::create_dir_all(&ws).unwrap();
    let mut c = Client::start(&ws);
    c.request("initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let tools = c.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for t in ["scene_get", "scene_edit", "scene_bind", "scene_shading", "scene_render", "scene_export", "scene_import"] { assert!(names.contains(&t), "{t}"); }
    let f = "roof3d.solarcraft";
    c.ok("create_project", json!({"file": f, "name": "MCP 3D", "latitude": 40.4, "longitude": -3.7}));
    c.ok("use_demo_weather", json!({"file": f}));
    let r = c.ok("scene_edit", json!({"file": f, "edits": [
        {"op": "add_object", "name": "Nave", "kind": {"type": "building", "footprint": [[0, 0], [20, 0], [20, 10], [0, 10]], "eave_height": 6, "roof": {"type": "hip", "pitch_deg": 20}}},
        {"op": "add_object", "name": "Pino", "kind": {"type": "tree", "center": [8, -7], "trunk_height": 4, "trunk_radius": 0.3, "crown_radius": 2.5, "crown_height": 7, "transparency": 0.2}},
        {"op": "add_field", "name": "Faldón sur", "mount": {"type": "roof", "face": "bld-1/roof-0"}}]}));
    assert_eq!(r["touched"], json!(["bld-1", "tree-2", "fld-3"]));
    c.ok("scene_bind", json!({"file": f, "field": "fld-3", "array_id": "array-1"}));
    let r = c.ok("scene_edit", json!({"file": f, "edits": [{"op": "place_modules", "field": "fld-3"}]}));
    assert!(r["counts"]["modules"].as_u64().unwrap() > 10);
    let png = c.ok("scene_render", json!({"file": f, "output": "roof.png", "color_by": "material"}));
    assert!(png["bytes"].as_u64().unwrap() > 1000);
    assert!(std::fs::read(ws.join("roof.png")).unwrap().starts_with(b"\x89PNG"));
    for out in ["roof.ifc", "roof.igz", "roof.glb", "roof.dxf"] {
        let e = c.ok("scene_export", json!({"file": f, "output": out}));
        assert!(e["fidelity"]["format"].is_string(), "{e}");
    }
    // Import the IFC into a second project: same objects, previewed first.
    let g = "copy.solarcraft";
    c.ok("create_project", json!({"file": g, "name": "Copy", "latitude": 40.4, "longitude": -3.7}));
    let pv = c.ok("scene_import", json!({"file": g, "source": "roof.ifc", "preview": true}));
    assert_eq!(pv["preview"], json!(true));
    assert_eq!(c.ok("scene_get", json!({"file": g}))["counts"]["objects"], json!(0));
    let imp = c.ok("scene_import", json!({"file": g, "source": "roof.ifc"}));
    assert_eq!(imp["counts"]["objects"], json!(2));
    // Workspace jail: no reading or writing outside it.
    let esc = c.call("scene_import", json!({"file": g, "source": "../outside.ifc"}));
    assert!(esc["error"].is_object() || esc["result"]["isError"] == json!(true), "{esc}");
    let esc = c.call("scene_export", json!({"file": g, "output": "/tmp/escape.glb"}));
    assert!(esc["error"].is_object() || esc["result"]["isError"] == json!(true), "{esc}");
    std::fs::remove_dir_all(&ws).ok();
}
