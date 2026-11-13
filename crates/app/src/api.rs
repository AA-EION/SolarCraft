// SPDX-License-Identifier: GPL-3.0-or-later
//! JSON method dispatch for the native shells (via `sc_invoke` in the C ABI).
//! Method names and parameters match the MCP tools, so every client speaks
//! one contract. Paths here are absolute (chosen in native file dialogs);
//! workspace jailing is the MCP server's job.
use crate::{self as app, AppError, ReportFormat};
use serde_json::{json, Value};
use std::path::PathBuf;

fn arg<'a>(p: &'a Value, k: &str) -> Result<&'a Value, AppError> {
    p.get(k).ok_or_else(|| AppError { code: "INVALID_ARGUMENT", message: format!("missing parameter '{k}'") })
}
fn s(p: &Value, k: &str) -> Result<String, AppError> {
    arg(p, k)?.as_str().map(String::from).ok_or_else(|| AppError { code: "INVALID_ARGUMENT", message: format!("'{k}' must be a string") })
}
fn f(p: &Value, k: &str) -> Result<f64, AppError> {
    arg(p, k)?.as_f64().ok_or_else(|| AppError { code: "INVALID_ARGUMENT", message: format!("'{k}' must be a number") })
}
fn path(p: &Value, k: &str) -> Result<PathBuf, AppError> { Ok(PathBuf::from(s(p, k)?)) }
fn v<T: serde::Serialize>(x: &T) -> Value { serde_json::to_value(x).unwrap_or(Value::Null) }

pub const METHODS: &[&str] = &[
    "version", "ui_strings", "create_project", "get_project", "set_fields", "set_mode", "validate", "fetch_weather_pvgis", "import_weather",
    "use_demo_weather", "add_module", "simulate", "explain", "export_report", "undo", "history",
];

/// Dispatch `method` with JSON `params`. `progress(fraction) -> continue?` is used by long methods.
pub fn dispatch(method: &str, params: &Value, progress: &mut dyn FnMut(f64) -> bool) -> Result<Value, AppError> {
    match method {
        "version" => Ok(json!({"core": env!("CARGO_PKG_VERSION"), "engine": solarcraft_engine::ENGINE_VERSION, "methods": METHODS})),
        "ui_strings" => Ok(v(&crate::ui_strings::strings(params.get("lang").and_then(Value::as_str).unwrap_or("en")))),
        "create_project" => Ok(v(&app::create_project(&path(params, "file")?, &s(params, "name")?, f(params, "latitude")?, f(params, "longitude")?, params.get("target_kwp").and_then(Value::as_f64))?)),
        "get_project" => Ok(v(&app::get_project(&path(params, "file")?)?)),
        "set_fields" => {
            let file = path(params, "file")?;
            let edits: Vec<(String, Value)> = arg(params, "edits")?.as_array().ok_or_else(|| AppError { code: "INVALID_ARGUMENT", message: "'edits' must be an array".into() })?
                .iter().map(|e| Ok((s(e, "pointer")?, arg(e, "value")?.clone()))).collect::<Result<_, AppError>>()?;
            if params.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
                let mut p = app::get_project(&file)?;
                for (k, val) in &edits { p.set_by_pointer(k, val.clone()).map_err(|m| AppError { code: "INVALID_VALUE", message: m })?; }
                return Ok(json!({"dry_run": true, "project": p, "issues": solarcraft_model::validate::validate(&p)}));
            }
            let p = app::set_fields(&file, &edits)?;
            Ok(json!({"project": p, "issues": solarcraft_model::validate::validate(&p)}))
        }
        "set_mode" => {
            let m = match s(params, "mode")?.as_str() { "easy" => solarcraft_model::DesignMode::Easy, "detailed" => solarcraft_model::DesignMode::Detailed, _ => return Err(AppError { code: "INVALID_ARGUMENT", message: "mode must be easy or detailed".into() }) };
            Ok(v(&app::set_mode(&path(params, "file")?, m)?))
        }
        "validate" => Ok(json!({"issues": app::validate_project(&path(params, "file")?)?})),
        "fetch_weather_pvgis" => {
            if params.get("confirm_network").and_then(Value::as_bool) != Some(true) { return Err(AppError { code: "CONSENT_REQUIRED", message: "confirm_network must be true: sends the site coordinates to PVGIS".into() }); }
            Ok(v(&app::fetch_weather_pvgis(&path(params, "file")?)?))
        }
        "import_weather" => Ok(v(&app::import_weather(&path(params, "file")?, &path(params, "source")?)?)),
        "use_demo_weather" => Ok(v(&app::use_demo_weather(&path(params, "file")?)?)),
        "add_module" => {
            let m: app::ModuleInput = serde_json::from_value(arg(params, "module")?.clone()).map_err(|e| AppError { code: "INVALID_ARGUMENT", message: format!("module: {e}") })?;
            Ok(v(&app::add_module(&path(params, "file")?, m)?))
        }
        "simulate" => { let r = app::simulate(&path(params, "file")?, progress)?; Ok(json!({"annual": r.annual, "manifest": r.manifest, "warnings": r.warnings})) }
        "explain" => match app::latest_results(&path(params, "file")?)? {
            Some(r) => Ok(json!({"annual": r.annual, "irradiance_kwh_m2": r.irradiance_ledger, "energy_kwh": r.energy_ledger, "monthly": r.monthly, "manifest": r.manifest, "warnings": r.warnings})),
            None => Err(AppError { code: "NO_RESULTS", message: "No results yet; run the simulation first.".into() }),
        },
        "export_report" => {
            let fmt: ReportFormat = serde_json::from_value(arg(params, "format")?.clone()).map_err(|e| AppError { code: "INVALID_ARGUMENT", message: format!("format: {e}") })?;
            let lang = params.get("lang").and_then(Value::as_str).unwrap_or("en");
            let designer = params.get("designer").and_then(Value::as_str).unwrap_or("");
            Ok(v(&app::generate_report(&path(params, "file")?, &path(params, "output")?, fmt, lang, designer)?))
        }
        "undo" => Ok(json!({"undone": app::undo(&path(params, "file")?)?.is_some()})),
        "history" => Ok(json!({"changes": app::history(&path(params, "file")?, 50)?.into_iter().map(|(seq, op, at)| json!({"seq": seq, "op": op, "at_unix": at})).collect::<Vec<_>>()})),
        _ => Err(AppError { code: "UNKNOWN_METHOD", message: format!("unknown method '{method}'") }),
    }
}

