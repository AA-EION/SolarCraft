// SPDX-License-Identifier: GPL-3.0-or-later
//! Application services. Every user-visible operation is a function here,
//! taking a project path and typed arguments and returning serialisable
//! results or an [`AppError`] with a stable code. The CLI, MCP server and
//! native shells call these functions and nothing else (brief: "one
//! documented computation path").
use serde::Serialize;
use serde_json::Value;
use solarcraft_engine::SimResult;
use solarcraft_model::equipment::{ModuleSpec, Provenance};
use solarcraft_model::validate::{validate, Issue};
use solarcraft_model::{DesignMode, Project, WeatherRef};
use solarcraft_store::Store;
use solarcraft_weather::{QcReport, Weather};
use std::path::Path;

pub mod api;
pub mod ui_strings;

#[derive(Debug, Clone, Serialize)]
pub struct AppError { pub code: &'static str, pub message: String }
impl std::fmt::Display for AppError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{} ({})", self.message, self.code) } }
impl std::error::Error for AppError {}
pub type Result<T> = std::result::Result<T, AppError>;
fn e(code: &'static str, m: impl std::fmt::Display) -> AppError { AppError { code, message: m.to_string() } }

fn open(path: &Path) -> Result<(Store, Project)> {
    let s = Store::open(path).map_err(|x| e("PROJECT_OPEN", x))?;
    let p = s.load().map_err(|x| e("PROJECT_LOAD", x))?;
    Ok((s, p))
}
fn save(s: &mut Store, p: &Project, op: &str) -> Result<()> { s.save(p, op).map_err(|x| e("PROJECT_SAVE", x)) }

pub fn create_project(path: &Path, name: &str, lat: f64, lon: f64, target_kwp: Option<f64>) -> Result<Project> {
    let id = format!("prj-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let mut p = Project::new(&id, name, lat, lon);
    if let Some(k) = target_kwp { p.set_by_pointer("/arrays/0/target_kwp", Value::from(k)).map_err(|m| e("INVALID_VALUE", m))?; }
    Store::create(path, &p).map_err(|x| e("PROJECT_CREATE", x))?;
    Ok(p)
}

pub fn get_project(path: &Path) -> Result<Project> { Ok(open(path)?.1) }

/// Set one or more fields by JSON pointer in a single transaction.
pub fn set_fields(path: &Path, edits: &[(String, Value)]) -> Result<Project> {
    let (mut s, mut p) = open(path)?;
    for (ptr, v) in edits { p.set_by_pointer(ptr, v.clone()).map_err(|m| e("INVALID_VALUE", m))?; }
    let op = format!("set {}", edits.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", "));
    save(&mut s, &p, &op)?;
    Ok(p)
}

pub fn set_mode(path: &Path, mode: DesignMode) -> Result<Project> {
    let (mut s, mut p) = open(path)?;
    p.set_mode(mode);
    save(&mut s, &p, &format!("mode {mode:?}"))?;
    Ok(p)
}

pub fn validate_project(path: &Path) -> Result<Vec<Issue>> { Ok(validate(&open(path)?.1)) }

#[derive(Debug, Clone, Serialize)]
pub struct WeatherSummary { pub weather_ref: WeatherRef, pub qc: QcReport, pub latitude: f64, pub longitude: f64, pub elevation: f64, pub notes: Vec<String>, pub licence: String }

fn attach_weather(path: &Path, w: Weather) -> Result<WeatherSummary> {
    let (mut s, mut p) = open(path)?;
    let qc = solarcraft_weather::quality_check(&w);
    let bytes = serde_json::to_vec(&w).map_err(|x| e("WEATHER_ENCODE", x))?;
    let hash = s.put_blob("weather", "application/vnd.solarcraft.weather+json", &bytes).map_err(|x| e("PROJECT_SAVE", x))?;
    let wref = WeatherRef { hash, provider: w.provenance.provider.clone(), summary: format!("{} — {:.0} kWh/m²·yr GHI, {} h", w.provenance.dataset, qc.annual_ghi_kwh_m2, qc.hours) };
    p.weather = Some(wref.clone());
    if w.elevation.is_finite() && !p.site.elevation.is_user() { p.site.elevation.derive(w.elevation); }
    save(&mut s, &p, &format!("weather {}", w.provenance.provider))?;
    Ok(WeatherSummary { weather_ref: wref, qc, latitude: w.latitude, longitude: w.longitude, elevation: w.elevation, notes: w.provenance.notes.clone(), licence: w.provenance.licence.clone() })
}

/// Fetch a PVGIS TMY for the project site (sends only coordinates to PVGIS).
pub fn fetch_weather_pvgis(path: &Path) -> Result<WeatherSummary> {
    let p = get_project(path)?;
    let w = solarcraft_weather::fetch_pvgis_tmy(p.site.latitude.value, p.site.longitude.value).map_err(|x| e("WEATHER_FETCH", x))?;
    attach_weather(path, w)
}

/// Import an EPW file or a saved PVGIS TMY JSON file.
pub fn import_weather(path: &Path, file: &Path) -> Result<WeatherSummary> {
    let text = std::fs::read_to_string(file).map_err(|x| e("FILE_READ", format!("{}: {x}", file.display())))?;
    let name = file.display().to_string();
    let w = if text.trim_start().starts_with('{') { solarcraft_weather::parse_pvgis_tmy(&text, &name) } else { solarcraft_weather::parse_epw(&text, &name) }
        .map_err(|x| e("WEATHER_PARSE", x))?;
    attach_weather(path, w)
}

/// Attach offline clear-sky demonstration weather (no network; clearly labelled).
pub fn use_demo_weather(path: &Path) -> Result<WeatherSummary> {
    let p = get_project(path)?;
    attach_weather(path, solarcraft_weather::synthetic_clear_sky(p.site.latitude.value, p.site.longitude.value, 2025))
}

pub fn load_weather(s: &Store, p: &Project) -> Result<(Weather, String)> {
    let r = p.weather.as_ref().ok_or_else(|| e("WEATHER_MISSING", "No weather data. Fetch PVGIS data or import a weather file."))?;
    let bytes = s.get_blob(&r.hash).map_err(|x| e("PROJECT_LOAD", x))?.ok_or_else(|| e("WEATHER_MISSING", "weather blob missing from project file"))?;
    Ok((serde_json::from_slice(&bytes).map_err(|x| e("WEATHER_DECODE", x))?, r.hash.clone()))
}

/// Run the annual simulation and store the results in the project file.
pub fn simulate(path: &Path, progress: impl FnMut(f64) -> bool) -> Result<SimResult> {
    let (mut s, p) = open(path)?;
    let (w, hash) = load_weather(&s, &p)?;
    let r = solarcraft_engine::simulate(&p, &w, &hash, progress).map_err(|x| if x.0 == "cancelled" { e("CANCELLED", x) } else { e("SIMULATION", x) })?;
    let bytes = serde_json::to_vec(&r).map_err(|x| e("RESULT_ENCODE", x))?;
    s.put_blob("results", "application/vnd.solarcraft.results+json", &bytes).map_err(|x| e("PROJECT_SAVE", x))?;
    Ok(r)
}

/// Latest stored results, if any.
pub fn latest_results(path: &Path) -> Result<Option<SimResult>> {
    let (s, _) = open(path)?;
    match s.latest_blob("results").map_err(|x| e("PROJECT_LOAD", x))? {
        Some((_, b)) => Ok(Some(serde_json::from_slice(&b).map_err(|x| e("RESULT_DECODE", x))?)),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModuleInput {
    pub id: String, pub manufacturer: String, pub model: String, pub technology: String,
    pub cells_in_series: u32, pub length_m: f64, pub width_m: f64,
    pub v_oc: f64, pub i_sc: f64, pub v_mp: f64, pub i_mp: f64, pub alpha_sc: f64, pub beta_voc: f64, pub gamma_pmp: f64,
}

/// Add (or replace) a user-authored module in the project library; returns the fitted record.
pub fn add_module(path: &Path, m: ModuleInput) -> Result<ModuleSpec> {
    let (mut s, mut p) = open(path)?;
    let mut spec = ModuleSpec {
        id: m.id, manufacturer: m.manufacturer, model: m.model, provenance: Provenance::User, technology: m.technology,
        cells_in_series: m.cells_in_series, length_m: m.length_m, width_m: m.width_m, v_oc: m.v_oc, i_sc: m.i_sc, v_mp: m.v_mp, i_mp: m.i_mp,
        alpha_sc: m.alpha_sc, beta_voc: m.beta_voc, gamma_pmp: m.gamma_pmp, model_params: None,
    };
    spec.fit().map_err(|x| e("MODULE_FIT", x))?;
    p.library.modules.retain(|x| x.id != spec.id);
    p.library.modules.push(spec.clone());
    solarcraft_model::derive::derive(&mut p);
    save(&mut s, &p, &format!("add module {}", spec.id))?;
    Ok(spec)
}

pub fn undo(path: &Path) -> Result<Option<Project>> {
    let mut s = Store::open(path).map_err(|x| e("PROJECT_OPEN", x))?;
    s.undo().map_err(|x| e("PROJECT_SAVE", x))
}

pub fn history(path: &Path, limit: usize) -> Result<Vec<(i64, String, i64)>> {
    let s = Store::open(path).map_err(|x| e("PROJECT_OPEN", x))?;
    Ok(s.history(limit).map_err(|x| e("PROJECT_LOAD", x))?.into_iter().map(|c| (c.seq, c.op, c.at_unix)).collect())
}

pub fn export_copy(path: &Path, dest: &Path) -> Result<()> {
    let s = Store::open(path).map_err(|x| e("PROJECT_OPEN", x))?;
    s.export_copy(dest).map_err(|x| e("EXPORT", x))
}
pub use solarcraft_engine::SimResult as SimResultExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat { Pdf, Docx }

#[derive(Debug, Clone, Serialize)]
pub struct ReportOutput { pub path: String, pub bytes: usize, pub format: ReportFormat, pub lang: String }

fn today_utc() -> (i32, u8, u8) {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64 / 86400).unwrap_or(0);
    let (y, m, d) = solarcraft_pv::civil_from_days(days);
    (y as i32, m as u8, d as u8)
}

/// Generate the customer report from the latest stored results.
/// `lang`: en, es, de or pt-BR.
pub fn generate_report(path: &Path, out: &Path, format: ReportFormat, lang: &str, designer: &str) -> Result<ReportOutput> {
    let l = solarcraft_report::i18n::Lang::parse(lang).ok_or_else(|| e("INVALID_ARGUMENT", format!("unsupported report language '{lang}' (use en, es, de, pt-BR)")))?;
    let (s, p) = open(path)?;
    let r = latest_results(path)?.ok_or_else(|| e("NO_RESULTS", "No results yet. Run the simulation first."))?;
    let (w, _) = load_weather(&s, &p)?;
    let input = solarcraft_report::ReportInput {
        project: &p, result: &r, lang: l, date: today_utc(), designer: designer.to_string(),
        weather: solarcraft_report::WeatherInfo { provider: w.provenance.provider.clone(), dataset: w.provenance.dataset.clone(), licence: w.provenance.licence.clone(), horizon_applied: w.horizon_applied },
    };
    let view = solarcraft_report::build_view(&input);
    let bytes = match format {
        ReportFormat::Pdf => solarcraft_report::pdf::render(&view, input.date),
        ReportFormat::Docx => solarcraft_report::docx::render(&view),
    }.map_err(|x| e("REPORT", x))?;
    std::fs::write(out, &bytes).map_err(|x| e("FILE_WRITE", format!("{}: {x}", out.display())))?;
    Ok(ReportOutput { path: out.display().to_string(), bytes: bytes.len(), format, lang: l.tag().into() })
}
