// SPDX-License-Identifier: GPL-3.0-or-later
//! `solarcraft` command-line interface: a thin layer over solarcraft-app.
//! Exit codes: 0 success, 1 error, 2 project has validation errors.
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::Value;
use solarcraft_app as app;
use solarcraft_model::validate::Severity;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "solarcraft", version, about = "SolarCraft photovoltaic design and simulation")]
struct Cli {
    /// Machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode { Easy, Detailed }

#[derive(Clone, Copy, ValueEnum)]
enum Format { Pdf, Docx }

#[derive(Subcommand)]
enum Cmd {
    /// Create a new project (Easy mode).
    New { file: PathBuf, #[arg(long)] name: String, #[arg(long, allow_hyphen_values = true)] lat: f64, #[arg(long, allow_hyphen_values = true)] lon: f64, #[arg(long)] kwp: Option<f64> },
    /// Show the project.
    Show { file: PathBuf },
    /// Set fields: `/json/pointer=value` (value is JSON; bare words are strings).
    Set { file: PathBuf, #[arg(required = true)] assignments: Vec<String> },
    /// Switch design mode (never loses data).
    Mode { file: PathBuf, mode: Mode },
    /// Validate the design.
    Validate { file: PathBuf },
    /// Weather data.
    #[command(subcommand)]
    Weather(WeatherCmd),
    /// Add a user-authored module from datasheet values.
    AddModule { file: PathBuf, #[arg(long)] spec: PathBuf },
    /// Run the annual simulation.
    Simulate { file: PathBuf },
    /// Explain the latest results (loss waterfall and monthly table).
    Explain { file: PathBuf },
    /// Generate the customer report (PDF or editable DOCX).
    Report { file: PathBuf, #[arg(short, long)] out: PathBuf, #[arg(long, default_value = "en")] lang: String, #[arg(long, value_enum, default_value_t = Format::Pdf)] format: Format, #[arg(long, default_value = "")] designer: String },
    /// Undo the last change.
    Undo { file: PathBuf },
    /// Show the change history.
    History { file: PathBuf, #[arg(long, default_value_t = 20)] limit: usize },
    /// Write a compact portable copy.
    Export { file: PathBuf, dest: PathBuf },
    /// Replace a plant section from a JSON file or inline JSON (grid, loads, storage, tariff, economics, electrical, …).
    Section { file: PathBuf, section: String, value: String },
    /// Add an equipment record (module, inverter, battery, battery_inverter) from a JSON file to the project.
    Equipment { file: PathBuf, kind: String, spec: PathBuf },
    /// Import a metered consumption series (one value per line; kWh per interval or kW with --unit kw).
    Load { file: PathBuf, source: PathBuf, #[arg(long, default_value = "kwh")] unit: String },
    /// Show the inverter/MPPT stringing and its checks.
    Stringing { file: PathBuf },
    /// Energy balance, battery and grid totals of the latest run.
    Balance { file: PathBuf },
    /// Hours behind the totals: --day 172, --top import, or --start/--count.
    Hours { file: PathBuf, #[arg(long)] day: Option<u32>, #[arg(long)] top: Option<String>, #[arg(long, default_value_t = 0)] start: u32, #[arg(long, default_value_t = 24)] count: u32 },
    /// Cash flow of the latest run.
    Cashflow { file: PathBuf },
    /// Save the current design as a named variant.
    Variant { file: PathBuf, name: String },
    /// Simulate the current design and all variants side by side.
    Compare { file: PathBuf },
    /// Call any API method with JSON parameters (the same contract as the shells and MCP).
    Call { method: String, #[arg(default_value = "{}")] params: String },
}

#[derive(Subcommand)]
enum WeatherCmd {
    /// Fetch a TMY from PVGIS (sends only the site coordinates).
    Fetch { file: PathBuf },
    /// Import an EPW file or saved PVGIS TMY JSON.
    Import { file: PathBuf, source: PathBuf },
}

fn print<T: serde::Serialize>(json: bool, v: &T, human: impl FnOnce()) {
    if json { println!("{}", serde_json::to_string_pretty(v).unwrap()); } else { human(); }
}

fn parse_assignment(a: &str) -> Result<(String, Value), String> {
    let (k, v) = a.split_once('=').ok_or_else(|| format!("expected /pointer=value, got '{a}'"))?;
    let ptr = if k.starts_with('/') { k.to_string() } else { format!("/{}", k.replace('.', "/")) };
    Ok((ptr, serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.to_string()))))
}

fn run(cli: Cli) -> Result<ExitCode, app::AppError> {
    let j = cli.json;
    match cli.cmd {
        Cmd::New { file, name, lat, lon, kwp } => {
            let p = app::create_project(&file, &name, lat, lon, kwp)?;
            print(j, &p, || println!("Created {} ({:.2} kWp, {} × {} modules, tilt {}°, azimuth {}°)", file.display(), p.arrays[0].target_kwp.value, p.arrays[0].strings.value, p.arrays[0].modules_per_string.value, p.arrays[0].tilt.value, p.arrays[0].azimuth.value));
        }
        Cmd::Show { file } => { let p = app::get_project(&file)?; println!("{}", serde_json::to_string_pretty(&p).unwrap()); }
        Cmd::Set { file, assignments } => {
            let edits = assignments.iter().map(|a| parse_assignment(a)).collect::<Result<Vec<_>, _>>().map_err(|m| app::AppError { code: "INVALID_ARGUMENT", message: m })?;
            let p = app::set_fields(&file, &edits)?;
            print(j, &p, || println!("Updated {} field(s). Array 1: {:.2} kWp, {} × {} modules", edits.len(), p.arrays[0].target_kwp.value, p.arrays[0].strings.value, p.arrays[0].modules_per_string.value));
        }
        Cmd::Mode { file, mode } => {
            let m = match mode { Mode::Easy => solarcraft_model::DesignMode::Easy, Mode::Detailed => solarcraft_model::DesignMode::Detailed };
            app::set_mode(&file, m)?; if !j { println!("Mode: {m:?}"); }
        }
        Cmd::Validate { file } => {
            let issues = app::validate_project(&file)?;
            print(j, &issues, || {
                if issues.is_empty() { println!("No issues."); }
                for i in &issues { println!("{:<8} {:<18} {}  [{}]", format!("{:?}", i.severity).to_uppercase(), i.code, i.message, i.pointer); }
            });
            if issues.iter().any(|i| i.severity == Severity::Error) { return Ok(ExitCode::from(2)); }
        }
        Cmd::Weather(WeatherCmd::Fetch { file }) => { let s = app::fetch_weather_pvgis(&file)?; print(j, &s, || weather_summary(&s)); }
        Cmd::Weather(WeatherCmd::Import { file, source }) => { let s = app::import_weather(&file, &source)?; print(j, &s, || weather_summary(&s)); }
        Cmd::AddModule { file, spec } => {
            let text = std::fs::read_to_string(&spec).map_err(|x| app::AppError { code: "FILE_READ", message: x.to_string() })?;
            let input: app::ModuleInput = serde_json::from_str(&text).map_err(|x| app::AppError { code: "INVALID_ARGUMENT", message: format!("module spec: {x}") })?;
            let m = app::add_module(&file, input)?;
            print(j, &m, || {
                let mp = m.model_params.as_ref().unwrap();
                println!("Added {} {} ({:.0} W). Fit: {}{}", m.manufacturer, m.model, m.pmp_stc(), if mp.fit_exact { "exact" } else if mp.fit_acceptable { "approximate (within limits)" } else { "APPROXIMATE — check datasheet" },
                    if mp.fit_exact { String::new() } else { format!(" (Pmp {:+.2} %, γ {:+.3} %/K)", mp.pmp_rel_err * 100.0, mp.gamma_abs_err) });
            });
        }
        Cmd::Simulate { file } => {
            let r = app::simulate(&file, |f| { if !j { eprint!("\rSimulating… {:>3.0} %", f * 100.0); } true })?;
            if !j { eprintln!(); }
            print(j, &app::api::summary(&r), || {
                let a = &r.annual;
                println!("{:.0} kWh/yr  ·  {:.0} kWh/kWp  ·  PR {:.1} %  ·  {:.2} kWp DC / {:.1} kW AC", a.ac_kwh, a.specific_yield_kwh_kwp, a.performance_ratio * 100.0, a.dc_kwp, a.ac_kw);
                let b = &r.balance;
                if b.load_kwh > 0.0 || b.batt_charge_kwh > 0.0 { println!("Consumption {:.0} kWh · import {:.0} · export {:.0} · self-consumption {:.1} % · autarky {:.1} %", b.load_kwh, b.import_kwh, b.export_kwh, b.self_consumption_rate * 100.0, b.autarky * 100.0); }
                if let Some(f) = &r.finance { println!("NPV {:.0} {} · IRR {} · payback {}", f.npv, f.currency, f.irr_pct.map_or("—".into(), |x| format!("{x:.1} %")), f.payback_years.map_or("—".into(), |x| format!("{x:.1} y"))); }
                for w in &r.warnings { println!("note: {}", w.text); }
            });
        }
        Cmd::Explain { file } => {
            let r = app::latest_results(&file)?.ok_or_else(|| app::AppError { code: "NO_RESULTS", message: "No results yet. Run `solarcraft simulate` first.".into() })?;
            print(j, &serde_json::json!({"irradiance": r.irradiance_ledger, "energy": r.energy_ledger, "monthly": r.monthly, "manifest": r.manifest}), || explain(&r));
        }
        Cmd::Report { file, out, lang, format, designer } => {
            let f = match format { Format::Pdf => app::ReportFormat::Pdf, Format::Docx => app::ReportFormat::Docx };
            let r = app::generate_report(&file, &out, f, &lang, &designer)?;
            print(j, &r, || println!("Wrote {} ({} kB, {})", r.path, r.bytes / 1024, r.lang));
        }
        Cmd::Undo { file } => { match app::undo(&file)? { Some(_) => println!("Undone."), None => println!("Nothing to undo.") } }
        Cmd::History { file, limit } => { for (seq, op, at) in app::history(&file, limit)? { println!("{seq:>5}  {at}  {op}"); } }
        Cmd::Export { file, dest } => { app::export_copy(&file, &dest)?; println!("Wrote {}", dest.display()); }
        Cmd::Section { file, section, value } => {
            let text = if std::path::Path::new(&value).is_file() { std::fs::read_to_string(&value).map_err(|x| app::AppError { code: "FILE_READ", message: x.to_string() })? } else { value };
            let v: Value = serde_json::from_str(&text).map_err(|x| app::AppError { code: "INVALID_ARGUMENT", message: format!("value: {x}") })?;
            let r = api("set_section", serde_json::json!({"file": file, "section": section, "value": v}))?;
            print(j, &r, || { println!("Updated {section}."); issues(&r["issues"]); });
        }
        Cmd::Equipment { file, kind, spec } => {
            let text = std::fs::read_to_string(&spec).map_err(|x| app::AppError { code: "FILE_READ", message: x.to_string() })?;
            let rec: Value = serde_json::from_str(&text).map_err(|x| app::AppError { code: "INVALID_ARGUMENT", message: x.to_string() })?;
            let r = app::plant_ops::add_equipment(&file, &kind, &rec)?;
            print(j, &r, || println!("Added {kind} {} ({} {})", r["id"].as_str().unwrap_or(""), r["manufacturer"].as_str().unwrap_or(""), r["model"].as_str().unwrap_or("")));
        }
        Cmd::Load { file, source, unit } => { let r = app::plant_ops::import_load_series(&file, &source, &unit)?; print(j, &r, || println!("Imported {} values ({} min) — {:.0} kWh/yr", r["values"], r["step_minutes"], r["annual_kwh"].as_f64().unwrap_or(0.0))); }
        Cmd::Stringing { file } => {
            let r = app::plant_ops::stringing(&file)?;
            print(j, &r, || {
                println!("Stringing: {}", if r["manual"].as_bool() == Some(true) { "manual" } else { "derived from the arrays" });
                for u in r["inverters"].as_array().into_iter().flatten() {
                    println!("  {} [{}]{}", u["unit"]["name"].as_str().unwrap_or(""), u["unit"]["id"].as_str().unwrap_or(""), if u["has_battery_port"] == Value::Bool(true) { "  (battery port)" } else { "" });
                    for (i, m) in u["unit"]["mppts"].as_array().into_iter().flatten().enumerate() {
                        let g: Vec<String> = m.as_array().into_iter().flatten().map(|g| format!("{} × {} modules ({})", g["strings"], g["modules_per_string"], g["array_id"].as_str().unwrap_or(""))).collect();
                        println!("    MPPT {}: {}", i + 1, if g.is_empty() { "—".into() } else { g.join(" + ") });
                    }
                }
                issues(&r["issues"]);
            });
        }
        Cmd::Balance { file } => {
            let r = app::latest_results(&file)?.ok_or_else(|| app::AppError { code: "NO_RESULTS", message: "No results yet. Run `solarcraft simulate` first.".into() })?;
            print(j, &serde_json::json!({"balance": r.balance, "electrical": r.electrical}), || balance(&r));
        }
        Cmd::Hours { file, day, top, start, count } => {
            let q = serde_json::json!({"day_of_year": day, "top": top, "start": start, "count": count});
            let r = app::plant_ops::results_hours(&file, &q)?;
            print(j, &r, || {
                println!("{:>5} {:>3} {:>3} {:>5} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>7} {:>7}", "idx", "mo", "day", "hour", "PV AC", "load", "→batt", "batt→", "import", "export", "SOC%", "backup", "curtail");
                for h in r["hours"].as_array().into_iter().flatten() {
                    let g = |k: &str| h[k].as_f64().unwrap_or(0.0);
                    println!("{:>5} {:>3} {:>3} {:>5} {:>7.0} {:>7.0} {:>7.0} {:>7.0} {:>7.0} {:>7.0} {:>6.1} {:>7.0} {:>7.0}  {}", g("index"), g("month"), g("day_of_year"), format!("{:02}:00", g("hour") as u32), g("pv_ac_wh"), g("load_wh"), g("batt_charge_wh"), g("batt_discharge_wh"), g("import_wh"), g("export_wh"), g("soc_pct"), g("backup_wh"), g("curtailed_wh"), if h["outage"] == Value::Bool(true) { "OUTAGE" } else { "" });
                }
                println!("(Wh per hour; totals kWh: {})", r["totals_kwh"]);
            });
        }
        Cmd::Cashflow { file } => {
            let r = app::latest_results(&file)?.ok_or_else(|| app::AppError { code: "NO_RESULTS", message: "No results yet. Run `solarcraft simulate` first.".into() })?;
            let f = r.finance.clone().unwrap_or_default();
            print(j, &f, || {
                println!("Bill year 1 without PV: {:.0} {c}; with PV: {:.0} {c} (export credit {:.0})", f.bill_without.total, f.bill_with.total, f.bill_with.export_credit, c = f.currency);
                println!("{:>4} {:>12} {:>12} {:>10} {:>12} {:>10} {:>12} {:>13}", "year", "savings", "export", "O&M", "replacement", "loan", "cash flow", "cumulative");
                for y in &f.years { println!("{:>4} {:>12.0} {:>12.0} {:>10.0} {:>12.0} {:>10.0} {:>12.0} {:>13.0}", y.year, y.savings, y.export_revenue, y.om, y.replacements, y.loan, y.cash_flow, y.cumulative); }
                println!("Investment {:.0} · NPV {:.0} · IRR {} · payback {} · LCOE {:.3}/kWh", f.investment, f.npv, f.irr_pct.map_or("—".into(), |x| format!("{x:.1} %")), f.payback_years.map_or("—".into(), |x| format!("{x:.1} y")), f.lcoe_per_kwh);
            });
        }
        Cmd::Variant { file, name } => { let v = app::plant_ops::create_variant(&file, &name)?; print(j, &v, || println!("Saved variant {} ({})", v.name, v.id)); }
        Cmd::Compare { file } => {
            let rows = app::plant_ops::compare_variants(&file)?;
            print(j, &rows, || {
                println!("{:<24} {:>7} {:>6} {:>9} {:>7} {:>6} {:>6} {:>9} {:>14} {:>7} {:>7}", "variant", "kWp", "kWh", "AC kWh", "kWh/kWp", "SC %", "aut %", "import", "NPV", "IRR %", "payback");
                for r in &rows {
                    if let Some(e) = &r.error { println!("{:<24} error: {e}", r.name); continue; }
                    println!("{:<24} {:>7.2} {:>6.1} {:>9.0} {:>7.0} {:>6.1} {:>6.1} {:>9.0} {:>14.0} {:>7} {:>7}", r.name, r.dc_kwp, r.battery_kwh, r.ac_kwh, r.specific_yield, r.self_consumption_pct, r.autarky_pct, r.import_kwh, r.npv,
                             r.irr_pct.map_or("—".into(), |x| format!("{x:.1}")), r.payback_years.map_or("—".into(), |x| format!("{x:.1}")));
                }
            });
        }
        Cmd::Call { method, params } => {
            let p: Value = serde_json::from_str(&params).map_err(|x| app::AppError { code: "INVALID_ARGUMENT", message: format!("params: {x}") })?;
            let r = api(&method, p)?;
            println!("{}", serde_json::to_string_pretty(&r).unwrap());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn api(method: &str, params: Value) -> Result<Value, app::AppError> { app::api::dispatch(method, &params, &mut |_| true) }

fn issues(v: &Value) {
    for i in v.as_array().into_iter().flatten() { println!("{:<8} {:<20} {}", i["severity"].as_str().unwrap_or("").to_uppercase(), i["code"].as_str().unwrap_or(""), i["message"].as_str().unwrap_or("")); }
}

fn balance(r: &solarcraft_app::SimResultExt) {
    let b = &r.balance;
    println!("Energy balance (kWh)                         Sources            Uses");
    println!("  PV DC after DC losses                    {:>9.1}", b.pv_dc_kwh);
    println!("  Grid import                              {:>9.1}", b.import_kwh);
    println!("  Consumption served                                       {:>9.1}", b.load_served_kwh);
    println!("  Inverter standby                                         {:>9.1}", b.standby_kwh);
    println!("  Grid export                                              {:>9.1}", b.export_kwh);
    println!("  Curtailed (export limit / outage)                        {:>9.1}", b.curtailed_kwh);
    println!("  Inverter clipping                                        {:>9.1}", b.clipped_kwh);
    println!("  Inverter conversion                                      {:>9.1}", b.inverter_loss_kwh);
    println!("  AC wiring and availability                               {:>9.1}", b.ac_wiring_loss_kwh);
    println!("  Battery conversion                                       {:>9.1}", b.batt_conversion_loss_kwh);
    println!("  Battery self-discharge                                   {:>9.1}", b.self_discharge_kwh);
    println!("  Change in stored energy                                  {:>9.1}", b.stored_delta_kwh);
    println!("  largest hourly residual: {:.1e} Wh", b.max_abs_residual_wh);
    println!("PV AC {:.0} kWh: to load {:.0}, to battery {:.0}; battery out {:.0} ({:.0} full cycles); self-consumption {:.1} %, autarky {:.1} %", b.pv_ac_kwh, b.pv_to_load_kwh, b.batt_charge_kwh - b.batt_charge_grid_kwh, b.batt_discharge_kwh, b.battery_full_cycles, b.self_consumption_rate * 100.0, b.autarky * 100.0);
    if b.outage_hours > 0 { println!("Outages {} h: unserved critical load {:.2} kWh, non-critical load shed {:.1} kWh", b.outage_hours, b.unserved_kwh, b.shed_noncritical_kwh); }
    println!("\nInverter / MPPT                                   kWp     DC kWh   mismatch   limits   limited h");
    for u in &r.electrical {
        println!("  {:<46} {:>6.2} {:>9.0}   DC/AC {:.2}  AC {:.0} kWh  clipped {:.0} kWh", u.name, u.stc_kw, u.dc_kwh, u.dc_ac_ratio, u.ac_kwh, u.clipped_kwh);
        for m in &u.mppts { println!("    MPPT {} {:<38} {:>6.2} {:>9.0} {:>9.1} {:>8.1} {:>9}", m.index, m.strings, m.stc_kw, m.dc_kwh, m.mismatch_kwh, m.limit_loss_kwh, m.limited_hours); }
    }
}

fn weather_summary(s: &app::WeatherSummary) {
    println!("Weather: {}", s.weather_ref.summary);
    println!("Location in data: {:.4}, {:.4}, {:.0} m", s.latitude, s.longitude, s.elevation);
    for n in &s.notes { println!("  · {n}"); }
    println!("Quality: {}{}", if s.qc.ok { "OK" } else { "CHECK" }, if s.qc.messages.is_empty() { String::new() } else { format!(" — {}", s.qc.messages.join("; ")) });
    println!("Licence: {}", s.licence);
}

fn explain(r: &solarcraft_app::SimResultExt) {
    println!("Irradiation (kWh/m²)");
    for s in &r.irradiance_ledger { println!("  {:<46} {:>9.1} {:>+8.1} %", s.label, s.value, if s.key == "ghi" { 0.0 } else { s.delta_pct }); }
    println!("\nEnergy (kWh)");
    for s in &r.energy_ledger {
        if s.key == "nominal" || s.key == "dc_out" || s.key == "grid" { println!("  {:<46} {:>9.0}", s.label, s.value); }
        else { println!("    {:<44} {:>+9.0} {:>+8.2} %", s.label, s.delta, s.delta_pct); }
    }
    println!("  ledger closure error: {:.2e} kWh", r.manifest.ledger_closure_error_kwh);
    println!("\nMonth   GHI kWh/m²   POA kWh/m²     AC kWh");
    for m in &r.monthly { println!("  {:>2}    {:>9.1}    {:>9.1}   {:>8.0}", m.month, m.ghi_kwh_m2, m.poa_kwh_m2, m.ac_kwh); }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(c) => c,
        Err(e) => {
            if json { eprintln!("{}", serde_json::to_string(&e).unwrap()); } else { eprintln!("error: {e}"); }
            ExitCode::from(1)
        }
    }
}
