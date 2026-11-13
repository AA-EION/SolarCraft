// SPDX-License-Identifier: GPL-3.0-or-later
//! Annual grid-connected PV simulation (slice 1).
//!
//! Every loss is the difference between two consecutive stages of one chain
//! of hourly calculations, so the loss waterfall closes exactly (checked).
//! Stages: horizontal → plane of array → IAM → soiling → effective
//! irradiance; nominal STC energy → irradiance level → temperature → MPP
//! voltage window → module quality/LID → mismatch → DC wiring → inverter
//! efficiency → clipping → night tare → AC wiring → availability → grid.
use serde::{Deserialize, Serialize};
use solarcraft_model::validate::{validate, Severity};
use solarcraft_model::{Project, TranspositionModel};
use solarcraft_pv::{diode, iam, inverter, irradiance as irr, spa, temperature};
use solarcraft_weather::Weather;

pub const ENGINE_VERSION: &str = concat!("solarcraft-engine ", env!("CARGO_PKG_VERSION"));

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
pub struct RunManifest { pub engine: String, pub weather_hash: String, pub weather_provider: String, pub transposition: String, pub hours: usize, pub ledger_closure_error_kwh: f64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hourly { pub unix_mid: Vec<i64>, pub poa_w_m2: Vec<f32>, pub t_cell_c: Vec<f32>, pub dc_w: Vec<f32>, pub ac_w: Vec<f32> }

/// A result note with a stable code (translated by reports/UI) and English text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note { pub code: String, pub subject: String, pub value: f64, pub text: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimResult {
    pub manifest: RunManifest,
    pub annual: Annual,
    /// Irradiance chain (kWh/m², area-weighted over arrays).
    pub irradiance_ledger: Vec<Step>,
    /// Energy chain (kWh).
    pub energy_ledger: Vec<Step>,
    pub monthly: Vec<Month>,
    pub hourly: Hourly,
    pub warnings: Vec<Note>,
}

#[derive(Debug)]
pub struct SimError(pub String);
impl std::fmt::Display for SimError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) } }
impl std::error::Error for SimError {}

const IRR_KEYS: [(&str, &str); 6] = [
    ("ghi", "Global horizontal irradiation"), ("poa", "Global incident in plane (transposition)"), ("shading", "After near shading (obstacles)"),
    ("iam", "After incidence angle (reflection)"), ("soiling", "After soiling"), ("effective", "Effective irradiation on modules"),
];
const E_KEYS: [(&str, &str); 15] = [
    ("nominal", "Nominal energy at STC efficiency"), ("irradiance_level", "Irradiance level"), ("temperature", "Module temperature"),
    ("mpp_window", "Inverter MPP voltage window"), ("quality", "Module quality"), ("lid", "Light-induced degradation"),
    ("mismatch", "Mismatch"), ("dc_wiring", "DC wiring"), ("dc_out", "Array DC output"),
    ("inverter", "Inverter efficiency"), ("clipping", "Inverter clipping (power limit)"), ("night", "Inverter night consumption"),
    ("ac_wiring", "AC wiring"), ("availability", "Availability"), ("grid", "Energy to grid"),
];

