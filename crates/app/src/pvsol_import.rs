// SPDX-License-Identifier: GPL-3.0-or-later
//! Offline import and comparison for the observed ZIP-based PV*SOL project formats.
mod archived_photo;
mod archived_routes;
mod archived_scene;
mod archived_shade;
mod island;

use crate::{e, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::read::GzDecoder;
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use solarcraft_model::{derive, plant::{BuckOptimizer, CatalogRef, Electrical, FeedIn, InverterUnit, LoadProfile, StringGroup},
    tracking::Tracker, Array, DesignMode, Field, Project, TranspositionModel};
use solarcraft_store::{sha256_hex, InitialBlob, Store};
use solarcraft_weather::{Hour, Provenance, Weather};
use std::{borrow::Cow, collections::{BTreeMap, HashMap}, fs::File, io::{Cursor, Read}, path::Path};
use zip::ZipArchive;

const HOURS: usize = 8760;
const WS_PER_KWH: f64 = 3_600_000.0;
const MAX_ARCHIVE: u64 = 128 * 1024 * 1024;
const REFERENCE_KIND: &str = "pvsol_reference";
const AC: &str = "inverter_ac";
const DC: &str = "pv_dc";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Channel {
    annual_kwh: Option<f64>,
    monthly_kwh: Option<Vec<f64>>,
    hourly_kwh: Option<Vec<f64>>,
    hourly_source: Option<String>,
}

impl Channel {
    fn annual(&self) -> Option<f64> {
        self.annual_kwh.or_else(|| self.hourly_kwh.as_ref().map(|hours| hours.iter().sum()))
    }

    fn monthly(&self) -> Option<Vec<f64>> {
        self.monthly_kwh.clone().or_else(|| {
            let hours = self.hourly_kwh.as_ref().filter(|hours| hours.len() == HOURS)?;
            let mut months = vec![0.0; 12];
            for (hour, value) in hours.iter().enumerate() {
                months[solarcraft_engine::calendar::local_time(hour as u32).month as usize - 1] += value;
            }
            Some(months)
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Reference {
    source_name: String,
    sha256: String,
    format: u32,
    system_type: String,
    weather_member: String,
    weather_diffuse: String,
    load_source: Option<String>,
    load_annual_kwh: Option<f64>,
    installed_modules: u32,
    inverters: usize,
    #[serde(default)]
    tracking_arrays: usize,
    #[serde(default)]
    archived_shade_arrays: usize,
    #[serde(default)]
    nonlinear_shade_arrays: usize,
    #[serde(default)]
    archived_scene_fields: usize,
    #[serde(default)]
    photoplan_arrays: usize,
    target_kwp: f64,
    initial_inputs_hash: String,
    warnings: Vec<String>,
    channels: BTreeMap<String, Channel>,
}

struct Imported {
    project: Project,
    weather: Vec<u8>,
    load: Option<Vec<u8>>,
    photo_blobs: Vec<archived_photo::PhotoBlob>,
    reference: Reference,
}

pub fn import(source: &Path, destination: Option<&Path>) -> Result<Value> {
    let imported = read(source)?;
    if let Some(path) = destination {
        if path.extension().and_then(|value| value.to_str()).is_none_or(|value| !value.eq_ignore_ascii_case("solarcraft")) {
            return Err(e("PVSOL_IMPORT", "Save the imported copy with a .solarcraft extension."));
        }
        let reference = encode(&imported.reference)?;
        let mut blobs = vec![
            InitialBlob { kind: "weather", media_type: "application/vnd.solarcraft.weather+json", data: &imported.weather },
            InitialBlob { kind: REFERENCE_KIND, media_type: "application/json", data: &reference },
        ];
        if let Some(load) = &imported.load {
            blobs.push(InitialBlob { kind: "load_series", media_type: "application/json", data: load });
        }
        blobs.extend(imported.photo_blobs.iter().map(|photo| InitialBlob {
            kind: photo.kind, media_type: photo.media_type, data: &photo.bytes,
        }));
        Store::create_with_blobs(path, &imported.project, "import PV*SOL project", &blobs)
            .map_err(|error| e("PROJECT_CREATE", error))?;
    }
    Ok(json!({"file": destination, "reference": overview(&imported.reference), "issues": crate::validate(&imported.project)}))
}

pub fn compare(path: &Path) -> Result<Value> {
    let (store, project) = crate::open(path)?;
    let Some((_, bytes)) = store.latest_blob(REFERENCE_KIND).map_err(|error| e("PROJECT_LOAD", error))? else {
        return Ok(json!({"reference":null,"comparison":null}));
    };
    let saved: Reference = serde_json::from_slice(&bytes).map_err(|error| e("PVSOL_REFERENCE", error))?;
    let Some(run) = crate::current_results(&store, &project)? else {
        return Ok(json!({"reference": overview(&saved), "comparison": null}));
    };
    let comparable = [(AC, &run.hourly.ac_w, run.monthly.iter().map(|m| m.ac_kwh).collect::<Vec<_>>()),
        (DC, &run.hourly.dc_w, run.monthly.iter().map(|m| m.dc_kwh).collect::<Vec<_>>())];
    let mut comparison = Map::new();
    for (name, hours, months) in comparable {
        let Some(channel) = saved.channels.get(name) else { continue; };
        let annual = months.iter().sum::<f64>();
        let reference_annual = channel.annual();
        let delta = reference_annual.map(|value| annual - value);
        let monthly = channel.monthly().map(|reference| months.iter().zip(reference).enumerate()
            .map(|(month, (actual, saved))| json!({"month":month + 1,"solarcraft_kwh":actual,"pvsol_kwh":saved,"delta_kwh":actual - saved}))
            .collect::<Vec<_>>());
        let hourly = channel.hourly_kwh.as_ref().filter(|rows| rows.len() == HOURS && hours.len() == HOURS).map(|rows| {
            let differences = hours.iter().zip(rows).map(|(&actual, &reference)| f64::from(actual) / 1000.0 - reference);
            let mae = differences.clone().map(f64::abs).sum::<f64>() / HOURS as f64;
            let rmse = (differences.map(|delta| delta * delta).sum::<f64>() / HOURS as f64).sqrt();
            json!({"rows":HOURS,"mae_kwh":mae,"rmse_kwh":rmse,"source":channel.hourly_source,
                "alignment":"ordered local standard-time hour from January 1"})
        });
        comparison.insert(name.into(), json!({"solarcraft_kwh": annual, "pvsol_kwh": reference_annual,
            "delta_kwh":delta, "delta_pct":delta.zip(reference_annual.filter(|v| v.abs() > 1e-9)).map(|(d, v)| d / v * 100.0),
            "monthly":monthly, "hourly":hourly}));
    }
    Ok(json!({"reference":overview(&saved),"input_hash":run.manifest.inputs_hash,
        "imported_input_hash_matches": saved.initial_inputs_hash == run.manifest.inputs_hash, "comparison":comparison}))
}

fn overview(reference: &Reference) -> Value {
    let channels: Map<_, _> = reference.channels.iter().map(|(key, channel)| (key.clone(),
        json!({"annual_kwh":channel.annual(),"monthly_kwh":channel.monthly(),
            "monthly_source":if channel.monthly_kwh.is_some() { Some("SimResults.xml") } else { channel.hourly_source.as_deref() },
            "hourly_rows":channel.hourly_kwh.as_ref().map_or(0, Vec::len), "hourly_source":channel.hourly_source}))).collect();
    json!({"source_name":reference.source_name,"sha256":reference.sha256,"format":reference.format,
        "system_type":reference.system_type,"weather_member":reference.weather_member,"weather_diffuse":reference.weather_diffuse,
        "load_source":reference.load_source,"load_annual_kwh":reference.load_annual_kwh,
        "installed_modules":reference.installed_modules,"inverters":reference.inverters,"target_kwp":reference.target_kwp,
        "tracking_arrays": reference.tracking_arrays, "archived_shade_arrays": reference.archived_shade_arrays,
        "photoplan_arrays": reference.photoplan_arrays, "nonlinear_shade_arrays": reference.nonlinear_shade_arrays,
        "archived_scene_fields": reference.archived_scene_fields,
        "warnings":reference.warnings,"channels":channels})
}

fn read(source: &Path) -> Result<Imported> {
    let bad = |message| e("PVSOL_IMPORT", message);
    let source_name = source.file_name().and_then(|v| v.to_str()).unwrap_or("PV*SOL project");
    let raw = read_limited(File::open(source).map_err(|error| e("PVSOL_IMPORT", error))?, MAX_ARCHIVE)?;
    let sha = sha256_hex(&raw);
    let mut archive = ZipArchive::new(Cursor::new(raw)).map_err(|error| e("PVSOL_IMPORT", error))?;
    if archive.len() > 4096 { return Err(bad("The archive has too many members.")); }
    let project_xml = member(&mut archive, "Project.xml", 24 * 1024 * 1024)?
        .ok_or_else(|| bad("Missing Project.xml."))?;
    let doc = document(&project_xml, "Project.xml")?;
    let root = doc.root_element();
    if !root.has_tag_name("PVData") { return Err(bad("Project.xml has no PVData root.")); }
    let format = number::<f64>(root, &["Versionsnummer"]).filter(|v| matches!(*v, 15.0 | 20.0 | 23.0 | 24.0))
        .map(|v| v as u32).ok_or_else(|| bad("Only observed project formats 15, 20, 23 and 24 are supported."))?;
    let system = value(root, &["AnlagenTyp", "SystemType"]).map(|v| v.strip_prefix("EnergySoftware.PVSOL.").unwrap_or(v))
        .filter(|v| v.starts_with("OnGrid") || matches!(*v, "OffGridPV" | "OffGridPVWithBackUp"))
        .ok_or_else(|| bad("This importer supports on-grid PV and the observed legacy island designs."))?;
    let island = system.starts_with("OffGrid");
    let db = member(&mut archive, "DB.json", 16 * 1024 * 1024)?.ok_or_else(|| bad("An embedded DB.json is required."))?;
    let db: Value = serde_json::from_slice(strip_bom(&db)).map_err(|error| e("PVSOL_IMPORT", error))?;
    let mut warnings = vec![
        "The imported copy replays the editable PV design. EV and heat pump controls, financial settings, and unsupported layout assets require separate migration.".into(),
        "SolarCraft uses one-hour intervals on a reference non-leap standard-time calendar and estimates irradiance at the midpoint. Module temperature and electrical models may differ from the archived run.".into(),
    ];
    if !island {
        warnings.push("Grid-connected storage settings require separate migration.".into());
    }
    let (mut saved, monthly_rear) = member(&mut archive, "SimResults.xml", 24 * 1024 * 1024)?
        .map(|bytes| read_saved_xml(&bytes)).transpose()?.unwrap_or_default();
    let (core, diffuse, rear) = match member(&mut archive, "core.results.3600", 80 * 1024 * 1024)? {
        Some(bytes) => read_core(&bytes)?,
        None => (BTreeMap::new(), None, None),
    };
    for (name, hours) in core {
        let channel = saved.entry(name).or_default();
        channel.hourly_kwh = Some(hours);
        channel.hourly_source = Some("core.results.3600".into());
    }
    let economy = member(&mut archive, "OecForeCast.json", 32 * 1024 * 1024)?
        .map(|bytes| serde_json::from_slice::<Value>(strip_bom(&bytes)).map_err(|error| e("PVSOL_IMPORT", error)))
        .transpose()?;
    let ledger = economy.as_ref().and_then(|v| v.pointer("/BilanzierungsModel/Data"));
    let hourly = |name: &str| ledger.and_then(|value| value.get(name)).and_then(|v| v.as_array())
        .filter(|rows| rows.len() == HOURS).and_then(|rows| rows.iter().map(|v| v.as_f64().filter(|x| x.is_finite())
            .map(|ws| ws / WS_PER_KWH)).collect::<Option<Vec<_>>>());
    if let Some(hours) = hourly("PvErzeugungHourly").filter(|rows| rows.iter().sum::<f64>() > 0.0) {
        let channel = saved.entry(AC.into()).or_default();
        if channel.hourly_kwh.is_none() {
            channel.hourly_kwh = Some(hours);
            channel.hourly_source = Some("OecForeCast.json / PvErzeugungHourly".into());
        }
    }
    for (name, channel) in &saved {
        if let Some((annual, hours)) = channel.annual_kwh.zip(channel.hourly_kwh.as_ref()) {
            let sum = hours.iter().sum::<f64>();
            if (annual - sum).abs() > 0.02_f64.max(annual.abs() * 1e-6) {
                warnings.push(format!("The saved annual {name} and sum of its hourly channel differ by {:.3} kWh.", sum - annual));
            }
        }
    }

    let name_from_project = value(root, &["Meteo", "MeteoFile"]).and_then(|s| s.rsplit(['\\', '/']).next());
    let names: Vec<_> = archive.file_names().collect::<std::result::Result<_, _>>()
        .map_err(|error| e("PVSOL_IMPORT", error))?;
    let candidates: Vec<_> = names.into_iter().filter(|s| !s.contains(['/', '\\']) && s.to_ascii_lowercase().ends_with(".wbv"))
        .map(|s| s.into_owned()).collect();
    let weather_member = if let Some(name) = name_from_project.and_then(|name| candidates.iter().find(|s| s.eq_ignore_ascii_case(name))) {
        name.clone()
    } else if let [only] = candidates.as_slice() {
        only.clone()
    } else { return Err(bad("Select a project with one identifiable embedded WBV weather file.")); };
    let wbv = member(&mut archive, &weather_member, 256 * 1024)?.ok_or_else(|| bad("The embedded WBV member is missing."))?;
    let spectral = number::<f64>(root, &["Meteo", "VerlusteDurchSpektrumsAbweichung"]).unwrap_or(0.0);
    let (weather, timezone, weather_diffuse) = read_weather(&wbv, &weather_member, diffuse.as_deref(), spectral)?;
    if diffuse.is_none() || weather_diffuse != "saved_hourly_diffuse" {
        warnings.push("WBV supplies global irradiance only. SolarCraft estimates diffuse and direct irradiance with Erbs.".into());
    } else {
        warnings.push("The diffuse input comes from the saved hourly series with the reference spectral factor removed. Direct normal irradiance is reconstructed using SolarCraft's midpoint solar position.".into());
    }
    if spectral != 0.0 {
        warnings.push(format!("PV*SOL's separate spectral correction ({:.3}%) is not mapped to the SolarCraft PV engine.", spectral * 100.0));
    }
    let title = value(root, &["ProjektdatenV1", "Projekttitel"]).filter(|v| !v.is_empty())
        .unwrap_or_else(|| source_name.trim_end_matches(".pvprj"));
    let mut project = Project::new(&format!("pvsol-{}", &sha[..20]), title, weather.latitude, weather.longitude);
    project.mode = DesignMode::Detailed;
    project.site.name = Field::user(weather_member.trim_end_matches(".wbv").to_owned());
    project.site.elevation = Field::user(weather.elevation);
    project.site.utc_offset_h = Some(Field::user(timezone));
    project.simulation.transposition = Field::user(match value(root, &["Meteo", "TranspositionModelType"]) {
        Some("HayDavies") => TranspositionModel::HayDavies, Some("Isotropic") => TranspositionModel::Isotropic,
        Some("Klucher") => TranspositionModel::Klucher, Some("Reindl") => TranspositionModel::Reindl,
        Some("Perez") => TranspositionModel::Perez,
        _ => { warnings.push("The source transposition model has no verified binding; using SolarCraft Perez.".into()); TranspositionModel::Perez }
    });
    let (installed_modules, target_kwp) = electrical(&mut project, root, &db, &sha, &mut warnings)?;
    apply_field_settings(&mut project, root, &mut warnings);
    let archived_shade_arrays = archived_shade::apply(&mut project, root, &mut archive, &mut warnings)?;
    let archived_scene_fields = archived_scene::apply(&mut project, root, &mut archive, &mut warnings);
    project.use_archived_scene_shading = project.scene.is_some() && archived_shade_arrays > 0;
    let photo_blobs = import_photos(&mut project, root, &mut archive, &mut warnings);
    if archived_scene_fields == 0 && (system.contains("3d") || system.contains("3D")) {
        warnings.push("3D fields retain angle and wired counts. This archive has no supported editable field geometry.".into());
    }
    project.grid.phases = number(root, &["ACNetz", "PhasenAnzahl"]).filter(|v| matches!(*v, 1 | 3)).unwrap_or(1);
    let minimum_bus_voltage = if island { 1.0 } else { 30.0 };
    if let Some(volts) = number::<f64>(root, &["ACNetz", "NetzSpannung"]).filter(|v| (minimum_bus_voltage..=500.0).contains(v)) {
        project.grid.voltage_v = volts;
    }
    project.grid.feed_in = if system.starts_with("OnGridPVAdvanced") || system.starts_with("OnGridPremium") { FeedIn::Full } else { FeedIn::Surplus };
    if flag(root, &["ACNetz", "IstProzentualeAbregelung"]) {
        if flag(root, &["ACNetz", "AbregelungAmWR"]) {
            warnings.push("The source specifies inverter-side power limiting, which is not imported.".into());
        } else if flag(root, &["ACNetz", "IsClippingRelativeToInstalledDCPower"]) {
            let fraction = number::<f64>(root, &["ACNetz", "ProzentualeAbregelung"]).filter(|v| (0.0..=1.0).contains(v))
                .ok_or_else(|| e("PVSOL_IMPORT", "Invalid saved export limit."))?;
            project.grid.export_limit_kw = Some(target_kwp * fraction);
        } else {
            warnings.push("The source's non-DC-relative export limit requires separate migration.".into());
        }
    }
    if flag(root, &["ACNetz", "IsGesamtverlust"]) {
        if let Some(fraction) = number::<f64>(root, &["ACNetz", "Gesamtverlust"]).filter(|v| (0.0..=0.25).contains(v)) {
            project.losses.ac_wiring_pct = Field::user(fraction * 100.0);
        }
    }
    if island {
        let bytes = member(&mut archive, "OffGrid.xml", 4 * 1024 * 1024)?
            .ok_or_else(|| bad("The legacy island design has no OffGrid.xml."))?;
        island::apply(&mut project, &bytes, &db, &sha, system == "OffGridPVWithBackUp", &mut warnings)?;
    }
    let mut load = None;
    let profile = read_project_load(root)?.map(|(values, minutes)| (values, minutes, "PV*SOL project base load"))
        .or_else(|| hourly("PowerConsumptionHourly").filter(|rows| rows.iter().all(|v| *v >= 0.0) && rows.iter().any(|v| *v > 0.0))
            .map(|values| (values, 60, "PV*SOL saved base-load ledger")));
    let profile = match profile {
        None if island => island::load(root, &mut warnings)?.map(|values| (values, 60, "PV*SOL island demand reconstruction")),
        other => other,
    };
    let mut load_source = profile.as_ref().map(|(_, _, label)| (*label).to_owned());
    let mut load_annual_kwh = profile.as_ref().map(|(values, _, _)| values.iter().sum());
    if let Some((values, minutes, label)) = profile {
        let data = encode(&values)?;
        project.loads.profile = LoadProfile::Series { blob: sha256_hex(&data), name: label.into(),
            step_minutes: minutes, values: values.len(), local_year: None };
        project.loads.annual_kwh = values.iter().sum();
        load = Some(data);
    } else if island {
        use solarcraft_model::plant::SyntheticProfile;
        if let Some(kwh) = number::<f64>(root, &["Verbrauch", "Gesamtverbrauch"]).filter(|x| (0.0..=1e12).contains(x)) {
            project.loads.profile = LoadProfile::Synthetic { profile: SyntheticProfile::Flat };
            project.loads.annual_kwh = kwh;
            load_source = Some("PV*SOL declared annual demand with SolarCraft flat profile".into());
            load_annual_kwh = Some(kwh);
            warnings.push("The island demand schedule could not be reconstructed. The declared annual consumption is spread evenly across the year.".into());
        } else {
            warnings.push("No usable island demand was found in the archive.".into());
        }
    }
    let bytes = encode(&weather)?;
    let weather_hash = sha256_hex(&bytes);
    if let [array] = project.arrays.as_mut_slice() {
        let bifacial = project.library.module(&array.module_id.value).and_then(|m| m.bifaciality).is_some();
        if let Some(shade) = array.archived_shading.as_mut().filter(|_| bifacial) {
            let estimated_hourly = rear.is_none();
            let profile = rear.or_else(|| distribute_monthly_rear(monthly_rear?, &weather.hours));
            if let Some(w_m2) = profile {
                let rear = solarcraft_model::archived_shading::RearIrradiance {
                    weather_hash: weather_hash.clone(), w_m2, estimated_hourly, albedo: Some(project.site.albedo.value),
                };
                if rear.valid() {
                    warnings.push(if estimated_hourly {
                        "Saved monthly rear irradiance is distributed over the original weather's daylight irradiance. Hourly rear readings are estimated."
                    } else {
                        "Saved hourly rear irradiance is used with the module's bifacial coefficient and original weather."
                    }.into());
                    shade.rear = Some(rear);
                }
            }
        }
    }
    if project.arrays.iter().any(|array| array.bifacial_rows.is_some()
        && array.archived_shading.as_ref().is_none_or(|shade| shade.rear.is_none())) {
        warnings.push("Bifacial arrays with saved row geometry can estimate rear irradiance when a saved rear profile is unavailable.".into());
    }
    project.weather = Some(solarcraft_model::WeatherRef { hash: weather_hash.clone(), provider: weather.provenance.provider,
        summary: format!("{weather_member} · {HOURS} hours") });
    derive::derive(&mut project);
    let reference = Reference { source_name: source_name.to_owned(), sha256: sha, format, system_type: system.to_owned(),
        weather_member, weather_diffuse, load_source, load_annual_kwh, installed_modules,
        inverters: project.electrical.inverters.len(), target_kwp,
        tracking_arrays: project.arrays.iter().filter(|a| a.tracking.is_some()).count(),
        nonlinear_shade_arrays: project.arrays.iter()
            .filter(|a| a.archived_shading.as_ref().is_some_and(|s| s.strings.is_some())).count(),
        archived_scene_fields,
        archived_shade_arrays, photoplan_arrays: project.roof_photos.len(),
        initial_inputs_hash: solarcraft_engine::inputs_hash(&project, &weather_hash), warnings, channels: saved };
    Ok(Imported { project, weather: bytes, load, photo_blobs, reference })
}

fn import_photos(project: &mut Project, root: Node<'_, '_>, archive: &mut ZipArchive<Cursor<Vec<u8>>>,
                 warnings: &mut Vec<String>) -> Vec<archived_photo::PhotoBlob> {
    let wired = derive::modules_by_array(&project.electrical);
    let fields = path_node(root, &["PVModule"]).into_iter().flat_map(|p| children(p, "Modulfelder"));
    let mut blobs = vec![];
    for (area, array) in fields.zip(&mut project.arrays).filter(|(area, _)| flag(*area, &["HasPhotoPlan"])) {
        let Some(module) = project.library.module(&array.module_id.value) else { continue; };
        match archived_photo::import(archive, area, array, module) {
            Ok(Some((photo, assets))) => {
                if array.roof.as_ref().is_none_or(|roof| roof.modules.len() < wired.get(&array.id).copied().unwrap_or(0) as usize) {
                    array.roof = None;
                    warnings.push(format!("PhotoPlan for {} has fewer saved placements than wired modules.", array.name.value));
                } else {
                    project.roof_photos.push(photo);
                    blobs.extend(assets);
                }
            }
            Ok(None) => {}
            Err(error) => warnings.push(format!("PhotoPlan for {} was omitted. {}", array.name.value, error.message)),
        }
    }
    if !project.roof_photos.is_empty() {
        warnings.push(format!("Recovered calibration, source images and placed modules for {} PhotoPlan areas.", project.roof_photos.len()));
    }
    blobs
}

fn equipment_record<'a>(db: &'a Value, family: &str, keys: &[&str], id: Option<&str>, version: Option<i64>, sha: &str)
    -> Result<(&'a Value, CatalogRef)> {
    let (id, version) = id.zip(version).filter(|(id, version)| !id.is_empty() && id.len() <= 80 && *version >= 0)
        .ok_or_else(|| e("PVSOL_IMPORT", format!("Missing embedded {family} ID or revision.")))?;
    let matched = keys.iter().find_map(|key| {
        db.get(key)?.as_array()?.iter().find(|r| {
            r.get("ID").or_else(|| r.get("Id")).and_then(Value::as_str).is_some_and(|x| x.eq_ignore_ascii_case(id))
                && r.get("Version").and_then(Value::as_i64) == Some(version)
        }).map(|record| (*key, record))
    });
    let (dataset, record) = matched.ok_or_else(|| e("PVSOL_IMPORT", format!("Embedded {family} {id}@{version} is missing.")))?;
    Ok((record, CatalogRef { family: family.to_owned(), id: id.to_owned(), version,
        source_sha256: sha.to_owned(), dataset: dataset.to_owned() }))
}

fn equipment(db: &Value, family: &str, keys: &[&str], id: Option<&str>, version: Option<i64>, sha: &str) -> Result<Value> {
    let (record, reference) = equipment_record(db, family, keys, id, version, sha)?;
    // Archive fields use Pascal and acronym casing. Map only the existing catalog
    // normalizer's supported fields, and keep online JSON's case-sensitive path.
    let common = "id version name description cellCount openCircuitVoltage isHalfCellModule cellType height width shortCircuitCurrent mpP_Voltage mpP_Current currentCoefficient voltageCoefficient outputpowerCoefficient specifiedOutputPower maxSysVoltage diodeCount lossVoltagePerBypassDiode increaseU0 onlyTransformerInverter bifacialityFactor depthOfPanel weight g2 umpp2 impp2 uoc2 isc2 iam standardCellConnection standardPartLoadOperation widthOfFrame isAvailable isCorrupt productType dbEntryType e0 e5 e10 e20 e30 e50 e75 e100 maxAC_Power maxInputVoltageDC nightConsumption standbyConsumption maxDC_Power dC_PowerRating nominalVoltageDC numberOfPhases withTransformer feedInFrom changeOfEtaDependentOnVoltage matchingEtaMPP20 matchingEtaMPP100 parallelOperating maxCurrentDC maxShortCircuitCurrentDC dC_InputCount voltageAC aC_PowerRating";
    let mut aliases: Vec<_> = common.split_whitespace().map(str::to_owned).collect();
    for group in 1..=3 {
        for key in ["NrOf", "U_MPP_Min", "U_MPP_Max", "MaxCurrent", "MaxShortCircuitCurrent", "MaxPower"] {
            aliases.push(format!("mppT{group}_{key}"));
        }
    }
    let obj = record.as_object().ok_or_else(|| e("PVSOL_IMPORT", "Invalid embedded equipment record."))?;
    let mut canonical = Map::new();
    for name in aliases {
        let mut matching = obj.iter().filter(|(key, _)| key.eq_ignore_ascii_case(&name));
        if let Some((_, data)) = matching.next() {
            if matching.next().is_some() { return Err(e("PVSOL_IMPORT", format!("Ambiguous equipment field {name}."))); }
            canonical.insert(name, data.clone());
        }
    }
    let company = record.pointer("/Company/Name").map(|value| json!({"name":value}));
    let normalized = solarcraft_catalog::normalize::normalize(family, &Value::Object(canonical), None,
        company.as_ref(), None, sha, &reference.dataset);
    let mut out = normalized.record.ok_or_else(|| e("PVSOL_IMPORT", format!("Cannot normalize {family} {}@{}: {}",
        reference.id, reference.version, normalized.issues.join("; "))))?;
    out["provenance"] = json!("imported");
    out["catalog_ref"] = json!(reference);
    crate::plant_ops::canonical_equipment(if family == "module" { "module" } else { "inverter" }, &out)
}

fn optimizer(node: Node<'_, '_>, area_id: &str, db: &Value, sha: &str) -> Result<BuckOptimizer> {
    if value(node, &["ModulfeldId"]) != Some(area_id) || number::<u32>(node, &["AnzahlInReihe"]) != Some(1)
        || number::<u32>(node, &["AnzahlStrings"]) != Some(1) {
        return Err(e("PVSOL_IMPORT", "Expected one saved optimizer per module in its assigned area."));
    }
    let (record, catalog_ref) = equipment_record(db, "optimizer", &["PowerOptimizers", "PowerOptimizersFrom3D"],
        value(node, &["ProductID"]), number(node, &["ProductVersion"]), sha)?;
    if record["PowerOptimizerMode"].as_i64() != Some(1) {
        return Err(e("PVSOL_IMPORT", "This optimizer uses a mode without a migrated electrical model."));
    }
    let get = |key| record.get(key).and_then(Value::as_f64)
        .ok_or_else(|| e("PVSOL_IMPORT", format!("Optimizer {} has no {key} rating.", catalog_ref.id)));
    let efficiency_curve = [(0.0,"E0"),(0.05,"E5"),(0.1,"E10"),(0.2,"E20"),(0.3,"E30"),(0.5,"E50"),(0.75,"E75"),(1.0,"E100")]
        .into_iter().filter_map(|(load,key)| record[key].as_f64().filter(|v| *v > 0.0).map(|v| [load,v])).collect();
    let model = BuckOptimizer { model: record["Name"].as_str().unwrap_or_default().to_owned(),
        rated_w: get("RatedDCOutputPower")?, mppt_min_v: get("U_MPP_Min")?, mppt_max_v: get("U_MPP_Max")?,
        input_max_v: get("MaxInputVoltage")?, input_max_a: get("MaxInputCurrent")?,
        output_max_v: get("MaxOutputVoltage")?, output_max_a: get("MaxOutputCurrent")?, efficiency_curve, catalog_ref };
    if !model.valid() { return Err(e("PVSOL_IMPORT", "Invalid embedded buck optimizer ratings or efficiency curve.")); }
    Ok(model)
}

fn apply_field_settings(project: &mut Project, root: Node<'_, '_>, warnings: &mut Vec<String>) {
    let Some(areas) = path_node(root, &["PVModule"]) else { return; };
    let fields: Vec<_> = children(areas, "Modulfelder").collect();
    let unanimous = |name: &str| {
        let mut values = fields.iter().map(|&field| number::<f64>(field, &["Verluste", "VerlusteSimulation", name]));
        let first = values.next().flatten().filter(|v| (0.0..=1.0).contains(v))?;
        values.all(|v| v == Some(first)).then_some(first)
    };
    if let Some(v) = unanimous("AlbedoJahreswert") { project.site.albedo = Field::user(v); }
    if let Some(v) = unanimous("VerschmutzungJahreswert") { project.losses.soiling_pct = Field::user(v * 100.0); }
    if let Some(v) = unanimous("MismatchHersteller") { project.losses.mismatch_pct = Field::user(v * 100.0); }
    if fields.iter().any(|&field| flag(field, &["Verluste", "VerschattungsObjekt", "IsHorizon"])) {
        warnings.push("The source contains a 2D horizon mask, which is not applied by the imported design.".into());
    }
}

fn read_project_load(root: Node<'_, '_>) -> Result<Option<(Vec<f64>, u32)>> {
    let Some(consumption) = path_node(root, &["Verbrauch", "VerbrauchGruppen"]) else { return Ok(None); };
    let profiles: Vec<_> = children(consumption, "Verbrauchsdefinition").flat_map(|node| children(node, "ImportierteProfile"))
        .filter(|node| value(*node, &["Values"]).is_some()).collect();
    let [profile] = profiles.as_slice() else { return Ok(None); };
    let Some(minutes) = number::<u32>(*profile, &["Intervall"]).filter(|v| matches!(*v, 1 | 5 | 15 | 30 | 60)) else { return Ok(None); };
    let count = HOURS * 60 / minutes as usize;
    if number::<usize>(*profile, &["NoOfValues"]) != Some(count) || number::<u32>(*profile, &["NoOfDays"]) != Some(365) {
        return Ok(None);
    }
    let values = read_load_values(value(*profile, &["Values"]).unwrap_or_default(), minutes, count)?;
    Ok(Some((values, minutes)))
}

fn read_tracking(array: &Array, node: Node<'_, '_>) -> Option<Tracker> {
    let mode = number::<u32>(node, &["KindOfTracking"])?;
    let limit = number::<f64>(node, &["RotationLimitOne"])?;
    let azimuth_limit = || number::<f64>(node, &["RotationLimitTwo"]);
    let gcr = if matches!(mode, 1 | 2) && flag(node, &["Backtracking"]) {
        Some(number::<f64>(node, &["GroundUtilization"])?)
    } else { None };
    let tracker = match mode {
        1 | 2 => Tracker::SingleAxis {
            axis_azimuth_deg: if mode == 1 { array.azimuth.value } else { 90.0 },
            axis_tilt_deg: array.tilt.value, max_rotation_deg: limit / 2.0, backtracking_gcr: gcr,
        },
        3 => Tracker::VerticalAxis {
            center_azimuth_deg: if array.azimuth.value == 0.0 { 0.0 } else { 180.0 },
            azimuth_travel_deg: limit,
        },
        4 => Tracker::DualAxis {
            center_azimuth_deg: 180.0, max_tilt_deg: limit, azimuth_travel_deg: azimuth_limit()?,
        },
        _ => return None,
    };
    tracker.valid().then_some(tracker)
}

fn read_bifacial_rows(array: &Array, node: Node<'_, '_>) -> Option<solarcraft_model::BifacialRows> {
    if value(node, &["BacksideIrradianceModel", "Value"]) != Some("ViewFactor") { return None; }
    let rows = solarcraft_model::BifacialRows {
        slant_width_m: number(node, &["WidthOfMountingStructure", "Value"])?,
        row_pitch_m: number(node, &["RowDistanceOfMountingStructure", "Value"])?,
        lower_edge_m: number(node, &["HeightFromGroundOfMountingStructure", "Value"])?,
        mounting_shade: number(node, &["ShadingByMountingStructure", "Value"])?,
        force_geometry: false,
    };
    rows.valid(array.tilt.value).then_some(rows)
}

fn read_load_values(encoded: &str, minutes: u32, count: usize) -> Result<Vec<f64>> {
    let encoded: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let packed = STANDARD.decode(encoded).map_err(|error| e("PVSOL_IMPORT", error))?;
    let data = read_limited(GzDecoder::new(&packed[..]), (count * 8) as u64)?;
    if data.len() != count * 8 { return Err(e("PVSOL_IMPORT", "The embedded load profile length does not match its interval count.")); }
    data.as_chunks::<8>().0.iter().map(|b| {
        let watts = f64::from_le_bytes(*b);
        (0.0..=1e12).contains(&watts).then_some(watts * f64::from(minutes) / 60_000.0)
    }).collect::<Option<Vec<_>>>().ok_or_else(|| e("PVSOL_IMPORT", "The embedded load profile contains an invalid power reading."))
}

fn read_weather(bytes: &[u8], label: &str, diffuse: Option<&[f64]>, spectral: f64) -> Result<(Weather, f64, String)> {
    use solarcraft_pv::{days_from_civil, irradiance::{dni_extra, erbs}, spa::{delta_t_estimate, solar_position, Observer}};
    const HEADER: usize = 2517;
    let error = |message| e("PVSOL_IMPORT", message);
    if bytes.len() != HEADER + HOURS * 12 || bytes.get(513..520) != Some(b"T;G;WGS") || bytes[512] != 7 {
        return Err(error("Only the observed 8760-row, three-channel T;G;WGS WBV layout is supported."));
    }
    let f64_at = |i| f64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
    let i32_at = |i| i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let (lat, lon, elevation) = (f64_at(781), -f64_at(789), f64_at(866));
    let tz = -f64_at(948);
    if i32_at(768) != 1 || i32_at(772) != HOURS as i32 || bytes[780] != 0
        || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon)
        || !(-500.0..=10_000.0).contains(&elevation) || !(-12.0..=14.0).contains(&tz) {
        return Err(error("Invalid WBV location, interval range or standard-time reference."));
    }
    let diffuse = diffuse.filter(|rows| rows.len() == HOURS && (0.0..0.5).contains(&spectral));
    let year = 2025;
    let start = days_from_civil(year, 1, 1) * 86400 - (tz * 3600.0).round() as i64;
    let observer = Observer::new(lat, lon, elevation);
    let mut hours = Vec::with_capacity(HOURS);
    for (index, row) in bytes[HEADER..].as_chunks::<12>().0.iter().enumerate() {
        let at = |i| f64::from(f32::from_le_bytes(row[i..i + 4].try_into().unwrap()));
        let (temp_air, ghi, wind) = (at(0), at(4), at(8));
        if !(-100.0..=100.0).contains(&temp_air) || !(0.0..=1800.0).contains(&ghi) || !(0.0..=100.0).contains(&wind) {
            return Err(error("An embedded WBV weather row is missing or outside the supported range."));
        }
        let unix_start = start + index as i64 * 3600;
        let unix_mid = unix_start + 1800;
        let sun = solar_position(unix_mid as f64, &observer, delta_t_estimate(year as f64));
        let (dni, dhi) = if let Some(saved) = diffuse {
            let cosine = sun.zenith.to_radians().cos();
            let dhi = (saved[index] / (1.0 - spectral)).clamp(0.0, ghi);
            if cosine > 0.065 {
                let dni = ((ghi - dhi) / cosine).min(1450.0);
                (dni, ghi - dni * cosine)
            } else { (0.0, ghi) }
        } else {
            erbs(ghi, sun.zenith, dni_extra((index / 24 + 1) as f64, 1366.1))
        };
        hours.push(Hour { unix_mid, unix_start: Some(unix_start), ghi, dni, dhi, temp_air, wind, pressure: observer.pressure * 100.0 });
    }
    let basis = if diffuse.is_some() { "saved_hourly_diffuse" } else { "erbs_from_wbv_global" };
    let provenance = Provenance { provider: "PV*SOL embedded WBV".into(), dataset: label.into(),
        url: String::new(), retrieved_unix: 0, licence: "Imported from the supplied local project.".into(), notes: vec![basis.into()] };
    Ok((Weather { latitude: lat, longitude: lon, elevation, horizon_applied: false, provenance, hours }, tz, basis.into()))
}

type SavedXml = (BTreeMap<String, Channel>, Option<[f64; 12]>);

fn read_saved_xml(bytes: &[u8]) -> Result<SavedXml> {
    let document = document(bytes, "SimResults.xml")?;
    let root = document.root_element();
    if value(root, &["ModelString"]).is_none_or(str::is_empty) { return Ok(SavedXml::default()); }
    let mut saved = BTreeMap::new();
    for (key, group, section) in [(AC, "EnergiebilanzResultsWr", "EB_WR5_PVEnergieAC"),
        (DC, "EnergiebilanzResultsVerschaltung", "EB_PV17_ErzeugtePVEnergie")] {
        let Some(node) = path_node(root, &[group, section]) else { continue; };
        let annual = number::<f64>(node, &["YearlyValue"]).filter(|v| v.is_finite()).map(|v| v / WS_PER_KWH);
        let monthly: Option<Vec<_>> = children(node, "MonthlyValues").map(|n| number::<f64>(n, &["value"])
            .filter(|v| v.is_finite()).map(|v| v / WS_PER_KWH)).collect();
        let monthly = monthly.filter(|v| v.len() == 12);
        if annual.is_some() || monthly.is_some() {
            saved.insert(key.into(), Channel { annual_kwh: annual, monthly_kwh: monthly, ..Default::default() });
        }
    }
    Ok((saved, read_monthly_rear(root)))
}

fn read_monthly_rear(root: Node<'_, '_>) -> Option<[f64; 12]> {
    let mut totals = None;
    for field in children(root, "EnergiebilanzResultsPVFeld") {
        let node = path_node(field, &["EB_Meteo4a_BacksideIrradiance"])?;
        let monthly: Vec<_> = children(node, "MonthlyValues").map(|month| {
            number::<f64>(month, &["value"]).filter(|v| v.is_finite() && *v >= 0.0)
                .map(|ws| ws / WS_PER_KWH)
        }).collect::<Option<_>>()?;
        let monthly: [f64; 12] = monthly.try_into().ok()?;
        let annual = number::<f64>(node, &["YearlyValue"])? / WS_PER_KWH;
        if !annual.is_finite() || annual <= 0.0 || (monthly.iter().sum::<f64>() - annual).abs() > 0.001
            || totals.is_some_and(|earlier| earlier != monthly) {
            return None;
        }
        totals = Some(monthly);
    }
    totals
}

/// WBV rows start on January 1 in local standard time. Weight each month's
/// saved rear energy by its original GHI and preserve the monthly integral.
fn distribute_monthly_rear(monthly: [f64; 12], hours: &[Hour]) -> Option<Vec<f32>> {
    if hours.len() != HOURS { return None; }
    let mut weights = [0.0; 12];
    let month = |index| solarcraft_engine::calendar::local_time(index).month as usize - 1;
    for (index, hour) in hours.iter().enumerate() {
        weights[month(index as u32)] += hour.ghi;
    }
    if monthly.iter().zip(weights).any(|(&rear, ghi)| rear > 0.0 && ghi == 0.0) {
        return None;
    }
    Some(hours.iter().enumerate().map(|(index, hour)| {
        let m = month(index as u32);
        if weights[m] > 0.0 { (hour.ghi * monthly[m] * 1000.0 / weights[m]) as f32 } else { 0.0 }
    }).collect())
}

#[cfg(test)]
#[test]
fn monthly_rear_preserves_energy_in_local_months_and_dark_hours() {
    let monthly = std::array::from_fn(|m| m as f64 + 1.0);
    let mut hours: Vec<_> = (0..HOURS).map(|i| Hour {
        unix_mid: 1800 + i as i64 * 3600, unix_start: None,
        ghi: if i % 24 == 12 { 800.0 } else { 0.0 }, dni: 0.0, dhi: 0.0,
        temp_air: 25.0, wind: 0.0, pressure: 101325.0,
    }).collect();
    let values = distribute_monthly_rear(monthly, &hours).unwrap();
    let mut totals = [0.0; 12];
    for (i, value) in values.into_iter().enumerate() {
        if hours[i].ghi == 0.0 { assert_eq!(value, 0.0); }
        let month = solarcraft_engine::calendar::local_time(i as u32).month as usize - 1;
        totals[month] += f64::from(value) / 1000.0;
    }
    assert!(monthly.into_iter().zip(totals).all(|(a, b)| (a - b).abs() < 1e-6));
    for hour in &mut hours[..31 * 24] { hour.ghi = 0.0; }
    assert!(distribute_monthly_rear(monthly, &hours).is_none());
}

type CoreSeries = (BTreeMap<String, Vec<f64>>, Option<Vec<f64>>, Option<Vec<f32>>);

fn read_core(bytes: &[u8]) -> Result<CoreSeries> {
    let text = std::str::from_utf8(strip_bom(bytes)).map_err(|error| e("PVSOL_REFERENCE", error))?;
    let mut lines = text.lines();
    let header: Vec<_> = lines.next().unwrap_or_default().split('\t').collect();
    let units: Vec<_> = lines.next().unwrap_or_default().split('\t').collect();
    if header.first() != Some(&"time") { return Err(e("PVSOL_REFERENCE", "Expected the saved core time column.")); }
    let recognized = [(AC, "EnergieBilanz.WRAcEnergie", "Ws"), (DC, "EnergieBilanz.PVErzeugteEnergie", "Ws"),
        ("diffuse", "MeteoBlock.IrradiationDiffuseOut", "Ws/m²"),
        ("rear", "EnergieBilanz.Meteo0.MeteoBacksideIrradiance", "W/m²")];
    let mut fields: Vec<_> = recognized.into_iter().filter_map(|(key, label, unit)| {
        let col = header.iter().position(|name| name.trim() == label)?;
        units.get(col).filter(|supplied| supplied.trim() == unit)?;
        Some((key, col, Vec::with_capacity(HOURS)))
    }).collect();
    if fields.is_empty() { return Ok((BTreeMap::new(), None, None)); }
    let mut row_count = 0;
    for line in lines.filter(|line| !line.is_empty()) {
        let columns: Vec<_> = line.split('\t').collect();
        row_count += 1;
        if row_count > HOURS || columns.first().and_then(|s| s.trim().parse::<usize>().ok()) != Some(row_count * 3600) {
            return Err(e("PVSOL_REFERENCE", "Saved hourly results must end each consecutive hour at 3600-second increments."));
        }
        for (name, col, values) in &mut fields {
            let entry = columns.get(*col).ok_or_else(|| e("PVSOL_REFERENCE", "Truncated saved result row."))?.trim();
            let ws = if entry.is_empty() { 0.0 } else {
                entry.replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite())
                    .ok_or_else(|| e("PVSOL_REFERENCE", "Saved result column contains an invalid numeric reading."))?
            };
            // This rear column's writer stores energy per interval despite its unit label.
            values.push(ws / if matches!(*name, "diffuse" | "rear") { 3600.0 } else { WS_PER_KWH });
        }
    }
    if row_count != HOURS { return Err(e("PVSOL_REFERENCE", "Expected 8760 saved hourly rows.")); }
    let mut saved = BTreeMap::new();
    let mut diffuse = None;
    let mut rear = None;
    for (name, _, values) in fields {
        match name {
            "diffuse" => diffuse = Some(values),
            "rear" if values.iter().all(|v| (0.0..=1800.0).contains(v)) && values.iter().any(|&v| v > 0.0) =>
                rear = Some(values.into_iter().map(|v| v as f32).collect()),
            "rear" => {}
            _ => { saved.insert(name.into(), values); }
        }
    }
    Ok((saved, diffuse, rear))
}

fn children<'a, 'b>(node: Node<'a, 'b>, name: &'static str) -> impl Iterator<Item = Node<'a, 'b>> {
    node.children().filter(move |child| child.has_tag_name(name))
}

fn path_node<'a, 'b>(node: Node<'a, 'b>, path: &[&str]) -> Option<Node<'a, 'b>> {
    path.iter().try_fold(node, |parent, name| parent.children().find(|child| child.has_tag_name(*name)))
}

fn value<'a, 'b>(node: Node<'a, 'b>, path: &[&str]) -> Option<&'a str> {
    let child = path_node(node, path)?;
    children(child, "Value").next().unwrap_or(child).text().map(str::trim)
}

fn number<T: std::str::FromStr>(node: Node<'_, '_>, path: &[&str]) -> Option<T> {
    value(node, path)?.parse().ok()
}

fn flag(node: Node<'_, '_>, path: &[&str]) -> bool {
    value(node, path).is_some_and(|s| s.eq_ignore_ascii_case("true") || s == "1")
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|error| e("PVSOL_IMPORT", error))
}