/// Envelope used over the C ABI: `{"method": "...", "params": {...}}` →
/// `{"ok": true, "result": ...}` or `{"ok": false, "error": {"code", "message"}}`.
pub fn invoke_json(request: &str, progress: &mut dyn FnMut(f64) -> bool) -> String {
    let resp = match serde_json::from_str::<Value>(request) {
        Err(e) => json!({"ok": false, "error": {"code": "INVALID_REQUEST", "message": e.to_string()}}),
        Ok(req) => {
            let method = req.get("method").and_then(Value::as_str).unwrap_or("");
            let params = req.get("params").cloned().unwrap_or(json!({}));
            match dispatch(method, &params, progress) {
                Ok(result) => json!({"ok": true, "result": result}),
                Err(e) => json!({"ok": false, "error": {"code": e.code, "message": e.message}}),
            }
        }
    };
    resp.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_offline_flow_through_json() {
        let dir = std::env::temp_dir().join(format!("sc-api-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("p.solarcraft");
        let _ = std::fs::remove_file(&f);
        let fs = f.to_str().unwrap();
        let call = |m: &str, p: Value| -> Value { serde_json::from_str(&invoke_json(&json!({"method": m, "params": p}).to_string(), &mut |_| true)).unwrap() };
        assert_eq!(call("create_project", json!({"file": fs, "name": "Cali", "latitude": 3.4516, "longitude": -76.532, "target_kwp": 3}))["ok"], json!(true));
        assert_eq!(call("simulate", json!({"file": fs}))["error"]["code"], json!("WEATHER_MISSING"));
        assert_eq!(call("fetch_weather_pvgis", json!({"file": fs}))["error"]["code"], json!("CONSENT_REQUIRED"));
        assert_eq!(call("use_demo_weather", json!({"file": fs}))["ok"], json!(true));
        let r = call("simulate", json!({"file": fs}));
        assert!(r["result"]["annual"]["ac_kwh"].as_f64().is_some_and(|x| x > 1000.0), "{r}");
        let e = call("explain", json!({"file": fs}));
        assert_eq!(e["result"]["monthly"].as_array().unwrap().len(), 12);
        let out = dir.join("r.pdf");
        let rep = call("export_report", json!({"file": fs, "output": out.to_str().unwrap(), "format": "pdf", "lang": "es"}));
        assert_eq!(rep["ok"], json!(true), "{rep}");
        assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF"));
        assert_eq!(call("nope", json!({}))["error"]["code"], json!("UNKNOWN_METHOD"));
        assert_eq!(serde_json::from_str::<Value>(&invoke_json("{bad", &mut |_| true)).unwrap()["error"]["code"], json!("INVALID_REQUEST"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
