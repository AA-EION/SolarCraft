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
        let Some(inv) = p.library.inverter(&a.inverter_id.value) else { push("INVERTER_MISSING", Severity::Error, format!("{base}/inverter_id"), "No inverter selected.".into()); continue };
        let n = a.modules_per_string.value as f64;
        let (voc_cold, _) = module_voltages(m, T_CELL_MIN);
        let (_, vmp_hot) = module_voltages(m, T_CELL_MAX);
        let (_, vmp_cold) = module_voltages(m, T_CELL_MIN);
        if n * voc_cold > inv.v_dc_max { push("STRING_VOC_MAX", Severity::Error, format!("{base}/modules_per_string"), format!("String Voc at {T_CELL_MIN} °C is {:.0} V, above the inverter maximum {:.0} V.", n * voc_cold, inv.v_dc_max)); }
        if n * vmp_hot < inv.mppt_v_min { push("STRING_VMP_MIN", Severity::Warning, format!("{base}/modules_per_string"), format!("String Vmp at {T_CELL_MAX} °C is {:.0} V, below the MPPT minimum {:.0} V: expect losses on hot days.", n * vmp_hot, inv.mppt_v_min)); }
        if n * vmp_cold > inv.mppt_v_max { push("STRING_VMP_MAX", Severity::Warning, format!("{base}/modules_per_string"), format!("String Vmp at {T_CELL_MIN} °C is {:.0} V, above the MPPT maximum {:.0} V.", n * vmp_cold, inv.mppt_v_max)); }
        let strings_per_inv = (a.strings.value as f64 / a.inverter_count.value.max(1) as f64).ceil();
        let per_mppt = (strings_per_inv / inv.mppt_count as f64).ceil();
        if per_mppt * m.i_sc * 1.25 > inv.i_max_per_mppt * 1.0 + 1e-9 && per_mppt * m.i_sc > inv.i_max_per_mppt {
            push("MPPT_CURRENT", Severity::Warning, format!("{base}/strings"), format!("{per_mppt:.0} string(s) per MPPT give Isc {:.1} A, above the {:.1} A input limit.", per_mppt * m.i_sc, inv.i_max_per_mppt));
        }
        if strings_per_inv > inv.mppt_count as f64 * (inv.i_max_per_mppt / m.i_sc).floor().max(1.0) { push("INVERTER_INPUTS", Severity::Error, format!("{base}/strings"), "More strings than the inverter can accept.".into()); }
        let dcac = n * a.strings.value as f64 * m.pmp_stc() / (inv.pac_max_w * a.inverter_count.value.max(1) as f64);
        if !(0.8..=1.5).contains(&dcac) { push("DC_AC_RATIO", Severity::Warning, format!("{base}/inverter_id"), format!("DC/AC ratio {dcac:.2} is outside 0.8–1.5.")); }
    }
    v
}