fn strip_bom(bytes: &[u8]) -> &[u8] { bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes) }

fn document<'a>(data: &'a [u8], name: &str) -> Result<Document<'a>> {
    let text = std::str::from_utf8(strip_bom(data)).map_err(|error| e("PVSOL_IMPORT", format!("{name}: {error}")))?;
    Document::parse(text).map_err(|error| e("PVSOL_IMPORT", format!("{name}: {error}")))
}

fn read_limited(input: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.take(limit + 1).read_to_end(&mut bytes).map_err(|error| e("PVSOL_IMPORT", error))?;
    if bytes.len() as u64 > limit { return Err(e("PVSOL_IMPORT", "A project input exceeds the supported size.")); }
    Ok(bytes)
}

fn archive_path(name: &str) -> Cow<'_, str> {
    if name.contains('\\') { Cow::Owned(name.replace('\\', "/")) } else { Cow::Borrowed(name) }
}

fn member(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str, limit: u64) -> Result<Option<Vec<u8>>> {
    let raw = {
        let mut matches = archive.file_names().filter_map(|n| n.ok())
            .filter(|n| archive_path(n) == name);
        let Some(raw) = matches.next() else { return Ok(None); };
        if matches.next().is_some() {
            return Err(e("PVSOL_IMPORT", format!("Duplicate project member {name}.")));
        }
        raw.into_owned()
    };
    match archive.by_name(&raw) {
        Ok(entry) if entry.size() <= limit && entry.is_file() => read_limited(entry, limit).map(Some),
        Ok(_) => Err(e("PVSOL_IMPORT", format!("Unexpected size or type for project member {name}."))),
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(error) => Err(e("PVSOL_IMPORT", error)),
    }
}

