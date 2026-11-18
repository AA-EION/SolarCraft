// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(clippy::needless_range_loop)]
//! Annual grid-connected PV simulation (slice 1).
//!
//! Every loss is the difference between two consecutive stages of one chain
//! of hourly calculations, so the loss waterfall closes exactly (checked).
//! Stages: horizontal → plane of array → IAM → soiling → effective
//! irradiance; nominal STC energy → irradiance level → temperature → MPP
//! voltage window → module quality/LID → mismatch → DC wiring → inverter
//! efficiency → clipping → night tare → AC wiring → availability → grid.
use serde::{Deserialize, Serialize};
use solarcraft_model::validate::Severity;
use solarcraft_model::{Project, TranspositionModel};
use solarcraft_pv::{diode, iam, irradiance as irr, spa, temperature};
use solarcraft_weather::Weather;

pub mod calendar;
pub mod electrical;
pub mod energy;
pub mod finance;
pub mod loads;
pub mod demand;
pub mod wiring;
pub mod flexible;
pub mod islands;
mod partial_shading;

pub const ENGINE_VERSION: &str = concat!("solarcraft-engine ", env!("CARGO_PKG_VERSION"), " (electrical 8)");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step { pub key: String, pub label: String, pub value: f64, pub delta: f64, pub delta_pct: f64 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Month { pub month: u32, pub ghi_kwh_m2: f64, pub poa_kwh_m2: f64, pub dc_kwh: f64, pub ac_kwh: f64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annual {
    pub dc_kwp: f64, pub ac_kw: f64, pub ghi_kwh_m2: f64, pub poa_kwh_m2: f64,
    pub ac_kwh: f64, pub specific_yield_kwh_kwp: f64, pub performance_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    /// SHA-256 of the design inputs (project without customer, name and variants), weather and engine version.
    #[serde(default)]
    pub inputs_hash: String,
    #[serde(default)]
    pub pv_model: solarcraft_model::PvModel,
    pub engine: String, pub weather_hash: String, pub weather_provider: String, pub transposition: String, pub hours: usize, pub ledger_closure_error_kwh: f64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hourly {
    pub unix_mid: Vec<i64>, pub poa_w_m2: Vec<f32>, pub t_cell_c: Vec<f32>, pub dc_w: Vec<f32>, pub ac_w: Vec<f32>,
    #[serde(default)] pub ev_wh: Vec<f32>,
    #[serde(default)] pub heating_wh: Vec<f32>,
    #[serde(default)] pub dc_cable_wh: Vec<f32>,
    #[serde(default)] pub ev_discharge_wh: Vec<f32>,
    #[serde(default)] pub fleet_soc_pct: Vec<Vec<f32>>,
    #[serde(default)] pub heat_store_kwh: Vec<f32>,
    #[serde(default)] pub cluster_unserved_wh: Vec<Vec<f32>>,
    #[serde(default)] pub cluster_soc_pct: Vec<Vec<f32>>,
    #[serde(default)] pub routed_cable_wh: Vec<Vec<f32>>,
}

/// A result note with a stable code (translated by reports/UI) and English text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note { pub code: String, pub subject: String, pub value: f64, pub text: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimResult {
    pub manifest: RunManifest,
    /// Per inverter and MPPT: stringing and simulated energy.
    #[serde(default)]
    pub electrical: Vec<UnitResult>,
    /// Irradiation incident on the plane and effective at the module cells,
    /// averaged across each assigned array.
    #[serde(default)]
    pub array_irradiation: Vec<ArrayIrradiation>,
    /// Hourly energy flows (PV, consumption, battery, grid).
    #[serde(default)]
    pub flows: energy::Flows,
    #[serde(default)]
    pub balance: energy::Balance,
    #[serde(default)]
    pub finance: Option<finance::Finance>,
    #[serde(default)] pub island_finance: Option<finance::IslandFinance>,
    #[serde(default)] pub ev: Option<demand::EvResult>,
    #[serde(default)] pub heating: Option<demand::HeatingResult>,
    #[serde(default)] pub fleet: Option<flexible::FleetResult>,
    #[serde(default)] pub heat_store: Option<flexible::TankResult>,
    #[serde(default)] pub clusters: Vec<islands::ClusterResult>,
    #[serde(default)] pub routed_cables: Vec<wiring::CircuitResult>,
    /// Local hour of year (reference calendar) for each simulated hour.
    #[serde(default)]
    pub local_hour: Vec<u32>,
    pub annual: Annual,
    /// Irradiance chain (kWh/m², area-weighted over arrays).
    pub irradiance_ledger: Vec<Step>,
    /// Energy chain (kWh).
    pub energy_ledger: Vec<Step>,
    pub monthly: Vec<Month>,
    pub hourly: Hourly,
    pub warnings: Vec<Note>,
    /// Per module of each 3D field: annual beam loss and sky view (for the shading map).
    #[serde(default)]
    pub module_shading: Vec<FieldShade>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FieldShade {
    pub field: String,
    pub array_id: String,
    pub diffuse_factor: f64,
    pub beam_loss_pct: f64,
    pub beam_area_loss_pct: f64,
    pub traced_directions: usize,
    pub modules: Vec<solarcraft_scene::shade::ModuleShade>,
    #[serde(default)]
    pub nonlinear: bool,
    #[serde(default)]
    pub half_cell: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArrayIrradiation {
    pub array_id: String,
    pub name: String,
    pub poa_kwh_m2: [f64; 12],
    pub effective_kwh_m2: [f64; 12],
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MpptResult { pub index: u32, pub strings: String, pub stc_kw: f64, pub dc_kwh: f64, pub limited_hours: u32, pub mismatch_kwh: f64, pub limit_loss_kwh: f64 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnitResult {
    pub id: String, pub name: String, pub inverter_id: String, pub stc_kw: f64, pub ac_kw: f64, pub dc_ac_ratio: f64,
    pub dc_kwh: f64, pub ac_kwh: f64, pub clipped_kwh: f64, pub mppts: Vec<MpptResult>,
    /// Signed AC energy after wiring, availability and night consumption.
    #[serde(default)]
    pub monthly_net_ac_kwh: [f64; 12],
}

#[derive(Debug)]
pub struct SimError(pub String);
impl std::fmt::Display for SimError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) } }
impl std::error::Error for SimError {}

const IRR_KEYS: [(&str, &str); 7] = [
    ("ghi", "Global horizontal irradiation"), ("poa", "Global incident in plane (transposition)"), ("shading", "After near shading (obstacles)"),
    ("bypass", "After partial-shading mismatch (bypass diodes)"),
    ("iam", "After incidence angle (reflection)"), ("soiling", "After soiling"), ("effective", "Effective irradiation on modules"),
];
const E_KEYS: [(&str, &str); 16] = [
    ("nominal", "Nominal energy at STC efficiency"), ("irradiance_level", "Irradiance level"), ("temperature", "Module temperature"),
    ("strings", "Parallel-string mismatch"), ("mpp_window", "Inverter input limits (MPP window, current)"), ("quality", "Module quality"), ("lid", "Light-induced degradation"),
    ("mismatch", "Mismatch"), ("dc_wiring", "DC wiring"), ("dc_out", "Array DC output"),
    ("inverter", "Inverter efficiency"), ("clipping", "Inverter clipping (power limit)"), ("night", "Inverter night consumption"),
    ("ac_wiring", "AC wiring"), ("availability", "Availability"), ("grid", "Energy to grid"),
];

/// `progress(fraction) -> continue?`
pub fn simulate(p: &Project, w: &Weather, weather_hash: &str, load_series: Option<&[f64]>, mut progress: impl FnMut(f64) -> bool) -> Result<SimResult, SimError> {
    let inputs_hash = inputs_hash(p, weather_hash);
    let active = solarcraft_model::workflow::Workflow::active(p);
    let p = active.as_ref();
    let errors: Vec<_> = solarcraft_model::validate::validate_active(p).into_iter().filter(|i| i.severity == Severity::Error).collect();
    if !errors.is_empty() {
        return Err(SimError(format!("project has errors: {}", errors.iter().map(|e| format!("{} ({})", e.message, e.code)).collect::<Vec<_>>().join("; "))));
    }
    let n = w.hours.len();
    if n != 8760 {
        return Err(SimError(format!("Expected 8760 hourly weather intervals, found {n}.")));
    }
    let local_hour: Vec<u32> = w.hours.iter().map(|hour| {
        calendar::local_interval_hour(w.interval_start(hour), p.site.utc_offset())
    }).collect();
    if let Some(index) = w.hours.iter().position(|hour| {
        ![hour.ghi, hour.dni, hour.dhi, hour.temp_air].iter().all(|v| v.is_finite())
    }) {
        return Err(SimError(format!("Weather row {} is missing an irradiance or air temperature value.", index + 1)));
    }
    if let Some(index) = local_hour.windows(2).position(|pair| (pair[0] + 1) % 8760 != pair[1]) {
        return Err(SimError(format!("Weather rows {} and {} are not consecutive hours.", index + 1, index + 2)));
    }
    let obs = spa::Observer::new(p.site.latitude.value, p.site.longitude.value, if w.elevation.is_finite() { w.elevation } else { p.site.elevation.value });
    let model = match p.simulation.transposition.value {
        TranspositionModel::Isotropic => irr::SkyModel::Isotropic, TranspositionModel::HayDavies => irr::SkyModel::HayDavies,
        TranspositionModel::Klucher => irr::SkyModel::Klucher, TranspositionModel::Reindl => irr::SkyModel::Reindl, TranspositionModel::Perez => irr::SkyModel::Perez,
    };
    let l = &p.losses;
    let f = |x: f64| 1.0 - x / 100.0;
    let elec = solarcraft_model::derive::electrical(p);
    let assigned = solarcraft_model::derive::modules_by_array(&elec);
    let mut irr_tot = [0.0f64; 7];
    let mut e_tot = [0.0f64; 16];
    let mut area_tot = 0.0;
    let mut monthly = vec![Month::default(); 12];
    for (i, m) in monthly.iter_mut().enumerate() { m.month = i as u32 + 1; }
    let month_of: Vec<usize> = local_hour.iter().map(|&hour| calendar::local_time(hour).month as usize - 1).collect();
    let mut hourly = Hourly { unix_mid: w.hours.iter().map(|h| h.unix_mid).collect(), poa_w_m2: vec![0.0; n], t_cell_c: vec![0.0; n], dc_w: vec![0.0; n], ac_w: vec![0.0; n], ev_wh: vec![], heating_wh: vec![], dc_cable_wh: vec![0.0; n], ev_discharge_wh: vec![], fleet_soc_pct: vec![], heat_store_kwh: vec![], cluster_unserved_wh: vec![], cluster_soc_pct: vec![], routed_cable_wh: vec![] };
    let mut warnings: Vec<Note> = vec![];
    let mut module_shading: Vec<FieldShade> = vec![];
    let mut array_irradiation = vec![];
    if w.horizon_applied { warnings.push(Note { code: "HORIZON_IN_WEATHER".into(), subject: String::new(), value: 0.0, text: "Weather beam irradiance already includes far-horizon shading; no additional horizon applied.".into() }); }
    let sun: Vec<spa::SolarPosition> = w.hours.iter().map(|h| spa::solar_position(h.unix_mid as f64, &obs, spa::delta_t_estimate(2025.0))).collect();

    // 1. Per array: plane-of-array irradiance, shading, cell temperature and module diode parameters.
    struct ArrayHours { params: Vec<Option<diode::DiodeParams>>, voc: Vec<f64>, shaded: Option<partial_shading::ShadedArray> }
    let mut arr_hours: std::collections::HashMap<String, ArrayHours> = Default::default();
    let mut dc_kwp = 0.0;
    for (ai, a) in p.arrays.iter().enumerate() {
        let module = p.library.module(&a.module_id.value).ok_or_else(|| SimError("module missing".into()))?;
        let n_mod = assigned.get(&a.id).copied().unwrap_or(0) as f64;
        if n_mod == 0.0 { continue; }
        let mut irradiation = ArrayIrradiation { array_id: a.id.clone(), name: a.name.value.clone(), ..Default::default() };
        use partial_shading::ModuleCurve;
        let module_curve = match p.simulation.pv_model {
            solarcraft_model::PvModel::SingleDiode => ModuleCurve::Single(module.desoto().ok_or_else(|| SimError("module not fitted".into()))?),
            solarcraft_model::PvModel::TwoDiode => ModuleCurve::Two(diode::fit_two_diode(&module.datasheet()).map_err(|error|
                SimError(format!("Module '{} {}' could not be fitted with the two-diode model ({error:?}).", module.manufacturer, module.model)))?),
        };
        let area = n_mod * module.area();
        let p_stc = n_mod * module.pmp_stc();
        dc_kwp += p_stc / 1000.0;
        area_tot += area;
        let iam_beam = |aoi: f64| iam::physical(aoi, 1.526, 4.0, 0.002);
        let iam_sky = iam::diffuse_average(iam_beam, a.tilt.value, true);
        let iam_gnd = if a.tilt.value > 0.0 { iam::diffuse_average(iam_beam, a.tilt.value, false) } else { 0.0 };
        let mut ah = ArrayHours { params: vec![None; n], voc: vec![0.0; n], shaded: None };
        let shade = if let Some((scene, field)) = bound_field(p, a)? {
            let routes = partial_shading::routes(field, module, &elec, &a.id);
            let (summary, diagnostic, trace) = trace_field(scene, field, a, &sun, w, routes.is_ok(), module.half_cell)?;
            match (routes, trace) {
                (Ok(routes), Some(trace)) => {
                    ah.shaded = Some(partial_shading::ShadedArray {
                        trace, routes, module: module_curve, hours: vec![partial_shading::Light::default(); n],
                        diodes: field.module.bypass_diodes,
                        forward_v: module.bypass_diode_voltage_v.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.5),
                        half_cell: module.half_cell,
                    });
                }
                (Err(reason), _) => warnings.push(Note { code: "SCENE_SHADING_APPROXIMATION".into(), subject: a.name.value.clone(),
                    value: 0.0, text: format!("{} uses field-average electrical shading because {reason}.", a.name.value) }),
                _ => {}
            }
            module_shading.push(diagnostic);
            Some(summary)
        } else if let Some(roof) = a.roof.as_ref().filter(|roof| !roof.face.obstacles.is_empty() && !roof.modules.is_empty()) {
            match partial_shading::roof_routes(module, &elec, &a.id, roof.modules.len()) {
                Ok(routes) => {
                    let diodes = module.bypass_diodes.unwrap_or(3);
                    let (summary, diagnostic, trace) = trace_roof(a, roof, &sun, w, &routes, diodes, module.half_cell);
                    module_shading.push(diagnostic);
                    ah.shaded = Some(partial_shading::ShadedArray {
                        trace, routes, module: module_curve, hours: vec![partial_shading::Light::default(); n], diodes,
                        forward_v: module.bypass_diode_voltage_v.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.5),
                        half_cell: module.half_cell,
                    });
                    Some(summary)
                }
                Err(reason) => {
                    warnings.push(Note { code: "ROOF_SHADING_APPROXIMATION".into(), subject: a.name.value.clone(),
                        value: 0.0, text: format!("{} uses roof-average electrical shading because {reason}.", a.name.value) });
                    near_shading(a, &sun)
                }
            }
        } else {
            None
        };
        for (k, h) in w.hours.iter().enumerate() {
            if ai == 0 && k % 876 == 0 && !progress(0.6 * k as f64 / n as f64) { return Err(SimError("cancelled".into())); }
            let sp = &sun[k];
            let (ghi, dni, dhi) = (h.ghi.max(0.0), h.dni.max(0.0), h.dhi.max(0.0));
            let geo = irr::Geometry { tilt: a.tilt.value, surface_azimuth: a.azimuth.value, zenith: sp.apparent_zenith, sun_azimuth: sp.azimuth };
            let doy = solarcraft_pv::day_of_year_utc(h.unix_mid as f64) as f64;
            let comp = irr::Components { ghi, dni, dhi, dni_extra: irr::dni_extra(doy, 1366.1) };
            let poa = irr::poa(model, &geo, &comp, p.site.albedo.value);
            let aoi = irr::aoi_projection(geo.tilt, geo.surface_azimuth, geo.zenith, geo.sun_azimuth).clamp(-1.0, 1.0).acos().to_degrees();
            let (beam_f, beam_geo, diff_f) = shade.as_ref().map_or((1.0, 1.0, 1.0), |s| (1.0 - s.beam_shaded[k] as f64, 1.0 - s.beam_area_shaded[k] as f64, s.diffuse_factor));
            let dir_s = poa.direct * if ah.shaded.is_some() { beam_geo } else { beam_f };
            let sky_s = poa.sky_diffuse * diff_f;
            let after_geo = poa.direct * beam_geo + sky_s + poa.ground_diffuse;
            let after_shade = dir_s + sky_s + poa.ground_diffuse;
            let beam_iam = iam_beam(aoi);
            let diffuse_iam = sky_s * iam_sky + poa.ground_diffuse * iam_gnd;
            let after_iam = dir_s * beam_iam + diffuse_iam;
            let e_eff = after_iam * f(l.soiling_pct.value);
            for (s, v) in [ghi, poa.global, after_geo, after_shade, after_iam, e_eff, e_eff].into_iter().enumerate() { irr_tot[s] += v * area; }
            let tc = temperature::faiman(poa.global, h.temp_air, h.wind.max(0.0), p.simulation.faiman_u0.value, p.simulation.faiman_u1.value);
            if let Some(shaded) = &mut ah.shaded {
                shaded.hours[k] = partial_shading::Light { direct: poa.direct * beam_iam * f(l.soiling_pct.value), diffuse: diffuse_iam * f(l.soiling_pct.value), temperature: tc };
            }
            e_tot[0] += e_eff / 1000.0 * p_stc;
            if e_eff > 0.0 {
                e_tot[1] += module_curve.at(e_eff, 25.0).mpp().2 * n_mod;
                let dp = module_curve.at(e_eff, tc);
                e_tot[2] += dp.mpp().2 * n_mod;
                ah.voc[k] = dp.voc();
                ah.params[k] = Some(dp);
            }
            monthly[month_of[k]].ghi_kwh_m2 += ghi * area / 1000.0;
            monthly[month_of[k]].poa_kwh_m2 += poa.global * area / 1000.0;
            irradiation.poa_kwh_m2[month_of[k]] += poa.global / 1000.0;
            irradiation.effective_kwh_m2[month_of[k]] += e_eff / 1000.0;
            hourly.poa_w_m2[k] += (poa.global * area) as f32;
            hourly.t_cell_c[k] += (tc * area) as f32;
        }
        if let Some(sh) = &shade {
            let text = if sh.scene {
                let treatment = match &ah.shaded {
                    Some(shaded) if shaded.half_cell => "parallel half-cell substrings with shared bypass diodes",
                    Some(_) => "string I–V curves with explicit bypass diodes",
                    None => "field-average electrical approximation",
                };
                format!("{}: near shading from the 3D scene ({} occluding objects, {} substrings per module, diffuse sky-view factor {:.3}). Electrical treatment: {treatment}.", a.name.value, sh.obstacles, sh.diodes, sh.diffuse_factor)
            } else {
                let treatment = match &ah.shaded {
                    Some(shaded) if shaded.half_cell => "parallel half-cell substrings and shared bypass diodes",
                    Some(_) => "string I–V curves and explicit bypass diodes",
                    None => "roof-average electrical approximation",
                };
                let routing = if ah.shaded.is_some() { " Installed modules and string membership follow roof placement order and electrical string lengths." } else { "" };
                format!("{}: near shading from {} roof obstacle(s), diffuse sky-view factor {:.3}. Electrical treatment: {treatment}.{routing}", a.name.value, sh.obstacles, sh.diffuse_factor)
            };
            warnings.push(Note { code: if sh.scene { "SCENE_SHADING".into() } else { "NEAR_SHADING".into() }, subject: a.name.value.clone(), value: sh.obstacles as f64, text });
        }
        arr_hours.insert(a.id.clone(), ah);
        array_irradiation.push(irradiation);
    }
    if area_tot <= 0.0 { return Err(SimError("no modules assigned to inverters".into())); }

    // 2. Per inverter unit and MPPT: tracker operating point, DC losses, inverter.
    let dc_chain = f(l.module_quality_pct.value) * f(l.lid_pct.value) * f(l.mismatch_pct.value);
    let dc_wire = wiring::Delivery { fixed_pct: l.dc_wiring_pct.value, hourly_pct: p.hourly_cables.dc_pct, availability: 1.0 };
    let ac_wire = wiring::Delivery { fixed_pct: l.ac_wiring_pct.value, hourly_pct: p.hourly_cables.ac_pct, availability: f(l.availability_pct.value) };
    let routed = wiring::Routed::new(p, &elec).map_err(SimError)?;
    hourly.routed_cable_wh = vec![vec![0.0; n]; routed.labels.len()];
    let mut unit_inputs = vec![];
    let mut unit_results = vec![];
    let mut ac_kw = 0.0;
    let mut p_independent_wh = 0.0;
    let mut nonlinear_hours = 0;
    let mut route_offsets = std::collections::HashMap::<(&str, u32), usize>::new();
    let nu = elec.inverters.len().max(1);
    for (ui, u) in elec.inverters.iter().enumerate() {
        let inv = p.library.inverter(&u.inverter_id).ok_or_else(|| SimError("inverter missing".into()))?;
        let unit_ac_wire = routed.ac_delivery(ac_wire, &u.id, inv.pac_max_w);
        ac_kw += inv.pac_max_w / 1000.0;
        let mut dc = vec![0.0f64; n];
        let mut before_wiring = vec![0.0f64; n];
        let mut ur = UnitResult { id: u.id.clone(), name: u.name.clone(), inverter_id: inv.id.clone(), ac_kw: inv.pac_max_w / 1000.0, ..Default::default() };
        // Trackers: interconnected MPPT inputs act as one tracker (parallel), the rest singly.
        let mut trackers: Vec<Vec<usize>> = u.interconnected.iter().filter(|g| !g.is_empty()).cloned().collect();
        for mi in 0..u.mppts.len() { if !trackers.iter().any(|g| g.contains(&mi)) { trackers.push(vec![mi]); } }
        trackers.sort_by_key(|g| g[0]);
        let mut vw = vec![0.0f64; n]; // Σ V·P for the power-weighted unit voltage
        for members in &trackers {
            let groups: Vec<&solarcraft_model::plant::StringGroup> = members.iter().filter_map(|&mi| u.mppts.get(mi)).flatten().collect();
            if groups.is_empty() { continue; }
            let offsets: Vec<_> = groups.iter().map(|group| {
                let start = route_offsets.entry((&group.array_id, group.modules_per_string)).or_default();
                let old = *start;
                *start += group.strings as usize;
                old
            }).collect();
            let mi = members[0];
            let circuit_links = routed.dc.get(&(u.id.clone(), mi));
            let mut mr = MpptResult { index: mi as u32 + 1, ..Default::default() };
            mr.strings = groups.iter().map(|g| format!("{} × {} ({})", g.strings, g.modules_per_string, p.arrays.iter().find(|a| a.id == g.array_id).map_or(g.array_id.clone(), |a| a.name.value.clone()))).collect::<Vec<_>>().join(" + ");
            if members.len() > 1 { mr.strings = format!("MPPT {} interconnected: {}", members.iter().map(|m| (m + 1).to_string()).collect::<Vec<_>>().join("+"), mr.strings); }
            mr.stc_kw = groups.iter().filter_map(|g| { let a = p.arrays.iter().find(|a| a.id == g.array_id)?; let m = p.library.module(&a.module_id.value)?; Some(g.strings as f64 * g.modules_per_string as f64 * m.pmp_stc() / 1000.0) }).sum();
            ur.stc_kw += mr.stc_kw;
            // Combined limits: common voltage window, summed current and power.
            let ins: Vec<_> = members.iter().map(|&m| inv.input(m)).collect();
            let v_min = ins.iter().map(|i| i.v_min).fold(f64::MIN, f64::max);
            let v_max = ins.iter().map(|i| i.v_max).fold(f64::MAX, f64::min);
            let i_max: f64 = ins.iter().map(|i| i.i_max).sum();
            let p_max: f64 = if ins.iter().all(|i| i.p_max_w.is_some()) { ins.iter().filter_map(|i| i.p_max_w).sum() } else { f64::INFINITY };
            for k in 0..n {
                if k % 2190 == 0 && !progress(0.6 + 0.3 * (ui as f64 + k as f64 / n as f64) / nu as f64) { return Err(SimError("cancelled".into())); }
                let state = |g: &solarcraft_model::plant::StringGroup| {
                    let ah = arr_hours.get(&g.array_id)?;
                    let dp = ah.params[k].as_ref()?;
                    Some(electrical::StringState { params: dp, voc_module: ah.voc[k], modules: g.modules_per_string as f64, strings: g.strings as f64 })
                };
                let resolved = |g: &solarcraft_model::plant::StringGroup| arr_hours.get(&g.array_id)
                    .and_then(|ah| ah.shaded.as_ref()).filter(|shaded| shaded.active(k));
                let t = if groups.iter().any(|group| resolved(group).is_some()) {
                    nonlinear_hours += 1;
                    let mut curves = vec![];
                    for (group, &start) in groups.iter().zip(&offsets) {
                        if let Some(shaded) = resolved(group) {
                            curves.extend(shaded.curves(k, group.modules_per_string, start, group.strings as usize));
                        } else if let Some(state) = state(group) {
                            curves.push((electrical::IvCurve::uniform(&state), state.strings));
                        }
                    }
                    electrical::operate_curves(&curves, v_min, v_max, i_max, p_max)
                } else {
                    let states: Vec<_> = groups.iter().filter_map(|&g| state(g)).collect();
                    electrical::operate_limited(&states, v_min, v_max, i_max, p_max)
                };
                p_independent_wh += t.p_independent;
                e_tot[3] += t.p_combined;
                e_tot[4] += t.p_operating;
                if t.window_limited { mr.limited_hours += 1; }
                mr.mismatch_kwh += (t.p_independent - t.p_combined) / 1000.0;
                mr.limit_loss_kwh += (t.p_combined - t.p_operating) / 1000.0;
                before_wiring[k] += t.p_operating;
                let raw = t.p_operating * dc_chain;
                let d = if let Some(links) = circuit_links {
                    let squared_current = |link: &wiring::Link| (raw / if t.v_operating > 0.0 { t.v_operating } else { link.design_voltage }).powi(2);
                    let loss: f64 = links.iter().map(|link| link.resistance * squared_current(link)).sum();
                    let scale = if loss > 0.0 { (0.45 * raw / loss).min(1.0) } else { 1.0 };
                    for link in links {
                        hourly.routed_cable_wh[link.index][k] = (link.resistance * squared_current(link) * scale) as f32;
                    }
                    raw - loss * scale
                } else { dc_wire.after_wire(raw, mr.stc_kw * 1000.0) };
                mr.dc_kwh += d / 1000.0;
                dc[k] += d;
                vw[k] += t.v_operating * d;
            }
            if mr.limited_hours > 0 { warnings.push(Note { code: "MPP_WINDOW_HOURS".into(), subject: format!("{} MPPT {}", u.name, mi + 1), value: mr.limited_hours as f64, text: format!("{} MPPT {}: {} h limited by the MPP voltage window or input current", u.name, mi + 1, mr.limited_hours) }); }
            ur.mppts.push(mr);
        }
        let v_dc: Vec<f64> = (0..n).map(|k| if dc[k] > 0.0 { vw[k] / dc[k] } else { 0.0 }).collect();
        // PV-only inverter chain for the loss ledger.
        for k in 0..n {
            let q = before_wiring[k];
            let qual = q * f(l.module_quality_pct.value);
            let lid = qual * f(l.lid_pct.value);
            let mm = lid * f(l.mismatch_pct.value);
            let dcw = dc[k];
            hourly.dc_cable_wh[k] += (mm - dcw).max(0.0) as f32;
            let vk = Some(v_dc[k]).filter(|v| *v > 0.0);
            let (unclipped, clipped) = if dcw > 0.0 { let (ac, used) = energy::inverter_ac(inv, dcw, vk); (if used > 0.0 { dcw * inv.efficiency_v(dcw, vk) } else { 0.0 }, ac) } else { (0.0, 0.0) };
            let night = if dcw > 0.0 { clipped } else { -inv.night_w };
            let acw = unit_ac_wire.after_wire(night, inv.pac_max_w);
            let avail = acw * f(l.availability_pct.value);
            for (s, v) in [qual, lid, mm, dcw, dcw, unclipped, clipped, night, acw, avail, avail].into_iter().enumerate() { e_tot[5 + s] += v; }
            ur.dc_kwh += dcw / 1000.0; ur.ac_kwh += avail.max(0.0) / 1000.0; ur.clipped_kwh += (unclipped - clipped) / 1000.0;
            monthly[month_of[k]].dc_kwh += dcw / 1000.0;
            monthly[month_of[k]].ac_kwh += avail / 1000.0;
            ur.monthly_net_ac_kwh[month_of[k]] += avail / 1000.0;
            hourly.dc_w[k] += dcw as f32;
            hourly.ac_w[k] += avail as f32;
        }
        ur.dc_ac_ratio = if ur.ac_kw > 0.0 { ur.stc_kw / ur.ac_kw } else { 0.0 };
        unit_results.push(ur);
        unit_inputs.push((u.id.clone(), inv, dc, v_dc));
    }
    for m in &mut monthly { m.ghi_kwh_m2 /= area_tot; m.poa_kwh_m2 /= area_tot; }
    for v in &mut hourly.poa_w_m2 { *v /= area_tot as f32; }
    for v in &mut hourly.t_cell_c { *v /= area_tot as f32; }

    // 3. Consumption, storage and grid.
    let profile: Vec<f64> = match (&p.loads.profile, load_series) {
        (solarcraft_model::plant::LoadProfile::None, _) => vec![0.0; 8760],
        (solarcraft_model::plant::LoadProfile::Synthetic { profile }, _) => loads::synthetic(*profile, p.loads.annual_kwh, p.site.latitude.value),
        (solarcraft_model::plant::LoadProfile::Series { .. }, Some(s)) => loads::hourly_from_series(s).map_err(SimError)?,
        (solarcraft_model::plant::LoadProfile::Series { name, .. }, None) => return Err(SimError(format!("load series '{name}' is missing from the project file"))),
    };
    let mut load_w: Vec<f64> = local_hour.iter().map(|&h| profile[h as usize % profile.len()]).collect();
    let mut reference_load = load_w.clone();
    let mut heat_store = None;
    let heating = p.heating.as_ref().map(|config| {
        let (series, reference, result) = if let Some(tank) = &p.heat_store {
            let temperatures: Vec<_> = w.hours.iter().map(|h| h.temp_air).collect();
            let run = flexible::thermal_store(config, tank, &temperatures, &local_hour, &load_w, &hourly.ac_w);
            let reference = flexible::thermal_store(config, tank, &temperatures, &local_hour, &reference_load, &vec![0.0; n]);
            heat_store = Some(run.result);
            hourly.heat_store_kwh = run.stored;
            (run.electrical, reference.electrical, run.demand)
        } else {
            let (series, result) = demand::heating(config, w.hours.iter().map(|h| h.temp_air));
            (series.clone(), series, result)
        };
        for (load, extra) in load_w.iter_mut().zip(&series) { *load += f64::from(*extra); }
        for (load, extra) in reference_load.iter_mut().zip(reference) { *load += f64::from(extra); }
        hourly.heating_wh = series;
        result
    });
    let ev = p.ev.as_ref().filter(|_| p.fleet.is_none()).map(|config| {
        let surplus: Vec<_> = hourly.ac_w.iter().zip(&load_w).map(|(&pv, &load)| (f64::from(pv) - load).max(0.0)).collect();
        let (series, result) = demand::charging(config, &local_hour, &surplus);
        for ((load, reference), extra) in load_w.iter_mut().zip(&mut reference_load).zip(&series) {
            *load += f64::from(*extra);
            *reference += f64::from(*extra);
        }
        hourly.ev_wh = series;
        result
    });
    let fleet = p.fleet.as_ref().map(|config| {
        let outage: std::collections::HashSet<_> = p.storage.iter().flat_map(|s| &s.outages)
            .flat_map(|o| (0..o.hours.min(8760)).map(|h| (o.start_hour + h) % 8760)).collect();
        let online: Vec<_> = local_hour.iter().map(|h| !outage.contains(h)).collect();
        let bidirectional = p.grid.feed_in != solarcraft_model::plant::FeedIn::Full;
        let run = flexible::fleet(config, &local_hour, &load_w, &hourly.ac_w, &online, bidirectional);
        let reference = flexible::fleet(config, &local_hour, &reference_load, &vec![0.0; n], &online, false);
        for (load, extra) in load_w.iter_mut().zip(&run.charge) { *load += f64::from(*extra); }
        for (load, extra) in reference_load.iter_mut().zip(reference.charge) { *load += f64::from(extra); }
        hourly.ev_wh = run.charge;
        hourly.ev_discharge_wh = run.discharge;
        hourly.fleet_soc_pct = run.soc;
        run.result
    });
    let units: Vec<energy::UnitInput> = unit_inputs.iter().map(|(id, spec, dc, v)| energy::UnitInput { id, spec, dc_w: dc.clone(), v_dc: v.clone() }).collect();
    let mut clusters = vec![];
    let en = if p.workflow.system.islanded() && !p.island_clusters.is_empty() {
        let run = islands::simulate(p, &units, &load_w, &local_hour, ac_wire, &routed).map_err(SimError)?;
        clusters = run.clusters;
        hourly.cluster_unserved_wh = run.unserved_wh;
        hourly.cluster_soc_pct = run.soc_pct;
        run.energy
    } else {
        energy::dispatch_bus(p, &units, &load_w, &local_hour, ac_wire, &hourly.ev_discharge_wh, &routed).map_err(SimError)?
    };
    for (&idx, series) in &en.flows.routed_ac_wh { hourly.routed_cable_wh[idx].clone_from(series); }
    let routed_cables = routed.labels.into_iter().zip(&hourly.routed_cable_wh).map(|(label, series)| wiring::CircuitResult {
        label, loss_kwh: series.iter().map(|&w| f64::from(w)).sum::<f64>() / 1000.0,
    }).collect();
    progress(0.95);

    // 4. Bills and cash flow.
    let island_finance = if p.workflow.system.islanded() { finance::island_cost(p, dc_kwp, &en.balance, &clusters) } else { None };
    let fin = if p.workflow.system.islanded() {
        warnings.push(Note { code: "ISLAND_FINANCE".into(), subject: String::new(), value: 0.0,
            text: "Off-grid results include unserved demand and generator fuel. Grid-tariff ROI is unavailable for this system.".into() });
        None
    } else {
        let battery_kwh = p.storage.as_ref().and_then(|s| p.library.battery(&s.battery_id).map(|b| b.capacity_kwh * s.units as f64)).unwrap_or(0.0);
        let zero = vec![0.0f32; n];
        let load_f32: Vec<f32> = reference_load.iter().map(|&x| x as f32).collect();
        let base_load_kwh = match &p.loads.profile {
            solarcraft_model::plant::LoadProfile::None => 0.0,
            solarcraft_model::plant::LoadProfile::Synthetic { .. } => p.loads.annual_kwh,
            solarcraft_model::plant::LoadProfile::Series { .. } => load_series.unwrap_or_default().iter().sum(),
        };
        let annual_load_kwh = base_load_kwh + ev.as_ref().map_or(0.0, |r| r.electrical_kwh)
            + heating.as_ref().map_or(0.0, |r| r.pump_electric_kwh + r.backup_kwh)
            + fleet.as_ref().map_or(0.0, |r| r.charge_kwh);
        let tariff = p.tariff.for_run(annual_load_kwh, dc_kwp);
        let without = finance::bill(&tariff, &load_f32, &zero, &local_hour);
        let with = finance::bill(&tariff, &en.flows.import, &en.flows.export, &local_hour);
        Some(finance::cash_flow(&tariff, &p.economics, &finance::Plant { dc_kwp, ac_kw, battery_kwh, pv_ac_kwh: en.balance.pv_generation_kwh }, without, with))
    };
    progress(1.0);
    if en.balance.pv_to_battery_dc_kwh > 0.0 {
        warnings.push(Note { code: "DC_BATTERY_LEDGER".into(), subject: String::new(), value: en.balance.pv_to_battery_dc_kwh,
            text: format!("The loss waterfall shows PV production as if all of it were converted by the inverters; {:.0} kWh charged the DC-coupled battery before conversion (see the energy balance).", en.balance.pv_to_battery_dc_kwh) });
    }

    let mk = |keys: &[(&str, &str)], vals: &[f64], scale: f64| -> Vec<Step> {
        let mut out: Vec<Step> = vec![];
        for (i, (k, lbl)) in keys.iter().enumerate() {
            let v = vals[i] * scale;
            let prev = if i == 0 { v } else { out[i - 1].value };
            out.push(Step { key: k.to_string(), label: lbl.to_string(), value: v, delta: v - prev, delta_pct: if prev != 0.0 { (v - prev) / prev * 100.0 } else { 0.0 } });
        }
        out
    };
    let irradiance_ledger = mk(&IRR_KEYS, &irr_tot, 1.0 / area_tot / 1000.0);
    let energy_ledger = if nonlinear_hours > 0 {
        let mut keys = E_KEYS.to_vec();
        let mut totals = e_tot.to_vec();
        keys.insert(3, ("substring_shading", "Within-string partial shading"));
        totals.insert(3, p_independent_wh);
        mk(&keys, &totals, 1.0 / 1000.0)
    } else { mk(&E_KEYS, &e_tot, 1.0 / 1000.0) };
    let mut energy_ledger = energy_ledger;
    if p.workflow.system.islanded() {
        if let Some(ac) = energy_ledger.last_mut() { ac.label = "PV energy at the AC bus".into(); }
    }
    let closure = energy_ledger[0].value + energy_ledger.iter().skip(1).map(|s| s.delta).sum::<f64>() - energy_ledger.last().unwrap().value;
    let ac_kwh = energy_ledger.last().unwrap().value;
    let poa = irradiance_ledger[1].value;
    Ok(SimResult {
        manifest: RunManifest { inputs_hash, engine: ENGINE_VERSION.into(), weather_hash: weather_hash.into(), weather_provider: w.provenance.provider.clone(), transposition: format!("{:?}", p.simulation.transposition.value), pv_model: p.simulation.pv_model, hours: n, ledger_closure_error_kwh: closure },
        annual: Annual { dc_kwp, ac_kw, ghi_kwh_m2: irradiance_ledger[0].value, poa_kwh_m2: poa, ac_kwh, specific_yield_kwh_kwp: ac_kwh / dc_kwp, performance_ratio: ac_kwh / (poa * dc_kwp) },
        electrical: unit_results, array_irradiation, flows: en.flows, balance: en.balance, finance: fin, island_finance, ev, heating, fleet, heat_store, clusters, routed_cables, local_hour,
        irradiance_ledger, energy_ledger, monthly, hourly, warnings, module_shading,
    })
}

/// Fingerprint of everything that determines a result.
pub fn inputs_hash(p: &Project, weather_hash: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let mut design = p.design_snapshot();
    if let Some(fields) = design.as_object_mut() {
        fields.remove("cable_plan"); // The simulation reads only applied loss percentages.
        if p.workflow.system == solarcraft_model::workflow::SystemType::Custom {
            fields.remove("workflow");
        } else if let Some(workflow) = fields.get_mut("workflow").and_then(serde_json::Value::as_object_mut) {
            workflow.remove("tools");
        }
    }
    h.update(serde_json::to_vec(&design).unwrap_or_default());
    h.update(weather_hash.as_bytes());
    h.update(ENGINE_VERSION.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Near shading of an array's modules by its roof obstacles.
pub struct NearShading {
    /// Shaded fraction of the beam on the modules, per hour (electrical: with bypass diodes).
    pub beam_shaded: Vec<f32>,
    /// Geometric shaded fraction (share of module area), per hour.
    pub beam_area_shaded: Vec<f32>,
    pub scene: bool,
    pub diodes: u32,
    /// Fraction of isotropic sky diffuse still reaching the modules.
    pub diffuse_factor: f64,
    pub obstacles: usize,
}

/// Ray-traced near shading for arrays with a roof layout and obstacles
/// (3 × 6 samples per module; deterministic cosine-weighted hemisphere for diffuse).
pub fn near_shading(a: &solarcraft_model::Array, sun: &[spa::SolarPosition]) -> Option<NearShading> {
    use solarcraft_geometry::roof3d::{obstacle_triangles, Frame};
    use solarcraft_shading::{sun_dir, Bvh, Triangle};
    let roof = a.roof.as_ref()?;
    if roof.face.obstacles.is_empty() || roof.modules.is_empty() { return None; }
    let frame = Frame::new(a.tilt.value, a.azimuth.value, 3.0);
    let tris: Vec<Triangle> = obstacle_triangles(&roof.face, &frame).into_iter().map(|(t, i)| Triangle::opaque(t[0], t[1], t[2], i as u32 + 1)).collect();
    let bvh = Bvh::from_triangles(&tris);
    let mut samples = vec![];
    for m in &roof.modules {
        for i in 0..3 { for j in 0..6 {
            let p = [m.x0 + (m.x1 - m.x0) * (i as f64 + 0.5) / 3.0, m.y0 + (m.y1 - m.y0) * (j as f64 + 0.5) / 6.0];
            samples.push(frame.above(p, 0.05));
        } }
    }
    let n = frame.n;
    let beam_shaded: Vec<f32> = sun.iter().map(|sp| {
        if sp.apparent_elevation <= 0.0 { return 0.0; }
        let d = sun_dir(sp.apparent_elevation, sp.azimuth);
        if d[0] * n[0] + d[1] * n[1] + d[2] * n[2] <= 0.0 { return 0.0; }
        samples.iter().filter(|&&o| bvh.occluded(o, d, 0)).count() as f32 / samples.len() as f32
    }).collect();
    // Cosine-weighted directions around the module normal; only sky directions (z > 0) count.
    let (t1, t2) = { let up = if n[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let c = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let norm = |v: [f64; 3]| { let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(); [v[0] / l, v[1] / l, v[2] / l] };
        let t1 = norm(c(up, n)); (t1, c(n, t1)) };
    let (nr, nphi) = (12usize, 24usize);
    let probe: Vec<_> = samples.iter().step_by(6).copied().collect();
    let (mut sky, mut visible) = (0usize, 0usize);
    for ir in 0..nr { for ip in 0..nphi {
        let r = ((ir as f64 + 0.5) / nr as f64).sqrt();
        let phi = (ip as f64 + 0.5) / nphi as f64 * std::f64::consts::TAU;
        let (x, y, z) = (r * phi.cos(), r * phi.sin(), (1.0 - r * r).sqrt());
        let d = [0, 1, 2].map(|k| t1[k] * x + t2[k] * y + n[k] * z);
        if d[2] <= 0.0 { continue; }
        for &o in &probe { sky += 1; if !bvh.occluded(o, d, 0) { visible += 1; } }
    } }
    let diffuse_factor = if sky > 0 { visible as f64 / sky as f64 } else { 1.0 };
    Some(NearShading { beam_area_shaded: beam_shaded.clone(), beam_shaded, diffuse_factor, obstacles: roof.face.obstacles.len(), scene: false, diodes: 0 })
}

/// Shading of an array from the 3D scene field bound to it (module samples per
/// bypass-diode substring, every scene object and module as occluder).
pub fn scene_shading(p: &Project, a: &solarcraft_model::Array, sun: &[spa::SolarPosition], w: &Weather) -> Result<Option<(NearShading, FieldShade)>, SimError> {
    let Some((scene, field)) = bound_field(p, a)? else { return Ok(None) };
    let (summary, diagnostic, _) = trace_field(scene, field, a, sun, w, false, false)?;
    Ok(Some((summary, diagnostic)))
}

fn bound_field<'a>(p: &'a Project, a: &solarcraft_model::Array) -> Result<Option<(&'a solarcraft_scene::Scene, &'a solarcraft_scene::place::ModuleField)>, SimError> {
    let Some(sc) = &p.scene else { return Ok(None) };
    let mut fields = sc.fields.iter().filter(|f| f.array_id.as_deref() == Some(a.id.as_str()) && !f.modules.is_empty());
    let Some(f) = fields.next() else { return Ok(None) };
    if fields.next().is_some() {
        return Err(SimError(format!("Array {} has multiple scene fields. Assign each field to a separate array.", a.id)));
    }
    Ok(Some((sc, f)))
}

fn trace_roof(a: &solarcraft_model::Array, roof: &solarcraft_model::RoofLayout, sun: &[spa::SolarPosition],
    weather: &Weather, routes: &std::collections::BTreeMap<u32, Vec<Vec<usize>>>, diodes: u32, half_cell: bool)
    -> (NearShading, FieldShade, solarcraft_scene::shade::SubstringTrace)
{
    use solarcraft_geometry::{layout::Orientation, roof3d::{Frame, obstacle_triangles}};
    use solarcraft_scene::{place::{ModuleDims, ModuleField, Mount, PlacedModule}, shade::shade_planar_substrings};
    use solarcraft_shading::Triangle;

    let frame = Frame::new(a.tilt.value, a.azimuth.value, 3.0);
    let triangles: Vec<_> = obstacle_triangles(&roof.face, &frame).into_iter()
        .map(|(t, i)| Triangle::opaque(t[0], t[1], t[2], i as u32 + 1)).collect();
    let installed = routes.values().flatten().map(Vec::len).sum();
    let mut modules: Vec<_> = roof.modules.iter().take(installed).enumerate().map(|(i, m)| PlacedModule {
        id: format!("{}-roof-{}", a.id, i + 1),
        corners: [[m.x0, m.y0], [m.x1, m.y0], [m.x1, m.y1], [m.x0, m.y1]].map(|p| frame.above(p, 0.03)),
        normal: frame.n, row: 0, col: 0, portrait: roof.orientation == Orientation::Portrait, string: None,
    }).collect();
    for (i, route) in routes.values().flatten().enumerate() {
        for &m in route { modules[m].string = Some(i as u32); }
    }
    let field = ModuleField { id: format!("{}-roof", a.id), name: a.name.value.clone(), array_id: Some(a.id.clone()),
        mount: Mount::Roof { face: String::new(), standoff: 0.03 },
        module: ModuleDims { bypass_diodes: diodes, ..Default::default() }, layout: Default::default(),
        modules, strings: vec![], inverter_at: None, source: None };
    let (suns, weights) = shade_inputs(sun, weather);
    let (summary, trace) = shade_planar_substrings(&field, &triangles, &suns, &weights, half_cell);
    let (near, diagnostic) = shade_summary(summary, a, &weights, roof.face.obstacles.len(), false, diodes, true, half_cell);
    (near, diagnostic, trace)
}

fn shade_inputs(sun: &[spa::SolarPosition], weather: &Weather) -> (Vec<(f64, f64)>, Vec<f64>) {
    let suns = sun.iter().map(|s| (s.apparent_elevation, s.azimuth)).collect();
    let weights = weather.hours.iter().map(|h| h.dni.max(0.0)).collect();
    (suns, weights)
}

fn trace_field(sc: &solarcraft_scene::Scene, f: &solarcraft_scene::place::ModuleField, a: &solarcraft_model::Array,
    sun: &[spa::SolarPosition], w: &Weather, nonlinear: bool, half_cell: bool)
    -> Result<(NearShading, FieldShade, Option<solarcraft_scene::shade::SubstringTrace>), SimError>
{
    use solarcraft_scene::shade::{shade_field, shade_field_substrings, ShadeOptions};
    let (suns, weights) = shade_inputs(sun, w);
    let opts = ShadeOptions::default();
    let (fs, trace) = if nonlinear {
        shade_field_substrings(sc, &f.id, &suns, &weights, &opts, half_cell).map(|(summary, trace)| (summary, Some(trace)))
    } else {
        shade_field(sc, &f.id, &suns, &weights, &opts).map(|summary| (summary, None))
    }.map_err(|err| SimError(format!("Shading field {}: {err}", f.id)))?;
    let obstacles = sc.objects.iter().filter(|o| !o.hidden).count();
    let (near, field) = shade_summary(fs, a, &weights, obstacles, true, f.module.bypass_diodes, nonlinear, half_cell);
    Ok((near, field, trace))
}

fn shade_summary(fs: solarcraft_scene::shade::FieldShading, array: &solarcraft_model::Array, weights: &[f64],
    obstacles: usize, scene: bool, diodes: u32, nonlinear: bool, half_cell: bool) -> (NearShading, FieldShade)
{
    let wsum: f64 = weights.iter().sum::<f64>().max(1e-9);
    let loss = |v: &[f32]| v.iter().zip(weights).map(|(x, w)| (1.0 - *x as f64) * w).sum::<f64>() / wsum * 100.0;
    let field = FieldShade { field: fs.field, array_id: array.id.clone(), diffuse_factor: fs.diffuse_factor,
        beam_loss_pct: loss(&fs.beam_bypass), beam_area_loss_pct: loss(&fs.beam_area), traced_directions: fs.traced_directions,
        modules: fs.modules, nonlinear, half_cell: nonlinear && half_cell };
    (NearShading { beam_shaded: fs.beam_bypass.iter().map(|x| 1.0 - x).collect(),
        beam_area_shaded: fs.beam_area.iter().map(|x| 1.0 - x).collect(), diffuse_factor: fs.diffuse_factor,
        obstacles, scene, diodes }, field)
}
