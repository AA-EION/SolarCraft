// SPDX-License-Identifier: GPL-3.0-or-later
//! SolarCraft MCP server (stdio). Usage: `solarcraft-mcp --workspace <dir>`.
//!
//! Safety model (brief, "MCP and automation"):
//! - every path is resolved inside the workspace; anything else is refused;
//! - only `.solarcraft` project files are created or modified;
//! - no command execution; the only network call (PVGIS) needs
//!   `confirm_network: true` and sends only site coordinates;
//! - edits support `dry_run` previews, and every saved change can be undone.
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use solarcraft_app as app;
use std::path::{Path, PathBuf};

#[derive(Clone)]
struct SolarCraftMcp {
    #[expect(dead_code, reason = "read by the tool_handler macro")]
    tool_router: ToolRouter<Self>,
    workspace: PathBuf,
}

#[derive(Deserialize, JsonSchema)]
struct FileArg {
    /// Project file name relative to the workspace, ending in `.solarcraft`.
    file: String,
}

#[derive(Deserialize, JsonSchema)]
struct CreateArgs {
    /// New project file name relative to the workspace, ending in `.solarcraft`.
    file: String,
    name: String,
    /// Degrees north, −90..90.
    latitude: f64,
    /// Degrees east, −180..180.
    longitude: f64,
    /// Optional DC size target in kWp (Easy mode). Default 10.
    target_kwp: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
struct Edit {
    /// JSON pointer into the project, e.g. `/arrays/0/tilt`, `/site/albedo`, `/losses/soiling_pct`.
    pointer: String,
    /// New value (number, string, or enum string such as "open_rack").
    value: Value,
}

#[derive(Deserialize, JsonSchema)]
struct SetArgs {
    file: String,
    edits: Vec<Edit>,
    /// If true, return the resulting project and validation without saving.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Deserialize, JsonSchema)]
struct ModeArgs {
    file: String,
    /// "easy" or "detailed". Switching never deletes data.
    mode: String,
}

#[derive(Deserialize, JsonSchema)]
struct FetchArgs {
    file: String,
    /// Must be true: sends the site latitude/longitude to the PVGIS service (European Commission JRC).
    confirm_network: bool,
}

#[derive(Deserialize, JsonSchema)]
struct ImportArgs {
    file: String,
    /// EPW or saved PVGIS TMY JSON file, relative to the workspace.
    source: String,
}

#[derive(Deserialize, JsonSchema)]
struct ModuleArgs {
    file: String,
    module: app::ModuleInput,
}

#[derive(Deserialize, JsonSchema)]
struct ReportArgs {
    file: String,
    /// Output file in the workspace, ending in .pdf or .docx (must not exist unless overwrite=true).
    output: String,
    /// "pdf" or "docx".
    format: app::ReportFormat,
    /// Report language: en, es, de or pt-BR.
    #[serde(default = "default_lang")]
    lang: String,
    /// Designer name printed on the cover.
    #[serde(default)]
    designer: String,
    #[serde(default)]
    overwrite: bool,
}
fn default_lang() -> String { "en".into() }

#[derive(Deserialize, JsonSchema)]
struct ResultsArgs {
    file: String,
    /// Include hourly series (paged).
    #[serde(default)]
    include_hourly: bool,
    /// Hourly page (0-based, 744 rows per page).
    #[serde(default)]
    page: usize,
}

fn ok(v: Value) -> Result<CallToolResult, ErrorData> { Ok(CallToolResult::structured(v)) }
fn app_err(e: app::AppError) -> Result<CallToolResult, ErrorData> { Ok(CallToolResult::structured_error(json!({"code": e.code, "message": e.message}))) }
fn to_value<T: serde::Serialize>(v: &T) -> Value { serde_json::to_value(v).unwrap_or(Value::Null) }

impl SolarCraftMcp {
    /// Resolve `rel` inside the workspace. `must_exist=false` allows a new file.
    fn path(&self, rel: &str, project: bool, must_exist: bool) -> Result<PathBuf, ErrorData> {
        let bad = |m: &str| ErrorData::invalid_params(m.to_string(), Some(json!({"path": rel})));
        let p = Path::new(rel);
        if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return Err(bad("paths must be relative to the workspace, without '..'")); }
        if project && p.extension().and_then(|e| e.to_str()) != Some("solarcraft") { return Err(bad("project files must end in .solarcraft")); }
        let full = self.workspace.join(p);
        let check = if must_exist { full.canonicalize().map_err(|_| bad("file not found in workspace"))? } else {
            let parent = full.parent().unwrap_or(&self.workspace).canonicalize().map_err(|_| bad("directory not found in workspace"))?;
            parent.join(full.file_name().ok_or_else(|| bad("missing file name"))?)
        };
        if !check.starts_with(&self.workspace) { return Err(bad("path escapes the workspace")); }
        Ok(check)
    }
}

#[tool_router]
impl SolarCraftMcp {
    fn new(workspace: PathBuf) -> Self { Self { tool_router: Self::tool_router(), workspace } }

