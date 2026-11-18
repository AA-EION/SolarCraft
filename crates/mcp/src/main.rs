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

#[derive(Deserialize, serde::Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Section { Grid, Loads, Storage, Tariff, Economics, Electrical, Losses, Simulation, Customer, Site }

#[derive(Deserialize, JsonSchema)]
struct SectionArgs {
    file: String,
    /// Section to replace as a whole. Shapes: see `get_project` (e.g. storage: {battery_id, units, coupling: {kind: dc_hybrid, unit_id} | {kind: ac_coupled, battery_inverter_id}, model: simple|detailed, soc_min_pct, soc_max_pct, initial_soc_pct, backup_reserve_pct, grid_services_pct, charge_priority: [backup, self_consumption, grid_services], grid_charge_backup, grid_service: {kind: none|export_window|peak_shaving, ...}, outages: [{start_hour, hours}]}; storage may be null).
    section: Section,
    value: Value,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Kind { Module, Inverter, Battery, BatteryInverter }
impl Kind { fn as_str(&self) -> &'static str { match self { Kind::Module => "module", Kind::Inverter => "inverter", Kind::Battery => "battery", Kind::BatteryInverter => "battery_inverter" } } }

#[derive(Deserialize, JsonSchema)]
struct EquipmentArgs {
    file: String,
    kind: Kind,
    /// The record in SolarCraft's equipment schema (same for user, imported and catalog items). Modules are fitted on save.
    record: Value,
}

#[derive(Deserialize, JsonSchema)]
struct StringingArgs {
    file: String,
    /// false: return to stringing derived from the arrays.
    manual: bool,
    /// [{id, name, inverter_id, mppts: [[{array_id, modules_per_string, strings}], ...]}]
    #[serde(default)]
    inverters: Value,
}

#[derive(Deserialize, JsonSchema)]
struct HoursArgs {
    file: String,
    /// Local day of the reference year, 1–365 (returns its 24 hours).
    day_of_year: Option<u32>,
    /// Channel to rank hours by: import, export, clipped, curtailed, batt_discharge, batt_charge, unserved, load, pv_ac, or -soc (lowest SOC first).
    top: Option<String>,
    start: Option<u32>,
    count: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct VariantArgs { file: String, name: String }

#[derive(Deserialize, JsonSchema)]
struct VariantIdArgs { file: String, id: String, #[serde(default)] keep_current: bool }

#[derive(Deserialize, JsonSchema)]
struct LibraryArgs {
    /// Filter by kind.
    kind: Option<Kind>,
    /// Text search across the record.
    query: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct LibrarySaveArgs { kind: Kind, record: Value }

#[derive(Deserialize, JsonSchema)]
struct PinArgs { file: String, kind: Kind, id: String }

#[derive(Deserialize, JsonSchema)]
struct LoadArgs {
    file: String,
    /// CSV/text file in the workspace: one value per line (8760 hourly … 525600 minute values).
    source: String,
    /// "kwh" (energy per interval, default) or "kw" (average power).
    unit: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct CatalogSearchArgs {
    /// module, pvinverter, batterysystem, batteryinverter, generator, feedintariff, netmeteringtariff, demandtariff, company, country, optimizer, ev, battery.
    family: Option<String>,
    /// Words matched in name, company (name, short name) and country (all locales, ISO codes).
    #[serde(default)]
    query: String,
    #[serde(default)]
    available_only: bool,
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct CatalogItemArgs { family: String, id: String, /// Revision; latest when omitted.
    version: Option<i64> }

#[derive(Deserialize, JsonSchema)]
struct CatalogPinArgs { file: String, family: String, id: String, version: Option<i64> }

#[derive(Deserialize, JsonSchema)]
struct CatalogTariffArgs { file: String, /// {family: demandtariff|netmeteringtariff, id, version?}
    import: Option<Value>, /// {family: feedintariff, id, version?}
    export: Option<Value> }

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
struct RoofArgs {
    file: String,
    /// Array index (default 0).
    #[serde(default)]
    array: u32,
    /// Roof outline in the roof plane, metres: x along the eave, y up the slope. Null removes the roof.
    outline: Option<Vec<[f64; 2]>>,
    /// Obstacles (chimneys, skylights) as polygons in the same coordinates.
    #[serde(default)]
    obstacles: Vec<Vec<[f64; 2]>>,
    /// Height of each obstacle above the roof, metres (default 1 m); used for near shading.
    #[serde(default)]
    obstacle_heights: Vec<f64>,
    /// Optional layout parameters: setback, obstacle_clearance, gap_x, gap_y (m), orientation (portrait|landscape|best).
    #[serde(default)]
    params: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize, JsonSchema)]
struct ReportArgs {
    file: String,
    /// Output file in the workspace, ending in .pdf or .docx (must not exist unless overwrite=true).
    output: String,
    /// "pdf" or "docx".
    format: app::ReportFormat,
    /// Customer short, customer extended (default), or system planner.
    #[serde(default)]
    report_type: app::ReportType,
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

#[derive(Deserialize, JsonSchema)]
struct SceneEditArgs {
    file: String,
    /// One transaction (all or nothing; one undo step). Each edit is an object with "op":
    /// add_object {name, layer?, id?, kind: {type: building|block|cylinder|tree|terrain|mesh, ...}, properties?},
    /// update_object {id, name?, layer?, kind?, hidden?, properties?}, move {id, dx, dy, dz?}, remove_object {id},
    /// set_georef {georef: {latitude, longitude, elevation, true_north_deg}}, set_layer {name, visible?, locked?},
    /// add_field {name, array_id?, mount: {type: roof, face: "bld-1/roof-0"} | {type: ground, origin, azimuth_deg, tilt_deg, rows, tables_per_row, modules_across, modules_up, row_pitch, terrain?}, module?, layout?},
    /// update_field, remove_field {id}, place_modules {field}, remove_modules {field, modules}, route_strings {field, modules_per_string, inverter_at?}.
    /// Building kinds: {type: building, footprint: [[x,y],...] metres (x east, y north), eave_height, base_z?, roof: {type: flat | shed{pitch_deg, facing_deg} | gable{pitch_deg} | hip{pitch_deg}}, openings?: [[[x,y],...]]}.
    /// Tree: {type: tree, center, trunk_height, trunk_radius, crown_radius, crown_height, transparency (0–1)}.
    edits: Vec<Value>,
    /// Return the resulting scene and issues without saving.
    #[serde(default)]
    preview: bool,
}

#[derive(Deserialize, JsonSchema)]
struct SceneBindArgs {
    file: String,
    /// Module field id (fld-…).
    field: String,
    /// Electrical array id (array-1, …): takes the field's tilt/azimuth and routed strings.
    array_id: String,
    /// Fit the field to the array first: place modules with the array's module size, keep
    /// modules-per-string × strings of them and route the strings.
    #[serde(default)]
    match_array: bool,
}

#[derive(Deserialize, JsonSchema)]
struct SceneRenderArgs {
    file: String,
    /// Output file in the workspace ending in .png or .svg.
    output: String,
    /// material | string | shading (per-module beam loss of the last run).
    #[serde(default)]
    color_by: Option<String>,
    /// Orbit angles in degrees (default yaw 25 = looking north-north-east, pitch 35).
    #[serde(default)]
    yaw_deg: Option<f64>,
    #[serde(default)]
    pitch_deg: Option<f64>,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize, JsonSchema)]
struct SceneExportArgs {
    file: String,
    /// Output file in the workspace: .glb, .gltf, .obj, .dxf, .ifc, .igz (IngeTrazo) or .json.
    output: String,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize, JsonSchema)]
struct SceneImportArgs {
    file: String,
    /// Source file in the workspace (.glb, .gltf, .obj, .dxf, .ifc, .igz, .json).
    source: String,
    /// relink (bool): update objects imported earlier from the same file; geometry_only (bool, .igz);
    /// up ("y"|"z", OBJ); unit_scale (number); default_height (DXF footprints, m).
    #[serde(default)]
    options: Option<Value>,
    #[serde(default)]
    preview: bool,
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

