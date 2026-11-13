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
    "use_demo_weather", "set_roof", "add_module", "simulate", "explain", "export_report", "undo", "history",
    "set_section", "add_equipment", "add_generic_storage_equipment", "import_load", "stringing", "set_stringing", "results_summary", "results_hours",
    "cash_flow", "create_variant", "list_variants", "apply_variant", "delete_variant", "compare_variants",
    "report_preview", "library_list", "library_get", "library_save", "library_delete", "library_history", "library_pin", "library_import",
];

/// Project sections that `set_section` replaces as a whole (validated on save).
pub const SECTIONS: &[&str] = &["grid", "loads", "storage", "tariff", "economics", "electrical", "losses", "simulation", "customer", "site"];

fn lib_path(p: &Value) -> PathBuf { p.get("library").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(app::userlib::default_path) }
fn bad(m: impl std::fmt::Display) -> AppError { AppError { code: "INVALID_ARGUMENT", message: m.to_string() } }

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
        "set_roof" => {
            // {file, array?: 0, outline: [[x,y],...] | null, obstacles?: [[[x,y],...]], params?: {...}}
            let file = path(params, "file")?;
            let idx = params.get("array").and_then(Value::as_u64).unwrap_or(0);
            let ptr = format!("/arrays/{idx}/roof");
            let value = match params.get("outline") {
                None | Some(Value::Null) => Value::Null,
                Some(outline) => {
                    let mut lp = serde_json::to_value(solarcraft_geometry::layout::LayoutParams::default()).unwrap_or(json!({}));
                    if let (Some(over), Some(base)) = (params.get("params").and_then(Value::as_object), lp.as_object_mut()) {
                        for (k, val) in over { base.insert(k.clone(), val.clone()); }
                    }
                    json!({"face": {"outline": outline, "obstacles": params.get("obstacles").cloned().unwrap_or(json!([])), "obstacle_heights": params.get("obstacle_heights").cloned().unwrap_or(json!([]))}, "params": lp, "modules": [], "orientation": "best", "inputs_key": ""})
                }
            };
            let p = app::set_fields(&file, &[(ptr, value)])?;
            let a = p.arrays.get(idx as usize).ok_or_else(|| AppError { code: "INVALID_ARGUMENT", message: "no such array".into() })?;
            Ok(json!({"modules_fit": a.roof.as_ref().map(|r| r.modules.len()), "orientation": a.roof.as_ref().map(|r| r.orientation), "target_kwp": a.target_kwp.value, "strings": a.strings.value, "modules_per_string": a.modules_per_string.value, "issues": solarcraft_model::validate::validate(&p)}))
        }
        "use_demo_weather" => Ok(v(&app::use_demo_weather(&path(params, "file")?)?)),
        "add_module" => {
            let m: app::ModuleInput = serde_json::from_value(arg(params, "module")?.clone()).map_err(|e| AppError { code: "INVALID_ARGUMENT", message: format!("module: {e}") })?;
            Ok(v(&app::add_module(&path(params, "file")?, m)?))
        }
        "simulate" => { let r = app::simulate(&path(params, "file")?, progress)?; Ok(summary(&r)) }
        "results_summary" => match app::latest_results(&path(params, "file")?)? { Some(r) => Ok(summary(&r)), None => Err(AppError { code: "NO_RESULTS", message: "No results yet; run the simulation first.".into() }) },
        "results_hours" => app::plant_ops::results_hours(&path(params, "file")?, params),
        "cash_flow" => match app::latest_results(&path(params, "file")?)? { Some(r) => Ok(v(&r.finance)), None => Err(AppError { code: "NO_RESULTS", message: "No results yet; run the simulation first.".into() }) },
        "set_section" => {
            let sec = s(params, "section")?;
            if !SECTIONS.contains(&sec.as_str()) { return Err(bad(format!("section must be one of {}", SECTIONS.join(", ")))); }
            let p = app::set_fields(&path(params, "file")?, &[(format!("/{sec}"), arg(params, "value")?.clone())])?;
            let issues = solarcraft_model::validate::validate(&p);
            Ok(json!({"section": sec, "value": serde_json::to_value(&p).ok().and_then(|x| x.get(&sec).cloned()), "issues": issues}))
        }
        "add_equipment" => app::plant_ops::add_equipment(&path(params, "file")?, &s(params, "kind")?, arg(params, "record")?),
        "add_generic_storage_equipment" => Ok(json!({"ids": app::plant_ops::add_generic_storage_equipment(&path(params, "file")?)?})),
        "import_load" => app::plant_ops::import_load_series(&path(params, "file")?, &path(params, "source")?, params.get("unit").and_then(Value::as_str).unwrap_or("kwh")),
        "stringing" => app::plant_ops::stringing(&path(params, "file")?),
        "set_stringing" => {
            // {file, manual: bool, inverters?: [...]} — manual=false returns to derived stringing.
            let file = path(params, "file")?;
            let manual = params.get("manual").and_then(Value::as_bool).unwrap_or(true);
            let value = if manual { json!({"manual": true, "inverters": arg(params, "inverters")?}) } else { json!({"manual": false, "inverters": []}) };
            app::set_fields(&file, &[("/electrical".into(), value)])?;
            app::plant_ops::stringing(&file)
        }
        "create_variant" => Ok(v(&app::plant_ops::create_variant(&path(params, "file")?, &s(params, "name")?)?)),
        "list_variants" => Ok(json!({"variants": app::get_project(&path(params, "file")?)?.variants.iter().map(|x| json!({"id": x.id, "name": x.name, "created_unix": x.created_unix})).collect::<Vec<_>>()})),
        "apply_variant" => Ok(v(&app::plant_ops::apply_variant(&path(params, "file")?, &s(params, "id")?, params.get("keep_current").and_then(Value::as_bool).unwrap_or(false))?)),
        "delete_variant" => { app::plant_ops::delete_variant(&path(params, "file")?, &s(params, "id")?)?; Ok(json!({"deleted": true})) }
        "compare_variants" => Ok(json!({"variants": app::plant_ops::compare_variants(&path(params, "file")?)?})),
        "library_list" => Ok(json!({"path": lib_path(params), "items": app::userlib::list(&lib_path(params), params.get("kind").and_then(Value::as_str), params.get("query").and_then(Value::as_str))?})),
        "library_get" => Ok(v(&app::userlib::get(&lib_path(params), &s(params, "kind")?, &s(params, "id")?)?)),
        "library_history" => Ok(json!({"revisions": app::userlib::history(&lib_path(params), &s(params, "kind")?, &s(params, "id")?)?})),
        "library_save" => Ok(v(&app::userlib::put(&lib_path(params), &s(params, "kind")?, arg(params, "record")?, params.get("source").and_then(Value::as_str).unwrap_or("user"))?)),
        "library_delete" => { app::userlib::delete(&lib_path(params), &s(params, "kind")?, &s(params, "id")?)?; Ok(json!({"deleted": true})) }
        "library_pin" => app::userlib::pin(&lib_path(params), &path(params, "file")?, &s(params, "kind")?, &s(params, "id")?),
        "library_import" => Ok(json!({"items": app::userlib::import_file(&lib_path(params), &path(params, "source")?, params.get("source_kind").and_then(Value::as_str).unwrap_or("imported"))?})),
        "explain" => match app::latest_results(&path(params, "file")?)? {
            Some(r) => Ok(json!({"annual": r.annual, "irradiance_kwh_m2": r.irradiance_ledger, "energy_kwh": r.energy_ledger, "monthly": r.monthly, "manifest": r.manifest, "warnings": r.warnings, "balance": r.balance, "electrical": r.electrical})),
            None => Err(AppError { code: "NO_RESULTS", message: "No results yet; run the simulation first.".into() }),
        },
        "export_report" => {
            let fmt: ReportFormat = serde_json::from_value(arg(params, "format")?.clone()).map_err(|e| AppError { code: "INVALID_ARGUMENT", message: format!("format: {e}") })?;
            let lang = params.get("lang").and_then(Value::as_str).unwrap_or("en");
            let designer = params.get("designer").and_then(Value::as_str).unwrap_or("");
            Ok(v(&app::generate_report(&path(params, "file")?, &path(params, "output")?, fmt, lang, designer)?))
        }
        "report_preview" => Ok(v(&app::report_view(&path(params, "file")?, params.get("lang").and_then(Value::as_str).unwrap_or("en"))?)),
        "undo" => Ok(json!({"undone": app::undo(&path(params, "file")?)?.is_some()})),
        "history" => Ok(json!({"changes": app::history(&path(params, "file")?, 50)?.into_iter().map(|(seq, op, at)| json!({"seq": seq, "op": op, "at_unix": at})).collect::<Vec<_>>()})),
        _ => Err(AppError { code: "UNKNOWN_METHOD", message: format!("unknown method '{method}'") }),
    }
}

/// Compact result summary (hourly series are fetched with `results_hours`).
pub fn summary(r: &solarcraft_engine::SimResult) -> Value {
    json!({"annual": r.annual, "manifest": r.manifest, "warnings": r.warnings, "balance": r.balance, "electrical": r.electrical,
           "finance": r.finance.as_ref().map(|f| json!({"currency": f.currency, "investment": f.investment, "npv": f.npv, "irr_pct": f.irr_pct, "payback_years": f.payback_years, "lcoe_per_kwh": f.lcoe_per_kwh, "bill_without": f.bill_without, "bill_with": f.bill_with}))})
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
        let roof = call("set_roof", json!({"file": fs, "outline": [[0,0],[8,0],[8,5],[0,5]], "obstacles": [[[3,2],[4,2],[4,3],[3,3]]], "params": {"setback": 0.5}}));
        assert!(roof["result"]["modules_fit"].as_u64().unwrap() > 6, "{roof}");
        assert_eq!(call("set_roof", json!({"file": fs, "outline": null}))["result"]["modules_fit"], json!(null));
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
