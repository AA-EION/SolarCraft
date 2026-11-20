// SPDX-License-Identifier: GPL-3.0-or-later
//! Design validation. Each issue has a stable code, severity and field pointer
//! so the UI, CLI and MCP report the same thing.
use crate::derive::{cold_open_circuit_voltage, module_voltages, T_CELL_MAX, T_CELL_MIN};
use crate::Project;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity { Error, Warning, Info }

#[derive(Debug, Clone, Serialize)]
pub struct Issue { pub code: &'static str, pub severity: Severity, pub pointer: String, pub message: String }

pub fn validate(p: &Project) -> Vec<Issue> {
    let active = crate::workflow::Workflow::active(p);
    validate_active(&active)
}

pub fn validate_active(p: &Project) -> Vec<Issue> {
    let mut v = vec![];
    let mut push = |code, severity, pointer: String, message: String| v.push(Issue { code, severity, pointer, message });
    if !(-90.0..=90.0).contains(&p.site.latitude.value) { push("SITE_LAT_RANGE", Severity::Error, "/site/latitude".into(), "Latitude must be between −90 and 90°.".into()); }
    if !(-180.0..=180.0).contains(&p.site.longitude.value) { push("SITE_LON_RANGE", Severity::Error, "/site/longitude".into(), "Longitude must be between −180 and 180°.".into()); }
    if !(0.0..=1.0).contains(&p.site.albedo.value) { push("SITE_ALBEDO_RANGE", Severity::Error, "/site/albedo".into(), "Albedo must be between 0 and 1.".into()); }
    if p.weather.is_none() { push("WEATHER_MISSING", Severity::Error, "/weather".into(), "No weather data. Fetch PVGIS data or import a weather file.".into()); }
    if p.simulation.cold_voltage_temperature_c.is_some_and(|t| !(-100.0..=25.0).contains(&t)) {
        push("COLD_VOLTAGE_TEMPERATURE", Severity::Error, "/simulation/cold_voltage_temperature_c".into(),
            "Cold voltage test temperature must be between −100 and 25 °C.".into());
    }
    let archived_arrays = p.arrays.iter().any(|a| a.archived_shading.is_some());
    let archived_counts = archived_arrays.then(|| crate::derive::modules_by_array(&crate::derive::electrical(p)));
    for (i, a) in p.arrays.iter().enumerate() {
        let base = format!("/arrays/{i}");
        if !(0.0..=90.0).contains(&a.tilt.value) { push("ARRAY_TILT_RANGE", Severity::Error, format!("{base}/tilt"), "Tilt must be between 0 and 90°.".into()); }
        if !(0.0..360.0).contains(&a.azimuth.value) { push("ARRAY_AZIMUTH_RANGE", Severity::Error, format!("{base}/azimuth"), "Azimuth must be in [0, 360)°, clockwise from north.".into()); }
        if a.site_shade.as_ref().is_some_and(|shade| !shade.valid()) {
            push("ARRAY_SITE_SHADE", Severity::Error, format!("{base}/site_shade"), "Check the uniform shading loss or the 360 horizon elevations.".into());
        }
        if let Some(tracker) = a.tracking {
            if !tracker.valid() {
                push("ARRAY_TRACKING", Severity::Error, format!("{base}/tracking"), "Check the tracker axis, travel and ground coverage values.".into());
            }
            if a.roof.is_some() || p.scene.as_ref().is_some_and(|scene| scene.fields.iter().any(|field| field.array_id.as_deref() == Some(a.id.as_str()))) {
                push("TRACKER_STATIC_GEOMETRY", Severity::Warning, format!("{base}/tracking"), "The tracker moves during simulation. Fixed roof and scene shading are skipped for this array.".into());
            }
        }
        if a.site_shade.as_ref().is_some_and(|shade| !shade.valid()) {
            push("SITE_SHADE_INVALID", Severity::Error, format!("{base}/site_shade"),
                "Site shading needs a loss fraction between zero and one or 360 horizon angles between 0 and 90°.".into());
        }
        if let Some(rows) = a.bifacial_rows {
            if !rows.valid(a.tilt.value) {
                push("BIFACIAL_ROWS_INVALID", Severity::Error, format!("{base}/bifacial_rows"),
                    "Bifacial rows require valid geometry and pitch at least as large as the projected row width.".into());
            } else if a.tracking.is_some() {
                push("BIFACIAL_ROWS_TRACKING", Severity::Warning, format!("{base}/bifacial_rows"),
                    "The independent rear irradiance model uses fixed rows. Rear irradiance for this tracker is omitted.".into());
            }
        }
        if let Some(shade) = &a.archived_shading {
            let wired = archived_counts.as_ref().and_then(|counts| counts.get(&a.id)).copied().unwrap_or_default();
            if !shade.valid() {
                push("ARCHIVED_SHADE_INVALID", Severity::Error, format!("{base}/archived_shading"), "The saved shade data requires 8,760 hourly fractions and a valid orientation.".into());
            } else if !shade.applies_to(a, &p.site, wired) {
                push("ARCHIVED_SHADE_INPUTS", Severity::Warning, format!("{base}/archived_shading"), "Saved 3D shading is skipped when the site, array orientation, module or wired module count differs from the archive.".into());
            }
        }
        let Some(m) = p.library.module(&a.module_id.value) else { push("MODULE_MISSING", Severity::Error, format!("{base}/module_id"), format!("Module '{}' is not in the project library.", a.module_id.value)); continue };
        if let Some(mp) = &m.model_params {
            if !mp.fit_acceptable { push("MODULE_FIT_APPROX", Severity::Warning, format!("{base}/module_id"), format!("Module datasheet could only be fitted approximately (Pmp error {:.2} %, γ error {:.3} %/K). Check the datasheet values.", mp.pmp_rel_err * 100.0, mp.gamma_abs_err)); }
        } else { push("MODULE_NOT_FITTED", Severity::Error, format!("{base}/module_id"), "Module has no fitted electrical model.".into()); }
        if let Some(roof) = &a.roof {
            let roof_kwp = roof.modules.len() as f64 * m.pmp_stc() / 1000.0;
            if a.target_kwp.is_user() && a.target_kwp.value > roof_kwp + 1e-6 {
                push("ROOF_LIMITS_SIZE", Severity::Warning, format!("{base}/target_kwp"), format!("The roof fits {} modules ({roof_kwp:.2} kWp); the requested {:.2} kWp cannot be reached.", roof.modules.len(), a.target_kwp.value));
            }
        }
    }
    electrical_issues(p, &mut push);
    plant_issues(p, &mut push);
    scene_issues(p, &mut push);
    v
}

/// Scene geometry and the binding of module fields to electrical arrays.
fn scene_issues(p: &Project, push: &mut impl FnMut(&'static str, Severity, String, String)) {
    let Some(sc) = &p.scene else { return };
    for i in solarcraft_scene::validate(sc) {
        let sev = match i.severity.as_str() { "error" => Severity::Error, "warning" => Severity::Warning, _ => Severity::Info };
        push(solarcraft_scene::static_code(&i.code), sev, format!("/scene/{}", i.subject), i.message);
    }
    let electrical = crate::derive::electrical(p);
    let assigned = crate::derive::modules_by_array(&electrical);
    for a in &p.arrays {
        let routes = sc.fields.iter().filter(|f| f.array_id.as_deref() == Some(&a.id)).flat_map(|f| &f.strings);
        if let Err(reason) = crate::plant::check_mppt_bindings(&electrical, &a.id, routes) {
            push("SCENE_MPPT_BINDING", Severity::Error, "/scene/fields".into(), format!("{}: {reason}.", a.name.value));
        }
    }
    let mut placed = std::collections::HashMap::new();
    for f in &sc.fields {
        if let Some(aid) = &f.array_id { *placed.entry(aid.as_str()).or_insert(0) += f.modules.len(); }
    }
    for (k, f) in sc.fields.iter().enumerate() {
        let Some(aid) = &f.array_id else { continue };
        let ptr = format!("/scene/fields/{k}");
        let Some(a) = p.arrays.iter().find(|a| &a.id == aid) else { push("SCENE_FIELD_ARRAY", Severity::Error, ptr, format!("Field {} feeds array {aid}, which does not exist.", f.id)); continue };
        if f.modules.is_empty() { continue; }
        let strung = assigned.get(aid).copied().unwrap_or(0) as usize;
        let scene_count = placed[aid.as_str()];
        if strung != scene_count {
            push("SCENE_FIELD_COUNT", Severity::Warning, ptr.clone(), format!("The scene holds {scene_count} modules for array {} with {strung} wired modules. Check the bound fields and wiring.", a.name.value));
        }
        if let Some((t, az)) = sc.field_orientation(f) {
            let daz = ((az - a.azimuth.value + 540.0) % 360.0 - 180.0).abs();
            if (t - a.tilt.value).abs() > 1.0 || daz > 1.0 {
                push("SCENE_FIELD_ORIENTATION", Severity::Warning, ptr.clone(), format!("Field {} faces tilt {t:.1}° / azimuth {az:.1}° but array {} uses {:.1}° / {:.1}°; bind the field again to copy its orientation.", f.id, a.name.value, a.tilt.value, a.azimuth.value));
            }
        }
        if let Some(m) = p.library.module(&a.module_id.value) {
            let (l, w) = (m.length_m.max(m.width_m), m.length_m.min(m.width_m));
            if (l - f.module.length).abs() > 0.02 || (w - f.module.width).abs() > 0.02 {
                push("SCENE_FIELD_MODULE_SIZE", Severity::Warning, ptr, format!("Field {} was laid out for {:.3} × {:.3} m modules; {} is {l:.3} × {w:.3} m. Place the modules again.", f.id, f.module.length, f.module.width, m.model));
            }
        }
    }
}

type Push<'a> = dyn FnMut(&'static str, Severity, String, String) + 'a;

/// Checks on the resolved inverter/MPPT stringing (manual or derived).
fn electrical_issues(p: &Project, push: &mut Push) {
    let e = crate::derive::electrical(p);
    let test_temperature = p.simulation.cold_voltage_temperature_c.unwrap_or(T_CELL_MIN);
    let manual = p.electrical.manual && !p.electrical.inverters.is_empty();
    let base_of = |u: usize| if manual { format!("/electrical/inverters/{u}") } else { "/arrays".to_string() };
    if e.inverters.is_empty() && !p.arrays.is_empty() { push("INVERTER_MISSING", Severity::Error, "/arrays/0/inverter_id".into(), "No inverter selected.".into()); }
    for (ui, u) in e.inverters.iter().enumerate() {
        let base = base_of(ui);
        let Some(inv) = p.library.inverter(&u.inverter_id) else { push("INVERTER_MISSING", Severity::Error, format!("{base}/inverter_id"), format!("{}: inverter '{}' is not in the project library.", u.name, u.inverter_id)); continue };
        if u.mppts.len() > inv.mppt_count as usize && u.mppts[inv.mppt_count as usize..].iter().any(|g| !g.is_empty()) {
            push("MPPT_INDEX", Severity::Error, format!("{base}/mppts"), format!("{}: strings assigned to MPPT {} but {} {} has only {} MPPT input(s).", u.name, u.mppts.len(), inv.manufacturer, inv.model, inv.mppt_count));
        }
        let mut stc_w = 0.0;
        for (mi, groups) in u.mppts.iter().enumerate() {
            if groups.is_empty() { continue; }
            let mp = format!("{base}/mppts/{mi}");
            let lbl = format!("{} MPPT {}", u.name, mi + 1);
            let inp = inv.input(mi);
            let mut limits = vec![None; groups.len()];
            let mut lens = vec![];
            let mut mods = vec![];
            for (gi, g) in groups.iter().enumerate() {
                let Some(a) = p.arrays.iter().find(|a| a.id == g.array_id) else { push("STRING_ARRAY_MISSING", Severity::Error, mp.clone(), format!("{lbl}: array '{}' does not exist.", g.array_id)); continue };
                let Some(m) = p.library.module(&a.module_id.value) else { continue };
                if g.modules_per_string == 0 || g.strings == 0 { push("STRING_EMPTY", Severity::Error, mp.clone(), format!("{lbl}: a string group has zero modules or strings.")); continue; }
                let n = g.modules_per_string as f64;
                let voc_cold = cold_open_circuit_voltage(m, p.simulation.cold_voltage_temperature_c);
                let (_, vmp_hot) = module_voltages(m, T_CELL_MAX);
                let (_, vmp_cold) = module_voltages(m, T_CELL_MIN);
                if m.requires_transformer_inverter && inv.transformer == Some(false) { push("MODULE_NEEDS_TRANSFORMER", Severity::Error, mp.clone(), format!("{lbl}: {} {} may only be used with a transformer inverter; {} {} is transformerless.", m.manufacturer, m.model, inv.manufacturer, inv.model)); }
                if let Some(o) = &g.optimizer {
                    if !o.valid() { push("OPTIMIZER_RATING", Severity::Error, mp.clone(), format!("{lbl}: invalid {} buck optimizer ratings.", o.model)); continue; }
                    if voc_cold > o.input_max_v {
                        push("OPTIMIZER_INPUT_VOLTAGE", Severity::Error, mp.clone(), format!("{lbl}: cold module Voc {voc_cold:.1} V exceeds the {} input limit {:.1} V.", o.model, o.input_max_v));
                    }
                    if m.i_sc > o.input_max_a {
                        push("OPTIMIZER_INPUT_CURRENT", Severity::Warning, mp.clone(), format!("{lbl}: module Isc {:.1} A exceeds the {} operating input-current limit {:.1} A. The replay applies the operating limit; check the separate short-circuit rating.", m.i_sc, o.model, o.input_max_a));
                    }
                }
                let cap = |v: f64| g.optimizer.as_ref().map_or(v, |o| v.min(o.output_max_v));
                limits[gi] = Some((n * cap(voc_cold), n * cap(vmp_hot), n * cap(vmp_cold), m.max_system_voltage_v));
                mods.push(m.id.clone());
                stc_w += n * g.strings as f64 * m.pmp_stc();
            }
            for chain in crate::plant::physical_strings(groups) {
                let first = &groups[chain[0]];
                if chain.iter().any(|&i| groups[i].strings != first.strings) {
                    push("STRING_SERIES_COUNT", Severity::Error, mp.clone(), format!("{lbl}: series segments must have equal parallel multiplicity."));
                    continue;
                }
                let Some(series) = chain.iter().map(|&i| limits[i]).collect::<Option<Vec<_>>>() else { continue; };
                let (voc, hot, cold) = series.iter().fold((0.0, 0.0, 0.0), |(v, h, c), n| (v + n.0, h + n.1, c + n.2));
                let count: u32 = chain.iter().map(|&i| groups[i].modules_per_string).sum();
                lens.push(count);
                if let Some(vmax) = series.iter().filter_map(|item| item.3).reduce(f64::min).filter(|&vmax| voc > vmax) {
                    push("MODULE_SYSTEM_VOLTAGE", Severity::Error, mp.clone(), format!("{lbl}: string Voc {voc:.0} V at {test_temperature} °C exceeds the module's maximum system voltage {vmax:.0} V."));
                }
                if voc > inv.v_dc_max {
                    push("STRING_VOC_MAX", Severity::Error, mp.clone(), format!("{lbl}: {count} modules in series reach Voc {voc:.0} V at {test_temperature} °C, above the inverter maximum {:.0} V.", inv.v_dc_max));
                }
                let regular = chain.iter().all(|&i| groups[i].optimizer.is_none());
                if regular && hot < inp.v_min { push("STRING_VMP_MIN", Severity::Warning, mp.clone(), format!("{lbl}: string Vmp at {T_CELL_MAX} °C is {hot:.0} V, below the MPPT minimum {:.0} V: the simulation shows the loss on hot days.", inp.v_min)); }
                if regular && cold > inp.v_max { push("STRING_VMP_MAX", Severity::Warning, mp.clone(), format!("{lbl}: string Vmp at {T_CELL_MIN} °C is {cold:.0} V, above the MPPT maximum {:.0} V.", inp.v_max)); }
            }
            let (isc, imp, _, direct_isc) = string_currents(p, groups);
            let (isc_lim, isc_txt) = match inp.i_sc_max { Some(x) => (x, format!("the input's {x:.1} A short-circuit rating")), None => (inp.i_max * 1.25, format!("125 % of the {:.1} A input limit", inp.i_max)) };
            if isc > isc_lim {
                if direct_isc <= isc_lim { push("OPTIMIZER_OUTPUT_CURRENT", Severity::Warning, mp.clone(), format!("{lbl}: optimizer output envelope {isc:.1} A exceeds {isc_txt}. Confirm device-specific short-circuit behavior separately.")); }
                else { push("MPPT_CURRENT", Severity::Error, mp.clone(), format!("{lbl}: short-circuit current {direct_isc:.1} A exceeds {isc_txt}. Move strings to another MPPT.")); }
            }
            else if imp > inp.i_max { push("MPPT_CURRENT_LIMITED", Severity::Warning, mp.clone(), format!("{lbl}: MPP current {imp:.1} A exceeds the {:.1} A input limit; the inverter will limit current (simulated).", inp.i_max)); }
            let stc_mppt: f64 = groups.iter().filter_map(|g| { let a = p.arrays.iter().find(|a| a.id == g.array_id)?; let m = p.library.module(&a.module_id.value)?; Some(g.strings as f64 * g.modules_per_string as f64 * m.pmp_stc()) }).sum();
            if let Some(pm) = inp.p_max_w { if stc_mppt > pm * 1.3 { push("MPPT_POWER", Severity::Warning, mp.clone(), format!("{lbl}: {:.0} W STC on an input rated {pm:.0} W; the input limits power (simulated).", stc_mppt)); } }
            lens.sort(); lens.dedup();
            if lens.len() > 1 { push("MPPT_STRING_LENGTHS", Severity::Warning, mp.clone(), format!("{lbl}: parallel strings of different lengths ({lens:?} modules) share one MPP tracker; the mismatch loss is simulated.")); }
            mods.sort(); mods.dedup();
            if mods.len() > 1 { push("MPPT_MIXED_MODULES", Severity::Warning, mp.clone(), format!("{lbl}: different module types on one MPP tracker.")); }
        }
        // Aggregate DC limits, connectors and MPPT interconnection rules.
        let (isc_all, imp_all, n_strings, direct_isc_all) = u.mppts.iter().map(|groups| string_currents(p, groups))
            .fold((0.0, 0.0, 0, 0.0), |(s, i, n, d), (b, c, k, a)| (s+b, i+c, n+k, d+a));
        if let Some(x) = inv.i_sc_dc_max { if isc_all > x {
            let severity = if direct_isc_all > x { Severity::Error } else { Severity::Warning };
            push("INVERTER_ISC_TOTAL", severity, format!("{base}/mppts"), format!("{}: total current envelope {isc_all:.1} A exceeds the inverter's {x:.1} A short-circuit limit.", u.name));
        } }
        if let Some(x) = inv.i_dc_max { if imp_all > x { push("INVERTER_IDC_TOTAL", Severity::Warning, format!("{base}/mppts"), format!("{}: total MPP current {imp_all:.1} A exceeds the inverter's {x:.1} A operating limit.", u.name)); } }
        if let Some(c) = inv.dc_connectors { if c > 0 && n_strings > c { push("INVERTER_CONNECTORS", Severity::Warning, format!("{base}/mppts"), format!("{}: {n_strings} strings but {c} DC input connectors; strings must be combined before the inverter.", u.name)); } }
        if let (Some(ph), 1) = (inv.phases, p.grid.phases) { if ph == 3 { push("INVERTER_PHASES", Severity::Error, format!("{base}/inverter_id"), format!("{}: a three-phase inverter cannot connect to a single-phase grid.", u.name)); } }
        let used: Vec<usize> = u.mppts.iter().enumerate().filter(|(_, g)| !g.is_empty()).map(|(i, _)| i).collect();
        let linked: usize = u.interconnected.iter().filter(|g| g.len() > 1).map(|g| g.len()).sum();
        match inv.mppt_interconnection {
            Some(0) if linked > 0 => push("MPPT_INTERCONNECT", Severity::Error, format!("{base}/interconnected"), format!("{}: this inverter does not allow interconnecting MPP trackers.", u.name)),
            Some(2) if linked > 0 && !(u.interconnected.len() == 1 && used.iter().all(|i| u.interconnected[0].contains(i)) && u.interconnected[0].len() == inv.mppt_count as usize) =>
                push("MPPT_INTERCONNECT", Severity::Error, format!("{base}/interconnected"), format!("{}: this inverter requires either no interconnection or all {} trackers interconnected.", u.name, inv.mppt_count)),
            _ => {}
        }
        if u.interconnected.iter().flatten().any(|&i| i >= inv.mppt_count as usize) { push("MPPT_INTERCONNECT", Severity::Error, format!("{base}/interconnected"), format!("{}: interconnection names an MPPT the inverter does not have.", u.name)); }
        if stc_w > 0.0 {
            let r = stc_w / inv.pac_max_w;
            if !(0.8..=1.5).contains(&r) { push("DC_AC_RATIO", Severity::Warning, format!("{base}/inverter_id"), format!("{}: DC/AC ratio {r:.2} is outside 0.8–1.5.", u.name)); }
        }
    }
    let used = crate::derive::modules_by_array(&e);
    for (i, a) in p.arrays.iter().enumerate() {
        let n = used.get(&a.id).copied().unwrap_or(0);
        if n == 0 { push("ARRAY_UNASSIGNED", Severity::Warning, format!("/arrays/{i}"), format!("{} has no modules assigned to an inverter.", a.name.value)); }
        if let Some(roof) = &a.roof {
            if n as usize > roof.modules.len() {
                push("ROOF_CAPACITY", Severity::Error, format!("/arrays/{i}/strings"), format!("{}: {n} modules in strings, but only {} fit on the roof layout.", a.name.value, roof.modules.len()));
            }
        }
    }
}

fn string_currents(p: &Project, groups: &[crate::plant::StringGroup]) -> (f64, f64, u32, f64) {
    crate::plant::physical_strings(groups).into_iter().fold((0.0, 0.0, 0, 0.0), |(isc, imp, n, direct_isc), chain| {
        let count = groups[chain[0]].strings;
        let with_converter = chain.iter().any(|&i| groups[i].optimizer.is_some());
        let current = |working| chain.iter().filter_map(|&i| {
            let g = &groups[i];
            let m = p.arrays.iter().find(|a| a.id == g.array_id).and_then(|a| p.library.module(&a.module_id.value))?;
            Some(g.optimizer.as_ref().map_or(if working {m.i_mp} else {m.i_sc}, |o| o.output_max_a))
        }).reduce(f64::min).unwrap_or(0.0) * f64::from(count);
        let chain_isc = current(false);
        let verified_isc = if with_converter { 0.0 } else { chain_isc };
        (isc + chain_isc, imp + current(true), n.saturating_add(count), direct_isc + verified_isc)
    })
}

fn validate_clusters(p: &Project, push: &mut Push) {
    use crate::{plant::Coupling, workflow::SystemType};
    let clusters = &p.island_clusters;
    if clusters.is_empty() || !p.workflow.system.islanded() { return; }
    let list = "/island_clusters";
    if !(1..=50).contains(&p.economics.horizon_years) || !(-99.0..=100.0).contains(&p.economics.discount_rate_pct) {
        push("ISLAND_FINANCE", Severity::Error, "/economics".into(), "Check the shared cost horizon and discount rate.".into());
    }
    let electrical = crate::derive::electrical(p);
    let mut units = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    let mut share = 0.0;
    let mut shares_site_load = false;
    if clusters.len() > 24 {
        push("ISLAND_CLUSTER_COUNT", Severity::Error, list.into(), "Use at most 24 independently dispatched islands.".into());
        return;
    }
    for (i, c) in clusters.iter().enumerate() {
        let path = format!("{list}/{i}");
        if let Some(load) = &c.load {
            use crate::plant::LoadProfile;
            let valid = match &load.profile {
                LoadProfile::None => false,
                LoadProfile::Synthetic { .. } => (0.000_001..=1e12).contains(&load.annual_kwh),
                LoadProfile::Series { blob, values, step_minutes, .. } => !blob.is_empty() && *values >= 8760
                    && *step_minutes > 0 && 60 % *step_minutes == 0,
            };
            if !valid {
                push("ISLAND_CLUSTER_PROFILE", Severity::Error, format!("{path}/load"), "Configure a positive synthetic load or import a local interval series for this island.".into());
            }
        } else {
            shares_site_load = true;
            share += c.load_share_pct;
        }
        if c.name.trim().is_empty() || c.name.len() > 100 || !names.insert(c.name.as_str())
            || !(0.0..=100.0).contains(&c.load_share_pct) {
            push("ISLAND_CLUSTER", Severity::Error, path.clone(), "Give each island a unique name and a load share from 0% to 100%.".into());
        }
        for id in &c.inverter_units {
            if !electrical.inverters.iter().any(|u| &u.id == id) || !units.insert(id.as_str()) {
                push("ISLAND_CLUSTER_INVERTER", Severity::Error, path.clone(), format!("Inverter unit '{id}' must exist and belong to only one island."));
            }
        }
        if matches!(&c.storage.coupling, Coupling::DcHybrid { unit_id } if !c.inverter_units.contains(unit_id)) {
            push("ISLAND_CLUSTER_HYBRID", Severity::Error, path.clone(), "A DC-coupled battery must use an inverter on the same island.".into());
        }
        let mut child = p.clone();
        child.island_clusters.clear();
        child.hourly_cables = Default::default();
        child.storage = Some(c.storage.clone());
        child.island_plan = c.controls.clone();
        child.workflow.system = if c.storage.generator_id.is_some() { SystemType::OffGridGenerator } else { SystemType::OffGrid };
        plant_issues(&child, &mut |code, severity, pointer, message| {
            if pointer.starts_with("/storage") || pointer == "/island_plan" {
                let suffix = if pointer == "/island_plan" { "/controls" } else { &pointer };
                push(code, severity, format!("{path}{suffix}"), message);
            }
        });
    }
    if units.len() != electrical.inverters.len() || (shares_site_load && (share - 100.0).abs() > 1e-6) {
        push("ISLAND_CLUSTER_ALLOCATION", Severity::Error, list.into(), "Assign every PV inverter unit once. Islands sharing the site load must split 100% of that profile.".into());
    }
    if p.workflow.system == SystemType::OffGridGenerator && clusters.iter().all(|c| c.storage.generator_id.is_none()) {
        push("ISLAND_GENERATOR_REQUIRED", Severity::Error, list.into(), "At least one island needs a generator in this workflow.".into());
    }
}
/// Grid, loads, storage and tariff checks.
fn plant_issues(p: &Project, push: &mut Push) {
    use crate::{plant::{Coupling, LoadProfile}, workflow::SystemType};
    let system = p.workflow.system;
    let all_islands_have_loads = system.islanded() && !p.island_clusters.is_empty() && p.island_clusters.iter().all(|c| c.load.is_some());
    if !matches!(system, SystemType::Custom | SystemType::GridExport) && !all_islands_have_loads && matches!(p.loads.profile, LoadProfile::None) {
        push("WORKFLOW_LOAD_REQUIRED", Severity::Error, "/loads/profile".into(), "Select a consumption profile on the Consumption page.".into());
    }
    if (system.needs_ev() && p.ev.is_none() && p.fleet.is_none()) || p.ev.as_ref().is_some_and(|ev| !ev.valid()) {
        push("EV_CONFIGURATION", Severity::Error, "/ev".into(), "Configure EV energy, charger power, charging efficiency and arrival/departure hours.".into());
    }
    if (system.needs_heating() && p.heating.is_none()) || p.heating.as_ref().is_some_and(|heat| !heat.valid()) {
        push("HEATING_CONFIGURATION", Severity::Error, "/heating".into(), "Configure a valid heating demand, heat pump and backup capacity.".into());
    }
    if p.fleet.as_ref().is_some_and(|fleet| !fleet.valid()) {
        push("EV_FLEET", Severity::Error, "/fleet".into(), "Check vehicle groups, weekday schedules, state of charge and shared charger limits.".into());
    }
    if p.heat_store.as_ref().is_some_and(|store| !store.valid() || p.heating.is_none()) {
        push("HEAT_STORE", Severity::Error, "/heat_store".into(), "Thermal storage requires valid capacity, heat limits and a configured heat pump.".into());
    }
    if let Some(plan) = p.island_plan.as_ref().filter(|_| system.islanded() && p.island_clusters.is_empty()) {
        if !plan.valid() || p.economics.horizon_years == 0 || p.economics.horizon_years > 50
            || !(-99.0..=100.0).contains(&p.economics.discount_rate_pct) {
            push("ISLAND_PLAN", Severity::Error, "/island_plan".into(), "Check generator settings, cost horizon and discount rate.".into());
        }
        if system == SystemType::OffGridGenerator && plan.charge_battery
            && p.storage.as_ref().is_some_and(|st| matches!(st.coupling, Coupling::DcHybrid { .. })
                || plan.start_soc_pct < st.soc_min_pct || plan.stop_soc_pct > st.soc_max_pct)
        {
            push("ISLAND_GENERATOR_CHARGE", Severity::Error, "/island_plan".into(), "Generator charging requires an AC battery converter and thresholds inside its SOC window.".into());
        }
    }
    if !p.hourly_cables.routed.is_empty() {
        if let Err(error) = crate::cable::routed(p, &crate::derive::electrical(p)) {
            push("ROUTED_CABLE_LOSS", Severity::Error, "/hourly_cables/routed".into(), error);
        }
    }
    for value in [p.hourly_cables.dc_pct, p.hourly_cables.ac_pct].into_iter().flatten() {
        if !(0.0..=25.0).contains(&value) { push("HOURLY_CABLE_LOSS", Severity::Error, "/hourly_cables".into(), "The hourly cable model requires a design-point loss between 0% and 25%.".into()); }
    }
    for (key, loss) in [("dc_wiring_pct", p.losses.dc_wiring_pct.value), ("ac_wiring_pct", p.losses.ac_wiring_pct.value)] {
        if !(0.0..100.0).contains(&loss) {
            push("CABLE_LOSS_RANGE", Severity::Error, format!("/losses/{key}"), "Wiring loss must be at least zero and below 100%.".into());
        }
    }
    validate_clusters(p, push);
    if system.needs_storage() && p.storage.is_none() && p.island_clusters.is_empty() {
        push("WORKFLOW_STORAGE_REQUIRED", Severity::Error, "/storage".into(), "Configure the battery on the Battery page.".into());
    }
    if system == SystemType::OffGridGenerator && p.storage.as_ref().is_some_and(|s| s.generator_id.is_none()) {
        push("ISLAND_GENERATOR_REQUIRED", Severity::Error, "/storage/generator_id".into(), "Select a backup generator on the Battery page.".into());
    }
    if let Some(l) = p.grid.export_limit_kw { if l < 0.0 { push("GRID_EXPORT_LIMIT", Severity::Error, "/grid/export_limit_kw".into(), "Export limit cannot be negative.".into()); } }
    if !matches!(p.grid.phases, 1 | 3) { push("GRID_PHASES", Severity::Error, "/grid/phases".into(), "Grid connection must be single-phase (1) or three-phase (3).".into()); }
    if !all_islands_have_loads && matches!(p.loads.profile, LoadProfile::Synthetic { .. }) && p.loads.annual_kwh <= 0.0 { push("LOAD_ANNUAL", Severity::Error, "/loads/annual_kwh".into(), "Annual consumption must be positive.".into()); }
    if !all_islands_have_loads && !(0.0..=1.0).contains(&p.loads.critical_fraction) { push("LOAD_CRITICAL", Severity::Error, "/loads/critical_fraction".into(), "Critical load share must be between 0 and 1.".into()); }
    if let Some(st) = &p.storage {
        let Some(b) = p.library.battery(&st.battery_id) else { push("BATTERY_MISSING", Severity::Error, "/storage/battery_id".into(), format!("Battery '{}' is not in the project library.", st.battery_id)); return };
        if st.units == 0 { push("BATTERY_UNITS", Severity::Error, "/storage/units".into(), "At least one battery unit is required.".into()); }
        if st.soc_min_pct < b.soc_min_pct - 1e-9 || st.soc_max_pct > b.soc_max_pct + 1e-9 || st.soc_min_pct >= st.soc_max_pct {
            push("STORAGE_SOC_WINDOW", Severity::Error, "/storage/soc_min_pct".into(), format!("Operating SOC window {:.0}–{:.0} % must lie inside the battery's {:.0}–{:.0} % and be non-empty.", st.soc_min_pct, st.soc_max_pct, b.soc_min_pct, b.soc_max_pct));
        }
        if st.backup_reserve_pct < 0.0 || st.grid_services_pct < 0.0 || st.backup_reserve_pct + st.grid_services_pct > 100.0 + 1e-9 {
            push("STORAGE_ALLOCATION", Severity::Error, "/storage/backup_reserve_pct".into(), format!("Backup reserve {:.0} % plus grid services {:.0} % exceed the usable energy (100 %). Stored energy cannot be allocated twice.", st.backup_reserve_pct, st.grid_services_pct));
        }
        let mut pr = st.charge_priority.clone(); pr.sort_by_key(|u| *u as u8); pr.dedup();
        if pr.len() != 3 || st.charge_priority.len() != 3 { push("STORAGE_PRIORITY", Severity::Error, "/storage/charge_priority".into(), "Charge priority must list backup, self_consumption and grid_services once each.".into()); }
        match &st.coupling {
            Coupling::DcHybrid { unit_id } => {
                let e = crate::derive::electrical(p);
                match e.inverters.iter().find(|u| &u.id == unit_id) {
                    None => push("STORAGE_COUPLING", Severity::Error, "/storage/coupling".into(), format!("DC coupling names inverter unit '{unit_id}', which does not exist. Units: {}.", e.inverters.iter().map(|u| u.id.as_str()).collect::<Vec<_>>().join(", "))),
                    Some(u) => match p.library.inverter(&u.inverter_id).and_then(|i| i.battery_port.clone()) {
                        None => push("STORAGE_COUPLING", Severity::Error, "/storage/coupling".into(), format!("{} has no DC battery port. Choose a hybrid inverter or AC coupling with a battery inverter.", u.name)),
                        Some(port) => if b.nominal_voltage_v < port.v_min || b.nominal_voltage_v > port.v_max {
                            push("STORAGE_VOLTAGE", Severity::Error, "/storage/battery_id".into(), format!("Battery voltage {:.0} V is outside the inverter battery port range {:.0}–{:.0} V.", b.nominal_voltage_v, port.v_min, port.v_max));
                        },
                    },
                }
            }
            Coupling::Integrated => if b.converter.is_none() {
                push("STORAGE_COUPLING", Severity::Error, "/storage/coupling".into(), format!("{} {} has no built-in converter; choose DC or AC coupling.", b.manufacturer, b.model));
            },
            Coupling::AcCoupled { battery_inverter_id } => if p.library.battery_inverter(battery_inverter_id).is_none() {
                push("STORAGE_COUPLING", Severity::Error, "/storage/coupling".into(), format!("Battery inverter '{battery_inverter_id}' is not in the project library."));
            },
        }
        if let Some(g) = &st.generator_id { if p.library.generator(g).is_none() { push("GENERATOR_MISSING", Severity::Error, "/storage/generator_id".into(), format!("Generator '{g}' is not in the project library.")); } }
        if let Coupling::AcCoupled { battery_inverter_id } = &st.coupling {
            if let Some(bi) = p.library.battery_inverter(battery_inverter_id) {
                let v = b.cells.as_ref().map_or(b.nominal_voltage_v, |c| c.pack_voltage());
                if bi.battery_v_min.is_some_and(|lo| v < lo) || bi.battery_v_max.is_some_and(|hi| v > hi) {
                    push("STORAGE_VOLTAGE", Severity::Error, "/storage/battery_id".into(), format!("Battery voltage {v:.0} V is outside the battery inverter's {:.0}–{:.0} V range.", bi.battery_v_min.unwrap_or(0.0), bi.battery_v_max.unwrap_or(f64::INFINITY)));
                }
            }
        }
        for (i, o) in st.outages.iter().enumerate() {
            if o.start_hour >= 8760 || o.hours == 0 { push("OUTAGE_RANGE", Severity::Error, format!("/storage/outages/{i}"), "Outage must start within the year (hour 0–8759) and last at least one hour.".into()); }
        }
        if let crate::plant::GridService::ExportWindow { start_hour, end_hour, power_kw } = st.grid_service {
            if start_hour > 23 || end_hour > 24 || power_kw <= 0.0 { push("GRID_SERVICE", Severity::Error, "/storage/grid_service".into(), "Export window hours must be 0–24 and power positive.".into()); }
        }
        if st.grid_services_pct > 0.0 && matches!(st.grid_service, crate::plant::GridService::None) {
            push("GRID_SERVICE_UNUSED", Severity::Warning, "/storage/grid_services_pct".into(), "Energy is allocated to grid services but no grid service is configured; it stays unused.".into());
        }
    }
    if system.islanded() { return; }
    for (i, t) in p.tariff.tou.iter().enumerate() {
        if t.start_hour > 23 || t.end_hour > 24 || t.months.iter().any(|m| !(1..=12).contains(m)) { push("TARIFF_PERIOD", Severity::Error, format!("/tariff/tou/{i}"), format!("Tariff period '{}' has invalid hours or months.", t.name)); }
    }
    if p.economics.horizon_years == 0 || p.economics.horizon_years > 50 { push("ECON_HORIZON", Severity::Error, "/economics/horizon_years".into(), "Analysis period must be 1–50 years.".into()); }
}