/// `progress(fraction) -> continue?`
pub fn simulate(p: &Project, w: &Weather, weather_hash: &str, mut progress: impl FnMut(f64) -> bool) -> Result<SimResult, SimError> {
    let errors: Vec<_> = validate(p).into_iter().filter(|i| i.severity == Severity::Error).collect();
    if !errors.is_empty() {
        return Err(SimError(format!("project has errors: {}", errors.iter().map(|e| format!("{} ({})", e.message, e.code)).collect::<Vec<_>>().join("; "))));
    }
    let n = w.hours.len();
    let obs = spa::Observer::new(p.site.latitude.value, p.site.longitude.value, if w.elevation.is_finite() { w.elevation } else { p.site.elevation.value });
    let model = match p.simulation.transposition.value {
        TranspositionModel::Isotropic => irr::SkyModel::Isotropic, TranspositionModel::HayDavies => irr::SkyModel::HayDavies,
        TranspositionModel::Klucher => irr::SkyModel::Klucher, TranspositionModel::Reindl => irr::SkyModel::Reindl, TranspositionModel::Perez => irr::SkyModel::Perez,
    };
    let l = &p.losses;
    let f = |x: f64| 1.0 - x / 100.0;
    let mut irr_tot = [0.0f64; 6]; // area-weighted Wh/m²·m²
    let mut e_tot = [0.0f64; 15];
    let mut area_tot = 0.0;
    let mut monthly = vec![Month::default(); 12];
    for (i, m) in monthly.iter_mut().enumerate() { m.month = i as u32 + 1; }
    let mut hourly = Hourly { unix_mid: w.hours.iter().map(|h| h.unix_mid).collect(), poa_w_m2: vec![0.0; n], t_cell_c: vec![0.0; n], dc_w: vec![0.0; n], ac_w: vec![0.0; n] };
    let (mut dc_kwp, mut ac_kw) = (0.0, 0.0);
    let mut warnings: Vec<Note> = vec![];
    if w.horizon_applied { warnings.push(Note { code: "HORIZON_IN_WEATHER".into(), subject: String::new(), value: 0.0, text: "Weather beam irradiance already includes far-horizon shading; no additional horizon applied.".into() }); }

    // Solar position once per hour (shared by arrays).
    let sun: Vec<spa::SolarPosition> = w.hours.iter().map(|h| spa::solar_position(h.unix_mid as f64, &obs, spa::delta_t_estimate(2025.0))).collect();

    for (ai, a) in p.arrays.iter().enumerate() {
        let module = p.library.module(&a.module_id.value).ok_or_else(|| SimError("module missing".into()))?;
        let params = module.desoto().ok_or_else(|| SimError("module not fitted".into()))?;
        let inv = p.library.inverter(&a.inverter_id.value).ok_or_else(|| SimError("inverter missing".into()))?;
        let ns = a.modules_per_string.value as f64;
        let n_mod = ns * a.strings.value as f64;
        let n_inv = a.inverter_count.value.max(1) as f64;
        let area = n_mod * module.area();
        let p_stc = n_mod * module.pmp_stc();
        dc_kwp += p_stc / 1000.0;
        ac_kw += inv.pac_max_w * n_inv / 1000.0;
        area_tot += area;
        let iam_beam = |aoi: f64| iam::physical(aoi, 1.526, 4.0, 0.002);
        let iam_sky = iam::diffuse_average(iam_beam, a.tilt.value, true);
        let iam_gnd = if a.tilt.value > 0.0 { iam::diffuse_average(iam_beam, a.tilt.value, false) } else { 0.0 };
        let shade = near_shading(a, &sun);
        let mut window_hours = 0usize;
        for (k, h) in w.hours.iter().enumerate() {
            if ai == 0 && k % 876 == 0 && !progress(k as f64 / n as f64) { return Err(SimError("cancelled".into())); }
            let sp = &sun[k];
            let (ghi, dni, dhi) = (h.ghi.max(0.0), h.dni.max(0.0), h.dhi.max(0.0));
            let geo = irr::Geometry { tilt: a.tilt.value, surface_azimuth: a.azimuth.value, zenith: sp.apparent_zenith, sun_azimuth: sp.azimuth };
            let doy = solarcraft_pv::day_of_year_utc(h.unix_mid as f64) as f64;
            let comp = irr::Components { ghi, dni, dhi, dni_extra: irr::dni_extra(doy, 1366.1) };
            let poa = irr::poa(model, &geo, &comp, p.site.albedo.value);
            let aoi = irr::aoi_projection(geo.tilt, geo.surface_azimuth, geo.zenith, geo.sun_azimuth).clamp(-1.0, 1.0).acos().to_degrees();
            let (beam_f, diff_f) = shade.as_ref().map_or((1.0, 1.0), |s| (1.0 - s.beam_shaded[k] as f64, s.diffuse_factor));
            let (dir_s, sky_s) = (poa.direct * beam_f, poa.sky_diffuse * diff_f);
            let after_shade = dir_s + sky_s + poa.ground_diffuse;
            let after_iam = dir_s * iam_beam(aoi) + sky_s * iam_sky + poa.ground_diffuse * iam_gnd;
            let after_soil = after_iam * f(l.soiling_pct.value);
            let e_eff = after_soil;
            let irr_stages = [ghi, poa.global, after_shade, after_iam, after_soil, e_eff];
            for s in 0..6 { irr_tot[s] += irr_stages[s] * area; }

            let tc = temperature::faiman(poa.global, h.temp_air, h.wind.max(0.0), p.simulation.faiman_u0.value, p.simulation.faiman_u1.value);
            let nominal = e_eff / 1000.0 * p_stc;
            let (dc25, dcm, dcw) = if e_eff > 0.0 {
                let d25 = diode::calcparams_desoto(e_eff, 25.0, &params).mpp().2 * n_mod;
                let dp = diode::calcparams_desoto(e_eff, tc, &params);
                let (vmp, _, pmp) = dp.mpp();
                let v_string = vmp * ns;
                // Inverter tracks within its MPPT window; outside, it operates at the bound.
                let pw = if v_string < inv.mppt_v_min || v_string > inv.mppt_v_max {
                    window_hours += 1;
                    let vb = v_string.clamp(inv.mppt_v_min, inv.mppt_v_max) / ns;
                    (vb * dp.current(vb)).max(0.0)
                } else { pmp };
                (d25, pmp * n_mod, pw * n_mod)
            } else { (0.0, 0.0, 0.0) };
            let q = dcw * f(l.module_quality_pct.value);
            let lid = q * f(l.lid_pct.value);
            let mm = lid * f(l.mismatch_pct.value);
            let dcwire = mm * f(l.dc_wiring_pct.value);
            let pdc_inv = dcwire / n_inv;
            let (ac_unclipped, ac_clipped) = if pdc_inv > 0.0 {
                let eff_ac = inverter::pvwatts_unclipped(pdc_inv, inv.pdc0_w(), inv.eta_nominal, 0.9637);
                (eff_ac * n_inv, eff_ac.min(inv.pac_max_w) * n_inv)
            } else { (0.0, 0.0) };
            let night = if pdc_inv > 0.0 { ac_clipped } else { -inv.night_w * n_inv };
            let acw = if night > 0.0 { night * f(l.ac_wiring_pct.value) } else { night };
            let avail = acw * f(l.availability_pct.value);
            let e_stages = [nominal, dc25, dcm, dcw, q, lid, mm, dcwire, dcwire, ac_unclipped, ac_clipped, night, acw, avail, avail];
            for s in 0..15 { e_tot[s] += e_stages[s]; }
            let mo = solarcraft_pv::civil_from_days(h.unix_mid.div_euclid(86400)).1 as usize - 1;
            monthly[mo].ghi_kwh_m2 += ghi * area / 1000.0;
            monthly[mo].poa_kwh_m2 += poa.global * area / 1000.0;
            monthly[mo].dc_kwh += dcwire / 1000.0;
            monthly[mo].ac_kwh += avail / 1000.0;
            hourly.poa_w_m2[k] += (poa.global * area) as f32;
            hourly.t_cell_c[k] = tc as f32;
            hourly.dc_w[k] += dcwire as f32;
            hourly.ac_w[k] += avail as f32;
        }
        if let Some(sh) = &shade {
            warnings.push(Note { code: "NEAR_SHADING".into(), subject: a.name.value.clone(), value: sh.obstacles as f64,
                text: format!("{}: near shading from {} roof obstacle(s) included (beam ray-traced hourly, diffuse sky-view factor {:.3}); electrical mismatch from partial shading is not yet modelled.", a.name.value, sh.obstacles, sh.diffuse_factor) });
        }
        if window_hours > 0 { warnings.push(Note { code: "MPP_WINDOW_HOURS".into(), subject: a.name.value.clone(), value: window_hours as f64, text: format!("{}: {window_hours} h outside the inverter MPP voltage window", a.name.value) }); }
    }
    progress(1.0);
    if area_tot <= 0.0 { return Err(SimError("no modules".into())); }
    for m in &mut monthly { m.ghi_kwh_m2 /= area_tot; m.poa_kwh_m2 /= area_tot; }
    for v in &mut hourly.poa_w_m2 { *v /= area_tot as f32; }

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
    let energy_ledger = mk(&E_KEYS, &e_tot, 1.0 / 1000.0);
    let closure = energy_ledger[0].value + energy_ledger.iter().skip(1).map(|s| s.delta).sum::<f64>() - energy_ledger.last().unwrap().value;
    let ac_kwh = energy_ledger.last().unwrap().value;
    let poa = irradiance_ledger[1].value;
    Ok(SimResult {
        manifest: RunManifest { engine: ENGINE_VERSION.into(), weather_hash: weather_hash.into(), weather_provider: w.provenance.provider.clone(), transposition: format!("{:?}", p.simulation.transposition.value), hours: n, ledger_closure_error_kwh: closure },
        annual: Annual { dc_kwp, ac_kw, ghi_kwh_m2: irradiance_ledger[0].value, poa_kwh_m2: poa, ac_kwh, specific_yield_kwh_kwp: ac_kwh / dc_kwp, performance_ratio: ac_kwh / (poa * dc_kwp) },
        irradiance_ledger, energy_ledger, monthly, hourly, warnings,
    })
}


/// Near shading of an array's modules by its roof obstacles.
pub struct NearShading {
    /// Shaded fraction of the beam on the modules, per hour.
    pub beam_shaded: Vec<f32>,
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
    let tris: Vec<Triangle> = obstacle_triangles(&roof.face, &frame).into_iter().map(|(t, i)| Triangle { a: t[0], b: t[1], c: t[2], owner: i as u32 + 1 }).collect();
    let bvh = Bvh::from_triangles(&tris);
    let mut samples = vec![];
    for m in &roof.modules {
        for i in 0..3 { for j in 0..6 {
            let p = [m.x0 + (m.x1 - m.x0) * (i as f64 + 0.5) / 3.0, m.y0 + (m.y1 - m.y0) * (j as f64 + 0.5) / 6.0];
            samples.push(frame.above(p, 0.05));
        } }
    }
    let n = frame.n;
    let beam_shaded = sun.iter().map(|sp| {
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
    Some(NearShading { beam_shaded, diffuse_factor, obstacles: roof.face.obstacles.len() })
}
