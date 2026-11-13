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


/// An MCP client completes a storage design: project access, equipment selection from the
/// workspace library, stringing, battery, simulation, hourly inspection, cash flow and export.
#[test]
fn client_designs_battery_system() {
    let ws = std::env::temp_dir().join(format!("sc-mcp-plant-{}", std::process::id()));
    std::fs::create_dir_all(&ws).unwrap();
    let mut c = Client::start(&ws);
    c.request("initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let mut log = vec![];
    let mut call = |c: &mut Client, name: &str, args: Value| -> Value {
        let r = c.call(name, args.clone());
        assert!(r["result"]["isError"] != json!(true), "{name}: {r}");
        let v = r["result"]["structuredContent"].clone();
        log.push(format!("→ {name} {args}\n← {}", { let s = v.to_string(); if s.len() > 300 { format!("{}…", &s[..300]) } else { s } }));
        v
    };
    call(&mut c, "create_project", json!({"file": "mcp.solarcraft", "name": "MCP storage", "latitude": 6.2442, "longitude": -75.5812}));
    call(&mut c, "use_demo_weather", json!({"file": "mcp.solarcraft"}));
    call(&mut c, "set_mode", json!({"file": "mcp.solarcraft", "mode": "detailed"}));
    let inv = json!({"id": "lib-hybrid-6k", "manufacturer": "Datasheet entry", "model": "hybrid 6 kW", "pac_max_w": 6000, "eta_nominal": 0.97, "mppt_v_min": 120, "mppt_v_max": 500, "v_dc_max": 600, "mppt_count": 2, "i_max_per_mppt": 16, "night_w": 2,
        "efficiency_curve": [[0.05, 0.9], [0.2, 0.96], [0.5, 0.972], [1.0, 0.967]], "battery_port": {"v_min": 150, "v_max": 500, "p_charge_max_w": 6000, "p_discharge_max_w": 6000, "dc_dc_efficiency": 0.98}});
    let bat = json!({"id": "lib-bat-15", "manufacturer": "Datasheet entry", "model": "LFP 15 kWh", "chemistry": "lfp", "capacity_kwh": 15, "nominal_voltage_v": 307, "soc_min_pct": 5, "soc_max_pct": 100,
        "p_charge_max_kw": 6, "p_discharge_max_kw": 6, "roundtrip_efficiency": 0.95, "self_discharge_pct_per_month": 2, "cycle_life": 6000});
    call(&mut c, "library_save", json!({"kind": "inverter", "record": inv}));
    call(&mut c, "library_save", json!({"kind": "battery", "record": bat}));
    let found = call(&mut c, "library_list", json!({"query": "hybrid"}));
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    call(&mut c, "library_pin", json!({"file": "mcp.solarcraft", "kind": "inverter", "id": "lib-hybrid-6k"}));
    call(&mut c, "library_pin", json!({"file": "mcp.solarcraft", "kind": "battery", "id": "lib-bat-15"}));
    call(&mut c, "set_fields", json!({"file": "mcp.solarcraft", "edits": [{"pointer": "/arrays/0/inverter_id", "value": "lib-hybrid-6k"}, {"pointer": "/arrays/0/modules_per_string", "value": 8}, {"pointer": "/arrays/0/strings", "value": 2}]}));
    let st = call(&mut c, "get_stringing", json!({"file": "mcp.solarcraft"}));
    let unit = st["inverters"][0]["unit"]["id"].as_str().unwrap().to_string();
    assert_eq!(st["inverters"][0]["has_battery_port"], json!(true));
    call(&mut c, "set_section", json!({"file": "mcp.solarcraft", "section": "loads", "value": {"profile": {"kind": "synthetic", "profile": "residential"}, "annual_kwh": 5000, "critical_fraction": 0.4}}));
    call(&mut c, "set_section", json!({"file": "mcp.solarcraft", "section": "storage", "value": {"battery_id": "lib-bat-15", "units": 1, "coupling": {"kind": "dc_hybrid", "unit_id": unit}, "model": "simple",
        "soc_min_pct": 10, "soc_max_pct": 100, "initial_soc_pct": 50, "backup_reserve_pct": 20, "grid_services_pct": 20, "charge_priority": ["backup", "self_consumption", "grid_services"],
        "grid_charge_backup": true, "grid_service": {"kind": "peak_shaving", "import_limit_kw": 1.0}, "outages": []}}));
    let r = call(&mut c, "simulate", json!({"file": "mcp.solarcraft"}));
    assert!(r["balance"]["batt_discharge_kwh"].as_f64().unwrap() > 100.0, "{r}");
    assert!(r["balance"]["max_abs_residual_wh"].as_f64().unwrap() < 1e-3);
    let peak = call(&mut c, "results_hours", json!({"file": "mcp.solarcraft", "top": "import", "count": 3}));
    assert_eq!(peak["hours"].as_array().unwrap().len(), 3);
    let cf = call(&mut c, "cash_flow", json!({"file": "mcp.solarcraft"}));
    assert!(cf["years"].as_array().unwrap().len() > 10);
    call(&mut c, "export_report", json!({"file": "mcp.solarcraft", "output": "mcp-report.pdf", "format": "pdf", "lang": "es"}));
    assert!(std::fs::read(ws.join("mcp-report.pdf")).unwrap().starts_with(b"%PDF"));
    // Safety: library and inputs stay inside the workspace.
    let esc = c.call("import_load", json!({"file": "mcp.solarcraft", "source": "../etc/passwd"}));
    assert!(esc["error"].is_object() || esc["result"]["isError"] == json!(true), "{esc}");
    if let Ok(dir) = std::env::var("SOLARCRAFT_MCP_TRANSCRIPT") { std::fs::write(dir, log.join("\n\n")).unwrap(); }
    std::fs::remove_dir_all(&ws).ok();
}