    #[tool(description = "List SolarCraft project files (.solarcraft) in the workspace.")]
    async fn list_projects(&self) -> Result<CallToolResult, ErrorData> {
        let mut v = vec![];
        if let Ok(rd) = std::fs::read_dir(&self.workspace) {
            for e in rd.flatten() { let n = e.file_name().to_string_lossy().to_string(); if n.ends_with(".solarcraft") { v.push(n); } }
        }
        v.sort();
        ok(json!({"projects": v}))
    }

    #[tool(description = "Create a new project in Easy mode. Derives tilt, inverter and string layout from the site and kWp target. Side effect: creates a file.")]
    async fn create_project(&self, Parameters(a): Parameters<CreateArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, false)?;
        match app::create_project(&f, &a.name, a.latitude, a.longitude, a.target_kwp) { Ok(p) => ok(to_value(&p)), Err(e) => app_err(e) }
    }

    #[tool(description = "Get the full project document. Every field is {value, origin}; origin is user, derived or default.")]
    async fn get_project(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::get_project(&f) { Ok(p) => ok(to_value(&p)), Err(e) => app_err(e) }
    }

    #[tool(description = "Set fields by JSON pointer in one transaction (marks them user-owned and re-derives the rest). Use dry_run to preview. Undo with `undo`.")]
    async fn set_fields(&self, Parameters(a): Parameters<SetArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let edits: Vec<(String, Value)> = a.edits.into_iter().map(|e| (e.pointer, e.value)).collect();
        if a.dry_run {
            let mut p = match app::get_project(&f) { Ok(p) => p, Err(e) => return app_err(e) };
            for (k, v) in &edits { if let Err(m) = p.set_by_pointer(k, v.clone()) { return app_err(app::AppError { code: "INVALID_VALUE", message: m }); } }
            return ok(json!({"dry_run": true, "project": p, "issues": solarcraft_model::validate::validate(&p)}));
        }
        match app::set_fields(&f, &edits) { Ok(p) => ok(json!({"project": p, "issues": solarcraft_model::validate::validate(&p)})), Err(e) => app_err(e) }
    }

    #[tool(description = "Switch the design mode between \"easy\" and \"detailed\". Never deletes or changes values.")]
    async fn set_mode(&self, Parameters(a): Parameters<ModeArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let m = match a.mode.as_str() { "easy" => solarcraft_model::DesignMode::Easy, "detailed" => solarcraft_model::DesignMode::Detailed, _ => return Err(ErrorData::invalid_params("mode must be easy or detailed", None)) };
        match app::set_mode(&f, m) { Ok(p) => ok(json!({"mode": p.mode})), Err(e) => app_err(e) }
    }

    #[tool(description = "Validate the design. Returns issues with stable codes, severity (error/warning/info) and the JSON pointer of the field concerned.")]
    async fn validate(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::validate_project(&f) { Ok(i) => ok(json!({"issues": i})), Err(e) => app_err(e) }
    }

    #[tool(description = "Fetch a typical meteorological year from PVGIS for the project site and store it in the project. Requires confirm_network=true (sends latitude/longitude to the EC JRC PVGIS service).")]
    async fn fetch_weather_pvgis(&self, Parameters(a): Parameters<FetchArgs>) -> Result<CallToolResult, ErrorData> {
        if !a.confirm_network { return Err(ErrorData::invalid_params("confirm_network must be true: this sends the site coordinates to PVGIS", None)); }
        let f = self.path(&a.file, true, true)?;
        match tokio::task::spawn_blocking(move || app::fetch_weather_pvgis(&f)).await { Ok(Ok(s)) => ok(to_value(&s)), Ok(Err(e)) => app_err(e), Err(e) => Err(ErrorData::internal_error(e.to_string(), None)) }
    }

    #[tool(description = "Import weather from an EPW or saved PVGIS TMY JSON file in the workspace. Returns provenance and quality checks.")]
    async fn import_weather(&self, Parameters(a): Parameters<ImportArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let src = self.path(&a.source, false, true)?;
        match app::import_weather(&f, &src) { Ok(s) => ok(to_value(&s)), Err(e) => app_err(e) }
    }

