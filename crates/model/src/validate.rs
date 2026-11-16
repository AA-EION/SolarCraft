// SPDX-License-Identifier: GPL-3.0-or-later
//! Design validation. Each issue has a stable code, severity and field pointer
//! so the UI, CLI and MCP report the same thing.
use crate::derive::{module_voltages, T_CELL_MAX, T_CELL_MIN};
use crate::Project;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity { Error, Warning, Info }

#[derive(Debug, Clone, Serialize)]
pub struct Issue { pub code: &'static str, pub severity: Severity, pub pointer: String, pub message: String }

pub fn validate(p: &Project) -> Vec<Issue> {
    let mut v = vec![];
    let mut push = |code, severity, pointer: String, message: String| v.push(Issue { code, severity, pointer, message });
    if !(-90.0..=90.0).contains(&p.site.latitude.value) { push("SITE_LAT_RANGE", Severity::Error, "/site/latitude".into(), "Latitude must be between −90 and 90°.".into()); }
    if !(-180.0..=180.0).contains(&p.site.longitude.value) { push("SITE_LON_RANGE", Severity::Error, "/site/longitude".into(), "Longitude must be between −180 and 180°.".into()); }
    if !(0.0..=1.0).contains(&p.site.albedo.value) { push("SITE_ALBEDO_RANGE", Severity::Error, "/site/albedo".into(), "Albedo must be between 0 and 1.".into()); }
    if p.weather.is_none() { push("WEATHER_MISSING", Severity::Error, "/weather".into(), "No weather data. Fetch PVGIS data or import a weather file.".into()); }
    for (i, a) in p.arrays.iter().enumerate() {
        let base = format!("/arrays/{i}");
        if !(0.0..=90.0).contains(&a.tilt.value) { push("ARRAY_TILT_RANGE", Severity::Error, format!("{base}/tilt"), "Tilt must be between 0 and 90°.".into()); }
        if !(0.0..360.0).contains(&a.azimuth.value) { push("ARRAY_AZIMUTH_RANGE", Severity::Error, format!("{base}/azimuth"), "Azimuth must be in [0, 360)°, clockwise from north.".into()); }
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
    v
}

type Push<'a> = dyn FnMut(&'static str, Severity, String, String) + 'a;

/// Checks on the resolved inverter/MPPT stringing (manual or derived).
fn electrical_issues(p: &Project, push: &mut Push) {
    let e = crate::derive::electrical(p);
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
            let (mut isc, mut imp) = (0.0, 0.0);
            let mut lens = vec![];
            let mut mods = vec![];
            for g in groups {
                let Some(a) = p.arrays.iter().find(|a| a.id == g.array_id) else { push("STRING_ARRAY_MISSING", Severity::Error, mp.clone(), format!("{lbl}: array '{}' does not exist.", g.array_id)); continue };
                let Some(m) = p.library.module(&a.module_id.value) else { continue };
                if g.modules_per_string == 0 || g.strings == 0 { push("STRING_EMPTY", Severity::Error, mp.clone(), format!("{lbl}: a string group has zero modules or strings.")); continue; }
                let n = g.modules_per_string as f64;
                let (voc_cold0, _) = module_voltages(m, T_CELL_MIN);
                // Initial (pre-stabilisation) Voc increase applies to the configuration check.
                let voc_cold = voc_cold0 * (1.0 + m.voc_initial_increase.unwrap_or(0.0));
                let (_, vmp_hot) = module_voltages(m, T_CELL_MAX);
                let (_, vmp_cold) = module_voltages(m, T_CELL_MIN);
                if m.requires_transformer_inverter && inv.transformer == Some(false) { push("MODULE_NEEDS_TRANSFORMER", Severity::Error, mp.clone(), format!("{lbl}: {} {} may only be used with a transformer inverter; {} {} is transformerless.", m.manufacturer, m.model, inv.manufacturer, inv.model)); }
                if let Some(vmax) = m.max_system_voltage_v { if n * voc_cold > vmax { push("MODULE_SYSTEM_VOLTAGE", Severity::Error, mp.clone(), format!("{lbl}: string Voc {:.0} V at {T_CELL_MIN} °C exceeds the module's maximum system voltage {vmax:.0} V.", n * voc_cold)); } }
                if n * voc_cold > inv.v_dc_max { push("STRING_VOC_MAX", Severity::Error, mp.clone(), format!("{lbl}: {} modules in series reach Voc {:.0} V at {T_CELL_MIN} °C, above the inverter maximum {:.0} V. Use at most {} modules per string.", g.modules_per_string, n * voc_cold, inv.v_dc_max, (inv.v_dc_max / voc_cold).floor())); }
                if n * vmp_hot < inp.v_min { push("STRING_VMP_MIN", Severity::Warning, mp.clone(), format!("{lbl}: string Vmp at {T_CELL_MAX} °C is {:.0} V, below the MPPT minimum {:.0} V: the simulation shows the loss on hot days.", n * vmp_hot, inp.v_min)); }
                if n * vmp_cold > inp.v_max { push("STRING_VMP_MAX", Severity::Warning, mp.clone(), format!("{lbl}: string Vmp at {T_CELL_MIN} °C is {:.0} V, above the MPPT maximum {:.0} V.", n * vmp_cold, inp.v_max)); }
                isc += g.strings as f64 * m.i_sc;
                imp += g.strings as f64 * m.i_mp;
                lens.push(g.modules_per_string);
                mods.push(m.id.clone());
                stc_w += n * g.strings as f64 * m.pmp_stc();
            }
            let (isc_lim, isc_txt) = match inp.i_sc_max { Some(x) => (x, format!("the input's {x:.1} A short-circuit rating")), None => (inp.i_max * 1.25, format!("125 % of the {:.1} A input limit", inp.i_max)) };
            if isc > isc_lim { push("MPPT_CURRENT", Severity::Error, mp.clone(), format!("{lbl}: short-circuit current {isc:.1} A exceeds {isc_txt}. Move strings to another MPPT.")); }
            else if imp > inp.i_max { push("MPPT_CURRENT_LIMITED", Severity::Warning, mp.clone(), format!("{lbl}: MPP current {imp:.1} A exceeds the {:.1} A input limit; the inverter will limit current (simulated).", inp.i_max)); }
            let stc_mppt: f64 = groups.iter().filter_map(|g| { let a = p.arrays.iter().find(|a| a.id == g.array_id)?; let m = p.library.module(&a.module_id.value)?; Some(g.strings as f64 * g.modules_per_string as f64 * m.pmp_stc()) }).sum();
            if let Some(pm) = inp.p_max_w { if stc_mppt > pm * 1.3 { push("MPPT_POWER", Severity::Warning, mp.clone(), format!("{lbl}: {:.0} W STC on an input rated {pm:.0} W; the input limits power (simulated).", stc_mppt)); } }
            lens.sort(); lens.dedup();
            if lens.len() > 1 { push("MPPT_STRING_LENGTHS", Severity::Warning, mp.clone(), format!("{lbl}: parallel strings of different lengths ({lens:?} modules) share one MPP tracker; the mismatch loss is simulated.")); }
            mods.sort(); mods.dedup();
            if mods.len() > 1 { push("MPPT_MIXED_MODULES", Severity::Warning, mp.clone(), format!("{lbl}: different module types on one MPP tracker.")); }
        }
        // Aggregate DC limits, connectors and MPPT interconnection rules.
        let (mut isc_all, mut imp_all, mut n_strings) = (0.0, 0.0, 0u32);
        for g in u.mppts.iter().flatten() {
            if let Some(m) = p.arrays.iter().find(|a| a.id == g.array_id).and_then(|a| p.library.module(&a.module_id.value)) { isc_all += g.strings as f64 * m.i_sc; imp_all += g.strings as f64 * m.i_mp; }
            n_strings += g.strings;
        }
        if let Some(x) = inv.i_sc_dc_max { if isc_all > x { push("INVERTER_ISC_TOTAL", Severity::Error, format!("{base}/mppts"), format!("{}: total short-circuit current {isc_all:.1} A exceeds the inverter's {x:.1} A.", u.name)); } }
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

/// Grid, loads, storage and tariff checks.
fn plant_issues(p: &Project, push: &mut Push) {
    use crate::plant::{Coupling, LoadProfile};
    if let Some(l) = p.grid.export_limit_kw { if l < 0.0 { push("GRID_EXPORT_LIMIT", Severity::Error, "/grid/export_limit_kw".into(), "Export limit cannot be negative.".into()); } }
    if !matches!(p.grid.phases, 1 | 3) { push("GRID_PHASES", Severity::Error, "/grid/phases".into(), "Grid connection must be single-phase (1) or three-phase (3).".into()); }
    if matches!(p.loads.profile, LoadProfile::Synthetic { .. }) && p.loads.annual_kwh <= 0.0 { push("LOAD_ANNUAL", Severity::Error, "/loads/annual_kwh".into(), "Annual consumption must be positive.".into()); }
    if !(0.0..=1.0).contains(&p.loads.critical_fraction) { push("LOAD_CRITICAL", Severity::Error, "/loads/critical_fraction".into(), "Critical load share must be between 0 and 1.".into()); }
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
    for (i, t) in p.tariff.tou.iter().enumerate() {
        if t.start_hour > 23 || t.end_hour > 24 || t.months.iter().any(|m| !(1..=12).contains(m)) { push("TARIFF_PERIOD", Severity::Error, format!("/tariff/tou/{i}"), format!("Tariff period '{}' has invalid hours or months.", t.name)); }
    }
    if p.economics.horizon_years == 0 || p.economics.horizon_years > 50 { push("ECON_HORIZON", Severity::Error, "/economics/horizon_years".into(), "Analysis period must be 1–50 years.".into()); }
}
