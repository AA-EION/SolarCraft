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
            print(j, &serde_json::json!({"annual": r.annual, "manifest": r.manifest, "warnings": r.warnings}), || {
                let a = &r.annual;
                println!("{:.0} kWh/yr  ·  {:.0} kWh/kWp  ·  PR {:.1} %  ·  {:.2} kWp DC / {:.1} kW AC", a.ac_kwh, a.specific_yield_kwh_kwp, a.performance_ratio * 100.0, a.dc_kwp, a.ac_kw);
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
    }
    Ok(ExitCode::SUCCESS)
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