    #[tool(description = "Add or replace a user-authored PV module from datasheet values (STC Voc, Isc, Vmp, Imp in V/A; alpha_sc A/K; beta_voc V/K; gamma_pmp %/K). Fits the CEC six-parameter model and reports fit quality.")]
    async fn add_module(&self, Parameters(a): Parameters<ModuleArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::add_module(&f, a.module) { Ok(m) => ok(to_value(&m)), Err(e) => app_err(e) }
    }

    #[tool(description = "Run the annual simulation and store results. Returns annual totals, run manifest and warnings.")]
    async fn simulate(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match tokio::task::spawn_blocking(move || app::simulate(&f, |_| true)).await {
            Ok(Ok(r)) => ok(json!({"annual": r.annual, "manifest": r.manifest, "warnings": r.warnings})),
            Ok(Err(e)) => app_err(e), Err(e) => Err(ErrorData::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Explain the latest results: irradiation and energy loss waterfalls (each step with absolute and % change) and the monthly table.")]
    async fn explain(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::latest_results(&f) {
            Ok(Some(r)) => ok(json!({"irradiance_kwh_m2": r.irradiance_ledger, "energy_kwh": r.energy_ledger, "monthly": r.monthly, "annual": r.annual, "manifest": r.manifest})),
            Ok(None) => app_err(app::AppError { code: "NO_RESULTS", message: "No results yet; call simulate first.".into() }),
            Err(e) => app_err(e),
        }
    }

    #[tool(description = "Get stored results. Hourly series (UTC interval midpoints, W and W/m²) are paged 744 rows per page.")]
    async fn get_results(&self, Parameters(a): Parameters<ResultsArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::latest_results(&f) {
            Ok(Some(r)) => {
                let mut v = json!({"annual": r.annual, "monthly": r.monthly, "manifest": r.manifest});
                if a.include_hourly {
                    let n = r.hourly.unix_mid.len();
                    let (s, e) = ((a.page * 744).min(n), ((a.page + 1) * 744).min(n));
                    v["hourly"] = json!({"page": a.page, "pages": n.div_ceil(744), "unix_mid": &r.hourly.unix_mid[s..e], "poa_w_m2": &r.hourly.poa_w_m2[s..e], "t_cell_c": &r.hourly.t_cell_c[s..e], "dc_w": &r.hourly.dc_w[s..e], "ac_w": &r.hourly.ac_w[s..e]});
                }
                ok(v)
            }
            Ok(None) => app_err(app::AppError { code: "NO_RESULTS", message: "No results yet; call simulate first.".into() }),
            Err(e) => app_err(e),
        }
    }

    #[tool(description = "Write the customer report (PDF, or editable DOCX) from the latest results into the workspace. Side effect: creates a file.")]
    async fn export_report(&self, Parameters(a): Parameters<ReportArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let ext = match a.format { app::ReportFormat::Pdf => "pdf", app::ReportFormat::Docx => "docx" };
        if !a.output.ends_with(&format!(".{ext}")) { return Err(ErrorData::invalid_params(format!("output must end in .{ext}"), None)); }
        let out = self.path(&a.output, false, false)?;
        if out.exists() && !a.overwrite { return Err(ErrorData::invalid_params("output exists; set overwrite=true to replace it", None)); }
        match tokio::task::spawn_blocking(move || app::generate_report(&f, &out, a.format, &a.lang, &a.designer)).await {
            Ok(Ok(r)) => ok(to_value(&r)), Ok(Err(e)) => app_err(e), Err(e) => Err(ErrorData::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Undo the last saved change to the project.")]
    async fn undo(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::undo(&f) { Ok(p) => ok(json!({"undone": p.is_some()})), Err(e) => app_err(e) }
    }

    #[tool(description = "List recent changes (newest first).")]
    async fn history(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::history(&f, 50) { Ok(h) => ok(json!({"changes": h.into_iter().map(|(s, op, at)| json!({"seq": s, "op": op, "at_unix": at})).collect::<Vec<_>>()})), Err(e) => app_err(e) }
    }
}

#[tool_handler]
impl ServerHandler for SolarCraftMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions("SolarCraft PV design tools. Typical flow: create_project → fetch_weather_pvgis (confirm_network) or import_weather → validate → simulate → explain. Fields are set by JSON pointer with set_fields; every saved change can be undone. Units: SI, angles in degrees, azimuth clockwise from north (180 = south).")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let ws = args.iter().position(|a| a == "--workspace").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or(std::env::current_dir()?);
    let ws = ws.canonicalize().map_err(|e| format!("workspace {}: {e}", ws.display()))?;
    eprintln!("solarcraft-mcp: workspace {}", ws.display());
    let service = SolarCraftMcp::new(ws).serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
