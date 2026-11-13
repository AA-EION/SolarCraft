// SPDX-License-Identifier: GPL-3.0-or-later
//! Drives the real binary over stdio JSON-RPC (no network).
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
}

#[test]
fn design_flow_and_safety() {
    let ws = std::env::temp_dir().join(format!("sc-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&ws).unwrap();
    let mut c = Client::start(&ws);
    let init = c.request("initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
    assert!(init["result"]["serverInfo"].is_object(), "{init}");
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let tools = c.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["create_project", "set_fields", "simulate", "explain", "fetch_weather_pvgis", "undo"] { assert!(names.contains(&n), "missing {n}: {names:?}"); }

    let r = c.call("create_project", json!({"file": "a.solarcraft", "name": "Medellín", "latitude": 6.2442, "longitude": -75.5812, "target_kwp": 4}));
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    let r = c.call("set_fields", json!({"file": "a.solarcraft", "edits": [{"pointer": "/arrays/0/tilt", "value": 15}], "dry_run": true}));
    assert_eq!(r["result"]["structuredContent"]["project"]["arrays"][0]["tilt"]["value"], json!(15.0));
    let r = c.call("get_project", json!({"file": "a.solarcraft"}));
    assert_ne!(r["result"]["structuredContent"]["arrays"][0]["tilt"]["value"], json!(15.0), "dry run must not save");
    let r = c.call("validate", json!({"file": "a.solarcraft"}));
    assert_eq!(r["result"]["structuredContent"]["issues"][0]["code"], json!("WEATHER_MISSING"));
    let r = c.call("simulate", json!({"file": "a.solarcraft"}));
    assert_eq!(r["result"]["isError"], json!(true));
    assert_eq!(r["result"]["structuredContent"]["code"], json!("WEATHER_MISSING"));

    let r = c.call("set_roof", json!({"file": "a.solarcraft", "outline": [[0,0],[9,0],[9,5],[0,5]], "obstacles": [[[4,2],[5,2],[5,3],[4,3]]]}));
    assert!(r["result"]["structuredContent"]["modules_fit"].as_u64().unwrap() > 4, "{r}");
    // Offline flow to a report.
    let r = c.call("use_demo_weather", json!({"file": "a.solarcraft"}));
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    let r = c.call("simulate", json!({"file": "a.solarcraft"}));
    assert!(r["result"]["structuredContent"]["annual"]["ac_kwh"].as_f64().unwrap() > 1000.0, "{r}");
    let r = c.call("export_report", json!({"file": "a.solarcraft", "output": "informe.pdf", "format": "pdf", "lang": "es"}));
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    assert!(std::fs::read(ws.join("informe.pdf")).unwrap().starts_with(b"%PDF"));
    let r = c.call("export_report", json!({"file": "a.solarcraft", "output": "informe.pdf", "format": "pdf"}));
    assert!(r["error"].is_object(), "must not overwrite without consent: {r}");
    let r = c.call("export_report", json!({"file": "a.solarcraft", "output": "../out.pdf", "format": "pdf"}));
    assert!(r["error"].is_object());

    // Safety: path escapes, wrong extension, network without consent.
    let r = c.call("get_project", json!({"file": "../etc/passwd.solarcraft"}));
    assert!(r["error"].is_object(), "{r}");
    let r = c.call("create_project", json!({"file": "x.txt", "name": "n", "latitude": 0, "longitude": 0}));
    assert!(r["error"].is_object());
    let r = c.call("fetch_weather_pvgis", json!({"file": "a.solarcraft", "confirm_network": false}));
    assert!(r["error"].is_object() || r["result"]["isError"] == json!(true), "{r}");
    let _ = c.child.kill();
    std::fs::remove_dir_all(&ws).ok();
}