    #[tool(description = "Set (or remove) the roof face of an array and place modules on it, respecting edge setback and obstacle clearance. In Easy mode the system size follows the modules that fit. Returns modules_fit and issues.")]
    async fn set_roof(&self, Parameters(a): Parameters<RoofArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let req = json!({"file": f.to_string_lossy(), "array": a.array, "outline": a.outline, "obstacles": a.obstacles, "obstacle_heights": a.obstacle_heights, "params": a.params});
        match app::api::dispatch("set_roof", &req, &mut |_| true) { Ok(v) => ok(v), Err(e) => app_err(e) }
    }

    #[tool(description = "Attach offline clear-sky demonstration weather (no network). Overestimates yield; for trying the workflow only.")]
    async fn use_demo_weather(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        match app::use_demo_weather(&f) { Ok(s) => ok(to_value(&s)), Err(e) => app_err(e) }
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
            Ok(Ok(r)) => ok(app::api::summary(&r)),
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
        match tokio::task::spawn_blocking(move || app::generate_report_for(&f, &out, a.format, &a.lang, &a.designer, a.report_type)).await {
            Ok(Ok(r)) => ok(to_value(&r)), Ok(Err(e)) => app_err(e), Err(e) => Err(ErrorData::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Replace a plant section (grid, loads, storage, tariff, economics, electrical, losses, simulation, customer, site) in one validated change. Returns the stored section and all validation issues. Undo with `undo`.")]
    async fn set_section(&self, Parameters(a): Parameters<SectionArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let sec = serde_json::to_value(&a.section).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        self.api("set_section", json!({"file": f, "section": sec, "value": a.value}))
    }

    #[tool(description = "Add or replace equipment in the project library (works offline; same schema as library and catalog items). kind: module | inverter | battery | battery_inverter.")]
    async fn add_equipment(&self, Parameters(a): Parameters<EquipmentArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("add_equipment", json!({"file": f, "kind": a.kind.as_str(), "record": a.record}))
    }

    #[tool(description = "Show the inverter/MPPT stringing used by the simulation (manual or derived) with its electrical checks (string voltage at −10/70 °C, MPPT current, DC/AC ratio, roof capacity).")]
    async fn get_stringing(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("stringing", json!({"file": f}))
    }

    #[tool(description = "Assign strings to inverter MPPT inputs (manual=true with inverters), or return to derived stringing (manual=false). Returns the checks; errors block simulation.")]
    async fn set_stringing(&self, Parameters(a): Parameters<StringingArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("set_stringing", json!({"file": f, "manual": a.manual, "inverters": a.inverters}))
    }

    #[tool(description = "Inspect the hours behind the totals: one local day, the top hours of a channel, or a range. Rows carry PV, consumption, battery charge/discharge, SOC, reserve allocations, import, export, curtailment and the balance residual (Wh).")]
    async fn results_hours(&self, Parameters(a): Parameters<HoursArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("results_hours", json!({"file": f, "day_of_year": a.day_of_year, "top": a.top, "start": a.start, "count": a.count}))
    }

    #[tool(description = "Year-1 bills with and without the plant and the cash flow (savings, export revenue, O&M, replacements, loan, cumulative) with NPV, IRR, payback and LCOE from the latest run.")]
    async fn cash_flow(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("cash_flow", json!({"file": f}))
    }

    #[tool(description = "Save the current design as a named variant.")]
    async fn create_variant(&self, Parameters(a): Parameters<VariantArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("create_variant", json!({"file": f, "name": a.name}))
    }

    #[tool(description = "Make a saved variant the active design (keep_current saves the current design first).")]
    async fn apply_variant(&self, Parameters(a): Parameters<VariantIdArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("apply_variant", json!({"file": f, "id": a.id, "keep_current": a.keep_current}))
    }

    #[tool(description = "Simulate the current design and every variant with their own pinned inputs and compare yield, self-consumption, autarky, grid exchange, NPV, IRR, payback and LCOE.")]
    async fn compare_variants(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let ws = self.workspace.clone();
        match tokio::task::spawn_blocking(move || { let _ = ws; app::api::dispatch("compare_variants", &json!({"file": f}), &mut |_| true) }).await {
            Ok(Ok(v)) => ok(v), Ok(Err(e)) => app_err(e), Err(e) => Err(ErrorData::internal_error(e.to_string(), None)),
        }
    }

    #[tool(description = "Search the workspace equipment library (library.sqlite in the workspace; offline, versioned).")]
    async fn library_list(&self, Parameters(a): Parameters<LibraryArgs>) -> Result<CallToolResult, ErrorData> {
        self.api("library_list", json!({"library": self.library(), "kind": a.kind.map(|k| k.as_str()), "query": a.query}))
    }

    #[tool(description = "Save an equipment record to the workspace library as a new revision (validated; modules are fitted).")]
    async fn library_save(&self, Parameters(a): Parameters<LibrarySaveArgs>) -> Result<CallToolResult, ErrorData> {
        self.api("library_save", json!({"library": self.library(), "kind": a.kind.as_str(), "record": a.record}))
    }

    #[tool(description = "Copy a library record into a project (the project keeps its own copy and never depends on the library).")]
    async fn library_pin(&self, Parameters(a): Parameters<PinArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("library_pin", json!({"library": self.library(), "file": f, "kind": a.kind.as_str(), "id": a.id}))
    }

    #[tool(description = "Import a metered consumption series from a workspace file into the project.")]
    async fn import_load(&self, Parameters(a): Parameters<LoadArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let src = self.path(&a.source, false, true)?;
        self.api("import_load", json!({"file": f, "source": src, "unit": a.unit.unwrap_or_else(|| "kwh".into())}))
    }

    #[tool(description = "Search the imported equipment and tariff catalog (workspace catalog.sqlite). Returns the latest revision per entity with its revision count, company and country.")]
    async fn catalog_search(&self, Parameters(a): Parameters<CatalogSearchArgs>) -> Result<CallToolResult, ErrorData> {
        self.api("catalog_search", json!({"catalog": self.catalog(), "family": a.family, "query": a.query, "available_only": a.available_only, "limit": a.limit.unwrap_or(25)}))
    }

    #[tool(description = "Catalog entry details: normalised SolarCraft record with units, original record, issues, join checks, all revisions and the changelog history.")]
    async fn catalog_details(&self, Parameters(a): Parameters<CatalogItemArgs>) -> Result<CallToolResult, ErrorData> {
        self.api("catalog_details", json!({"catalog": self.catalog(), "family": a.family, "id": a.id, "version": a.version}))
    }

    #[tool(description = "Pin a catalog revision (latest when version is omitted) into the project library; the project keeps the record and its catalog reference.")]
    async fn catalog_pin(&self, Parameters(a): Parameters<CatalogPinArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("catalog_pin", json!({"catalog": self.catalog(), "file": f, "family": a.family, "id": a.id, "version": a.version}))
    }

    #[tool(description = "Apply catalog tariffs to the project billing: import side (demand or net-metering tariff with its decoded time-of-use schedule) and/or export side (feed-in tariff). Reports applied and not-applied rules.")]
    async fn catalog_apply_tariff(&self, Parameters(a): Parameters<CatalogTariffArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("catalog_apply_tariff", json!({"catalog": self.catalog(), "file": f, "import": a.import, "export": a.export}))
    }

    #[tool(description = "Get the 3D scene: objects (buildings, blocks, cylinders, trees, terrain, imported meshes), roof faces with tilt/azimuth/area, module fields with modules and strings, counts and geometry issues. Coordinates: metres, x east, y north, z up.")]
    async fn scene_get(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("scene_get", json!({"file": f}))
    }

    #[tool(description = "Edit the 3D scene in one transaction (all edits or none; one undo step). Stable ids: bld-N, blk-N, cyl-N, tree-N, ter-N, mesh-N, fld-N; faces bld-N/roof-K; modules fld-N/mNNNN; strings fld-N/sK. Use preview=true to check first. Returns the scene, touched ids and issues.")]
    async fn scene_edit(&self, Parameters(a): Parameters<SceneEditArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("scene_edit", json!({"file": f, "edits": a.edits, "preview": a.preview}))
    }

    #[tool(description = "Bind a module field to an electrical array: the field takes the array module's size (if not yet placed); the array takes the field's tilt, azimuth and, once routed, its modules per string and string count. The simulation then shades that array per module from the scene.")]
    async fn scene_bind(&self, Parameters(a): Parameters<SceneBindArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("scene_bind", json!({"file": f, "field": a.field, "array_id": a.array_id, "match_array": a.match_array}))
    }

    #[tool(description = "Per-module shading of the latest run: beam loss with bypass diodes and geometric, sky view factor, string, per field.")]
    async fn scene_shading(&self, Parameters(a): Parameters<FileArg>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        self.api("scene_shading", json!({"file": f}))
    }

    #[tool(description = "Render the 3D scene to a PNG (z-buffered) or SVG file in the workspace, coloured by material, string or shading loss. Side effect: creates a file.")]
    async fn scene_render(&self, Parameters(a): Parameters<SceneRenderArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let out = self.path(&a.output, false, false)?;
        if out.exists() && !a.overwrite { return Err(ErrorData::invalid_params("output exists; pass overwrite=true".to_string(), None)); }
        let cam = json!({"yaw_deg": a.yaw_deg.unwrap_or(25.0), "pitch_deg": a.pitch_deg.unwrap_or(35.0), "width": 1200, "height": 800});
        let params = json!({"file": f, "camera": cam, "color_by": a.color_by.unwrap_or_else(|| "material".into())});
        let png = out.extension().and_then(|e| e.to_str()) == Some("png");
        if !png && out.extension().and_then(|e| e.to_str()) != Some("svg") { return Err(ErrorData::invalid_params("output must end in .png or .svg".to_string(), None)); }
        let r = if png { app::scene_ops::image(&f, &params).and_then(|v| {
            use base64::Engine;
            let b = base64::engine::general_purpose::STANDARD.decode(v["png_base64"].as_str().unwrap_or("")).unwrap_or_default();
            std::fs::write(&out, &b).map(|_| json!({"file": a.output, "bytes": b.len()})).map_err(|e| app::AppError { code: "EXPORT", message: e.to_string() })
        }) } else { app::scene_ops::svg(&f, &params).and_then(|s| std::fs::write(&out, &s).map(|_| json!({"file": a.output, "bytes": s.len()})).map_err(|e| app::AppError { code: "EXPORT", message: e.to_string() })) };
        match r { Ok(v) => ok(v), Err(e) => app_err(e) }
    }

    #[tool(description = "Export the 3D scene to CAD/BIM/mesh: glTF/GLB, OBJ, DXF (3D faces + hidden-line roof plan + strings), IFC4 (IfcRoof/IfcWall/IfcSolarDevice with Pset_SolarCraft), IngeTrazo .igz, or scene JSON. Returns a fidelity report (units, axes, georeference, coordinate error, what the format cannot carry). Side effect: creates a file.")]
    async fn scene_export(&self, Parameters(a): Parameters<SceneExportArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let out = self.path(&a.output, false, false)?;
        if out.exists() && !a.overwrite { return Err(ErrorData::invalid_params("output exists; pass overwrite=true".to_string(), None)); }
        self.api("scene_export", json!({"file": f, "output": out}))
    }

    #[tool(description = "Import a CAD/BIM/mesh file from the workspace into the 3D scene as one undoable transaction (preview=true to inspect first). SolarCraft exports come back exactly; other files become mesh objects that shade and carry their source identity. relink=true updates objects previously imported from the same file and keeps SolarCraft edits. SketchUp, Revit and DWG are reported with the export path to use instead.")]
    async fn scene_import(&self, Parameters(a): Parameters<SceneImportArgs>) -> Result<CallToolResult, ErrorData> {
        let f = self.path(&a.file, true, true)?;
        let src = self.path(&a.source, false, true)?;
        self.api("scene_import", json!({"file": f, "source": src, "options": a.options.unwrap_or(json!({})), "preview": a.preview}))
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

impl SolarCraftMcp {
    fn library(&self) -> PathBuf { self.workspace.join("library.sqlite") }
    fn catalog(&self) -> PathBuf { self.workspace.join("catalog.sqlite") }
    fn api(&self, method: &str, params: Value) -> Result<CallToolResult, ErrorData> {
        match app::api::dispatch(method, &params, &mut |_| true) { Ok(v) => ok(v), Err(e) => app_err(e) }
    }
}

#[tool_handler]
impl ServerHandler for SolarCraftMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions("SolarCraft PV design tools. Typical flow: create_project → fetch_weather_pvgis (confirm_network) or import_weather → set_section loads/grid/tariff → add_equipment or library_pin → set_roof → set_stringing → set_section storage → validate → simulate → explain / results_hours / cash_flow → create_variant / compare_variants → export_report. Fields are set by JSON pointer with set_fields; every saved change can be undone. Units: SI, angles in degrees, azimuth clockwise from north (180 = south).")
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