fn electrical(project: &mut Project, root: Node<'_, '_>, db: &Value, sha: &str, warnings: &mut Vec<String>) -> Result<(u32, f64)> {
    let fail = |message: String| e("PVSOL_IMPORT", message);
    let fields: Vec<_> = path_node(root, &["PVModule"]).into_iter().flat_map(|p| children(p, "Modulfelder")).collect();
    let wiring: Vec<_> = children(root, "Wechselrichter").flat_map(|root| children(root, "Verschaltung"))
        .flat_map(|group| children(group, "WRObj")).collect();
    if !(1..=100).contains(&fields.len()) || !(1..=256).contains(&wiring.len()) {
        return Err(fail("Expected 1–100 module areas and a wired inverter configuration.".into()));
    }
    let mut ids = HashMap::new();
    let array_seed = project.arrays[0].clone();
    project.arrays.clear();
    project.library = Default::default();
    for (i, node) in fields.iter().copied().enumerate() {
        let id = value(node, &["ModulfeldID"]).ok_or_else(|| fail("Module area ID is missing.".into()))?;
        let record = equipment(db, "module", &["PVModules", "PVModulesFrom3D"],
            value(node, &["ModulID"]), number(node, &["ModulVersion"]), sha)?;
        let module: solarcraft_model::equipment::ModuleSpec = serde_json::from_value(record).map_err(|error| e("PVSOL_IMPORT", error))?;
        let mut array = array_seed.clone();
        array.id = format!("array-{}", i + 1);
        if ids.insert(id, i).is_some() { return Err(fail("Duplicate module area ID.".into())); }
        array.name = Field::user(value(node, &["Modulname"]).unwrap_or(id).to_owned());
        array.tilt = Field::user(number::<f64>(node, &["Modulneigung"]).filter(|v| (0.0..=90.0).contains(v))
            .ok_or_else(|| fail(format!("Missing or invalid tilt for {}.", array.name.value)))?);
        array.azimuth = Field::user(number::<f64>(node, &["Modulausrichtung"]).filter(|v| (-360.0..=360.0).contains(v))
            .ok_or_else(|| fail(format!("Missing or invalid azimuth for {}.", array.name.value)))?.rem_euclid(360.0));
        array.module_id = Field::user(module.id.clone());
        if module.bifaciality.is_some() {
            array.bifacial_rows = path_node(node, &["Bifaciality"]).and_then(|settings| read_bifacial_rows(&array, settings));
        }
        if !project.library.modules.iter().any(|m| m.id == module.id) {
            project.library.modules.push(module);
        }
        if let Some(tracking) = path_node(node, &["PVTracking"]).filter(|n| number::<u32>(*n, &["KindOfTracking"]).is_some_and(|kind| kind != 0)) {
            array.tracking = read_tracking(&array, tracking);
            if array.tracking.is_none() {
                warnings.push(format!("{} has unsupported tracking settings. This replay uses its fixed orientation.", array.name.value));
            } else if flag(tracking, &["Backtracking"]) {
                warnings.push(format!("{} uses SolarCraft's moving-row backtracking with the saved ground coverage ratio.", array.name.value));
            }
        }
        project.arrays.push(array);
    }
    let mut units = vec![];
    let mut totals = vec![0_u32; fields.len()];
    let mut assignments = vec![None; fields.len()];
    let mut optimized_modules = 0_u32;
    let mut physical_series = 0_u32;
    for node in wiring {
        let record = equipment(db, "pvinverter", &["PVInverters", "PVInvertersFrom3D"],
            value(node, &["InverterID"]), number(node, &["InverterVersion"]), sha)?;
        let inverter: solarcraft_model::equipment::InverterSpec = serde_json::from_value(record).map_err(|error| e("PVSOL_IMPORT", error))?;
        let quantity = number::<u32>(node, &["Haeufigkeit"]).filter(|n| (1..=128).contains(n))
            .ok_or_else(|| fail("Unsupported inverter multiplicity.".into()))?;
        if units.len() + quantity as usize > 256 || !(1..=64).contains(&inverter.mppt_count) {
            return Err(fail("The project exceeds native inverter or MPPT limits.".into()));
        }
        let mut mppts = vec![vec![]; inverter.mppt_count as usize];
        let mut populated = vec![false; mppts.len()];
        let mut has_optimizer = false;
        for input in children(node, "MPPListe") {
            let slot = number::<usize>(input, &["MPPListe"]).filter(|i| *i < mppts.len() && !populated[*i])
                .ok_or_else(|| fail("Invalid or duplicate inverter MPPT slot.".into()))?;
            populated[slot] = true;
            for field in children(input, "ModulfeldTeil") {
                let sets: Vec<_> = children(field, "PolyStringVerschaltung").collect();
                if sets.iter().all(|&set| children(set,"Straenge").next().is_none()) {
                    if number::<u32>(field, &["AnzahlStrings"]).is_some_and(|n| n > 0) {
                        return Err(fail("The inverter uses an unrecognized legacy string layout.".into()));
                    }
                    continue;
                }
                for set in sets {
                    let chain = flag(set, &["StraengeInReihe"]).then_some(mppts[slot].len() as u32);
                    let mut chain_count = None;
                    for string in children(set, "Straenge") {
                        let area_id = value(string, &["ModulfeldId"]).filter(|id| ids.contains_key(id))
                            .ok_or_else(|| fail("A string points to an unknown PV area.".into()))?;
                        let i = ids[area_id];
                        let modules = number::<u32>(string, &["AnzahlInReihe"]).filter(|n| (1..=1000).contains(n));
                        let strings = number::<u32>(string, &["AnzahlStrings"]).filter(|n| (1..=1000).contains(n));
                        let (modules, strings) = modules.zip(strings).ok_or_else(|| fail("Invalid saved string dimensions.".into()))?;
                        if chain.is_some() && *chain_count.get_or_insert(strings) != strings {
                            return Err(fail("Series-connected segments require equal parallel string counts.".into()));
                        }
                        let count = modules * strings * quantity;
                        totals[i] = totals[i].checked_add(count).filter(|v| *v <= 100_000)
                            .ok_or_else(|| fail("Too many wired modules in a PV area.".into()))?;
                        assignments[i].get_or_insert_with(|| (inverter.id.clone(), modules, strings, quantity));
                        let optimizer = path_node(string, &["powerOptimizer"]).map(|node| optimizer(node, area_id, db, sha)).transpose()?;
                        if optimizer.is_some() { has_optimizer = true; optimized_modules += count; }
                        mppts[slot].push(StringGroup { array_id: project.arrays[i].id.clone(), modules_per_string: modules, strings,
                            series_chain: chain, optimizer });
                    }
                    if let Some(strings) = chain_count { physical_series += strings * quantity; }
                }
            }
        }
        if (flag(node, &["HatLeistungsOpt"]) || flag(node, &["WithPowerOptimizerProduct"])) && !has_optimizer {
            return Err(fail("The source marks this inverter optimized but supplies no supported device assignment.".into()));
        }
        for _ in 0..quantity {
            units.push(InverterUnit { id: format!("inv-{}", units.len() + 1),
                name: format!("{} {}", inverter.model, units.len() + 1), inverter_id: inverter.id.clone(),
                mppts: mppts.clone(), interconnected: vec![] });
        }
        if !project.library.inverters.iter().any(|i| i.id == inverter.id) { project.library.inverters.push(inverter); }
    }
    let mut target_kwp = 0.0;
    for (i, array) in project.arrays.iter_mut().enumerate() {
        let Some((inverter, modules, strings, quantity)) = assignments[i].take() else {
            array.inverter_id = Field::user(units[0].inverter_id.clone());
            array.modules_per_string = Field::user(0);
            array.strings = Field::user(0);
            let declared = number::<u32>(fields[i], &["Modulanzahl"]).unwrap_or(0);
            warnings.push(format!("{} has no saved wire assignment for its {declared} planned modules. Its orientation and equipment remain editable. It contributes zero modules to the replay.", array.name.value));
            continue;
        };
        array.inverter_id = Field::user(inverter);
        array.modules_per_string = Field::user(modules);
        array.strings = Field::user(strings);
        array.inverter_count = Field::user(quantity);
        let model = project.library.module(&array.module_id.value).unwrap();
        let nameplate = model.nameplate_w.filter(|p| *p > 0.0).unwrap_or_else(|| model.pmp_stc());
        let kwp = f64::from(totals[i]) * nameplate / 1000.0;
        array.target_kwp = Field::user(kwp);
        target_kwp += kwp;
        if number::<u32>(fields[i], &["Modulanzahl"]).is_some_and(|total| total != totals[i]) {
            warnings.push(format!("Used wired count {} rather than the design count for {}.", totals[i], array.name.value));
        }
    }
    if totals.iter().all(|&total| total == 0) { return Err(fail("The project contains no wired PV strings.".into())); }
    if physical_series > 0 { warnings.push(format!("Preserved {physical_series} physical series strings spanning multiple module areas.")); }
    if optimized_modules > 0 {
        warnings.push(format!("Applied archived buck converter ratings and efficiency to {optimized_modules} wired modules. Vendor mismatch, subgroup and bypass logic require separate validation."));
    }
    project.electrical = Electrical { manual: true, inverters: units };
    Ok((totals.into_iter().sum(), target_kwp))
}
