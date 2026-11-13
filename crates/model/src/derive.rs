// SPDX-License-Identifier: GPL-3.0-or-later
//! Derivation rules that fill non-user fields (Easy mode and defaults).
use crate::equipment::{InverterSpec, GENERIC_INVERTER_SIZES_KW};
use crate::Project;
use solarcraft_pv::diode;

/// Rule-of-thumb fixed tilt: about 0.87·|latitude| (bounded 5–60°).
/// Low-latitude sites keep ≥ 5–10° for rain self-cleaning.
pub fn default_tilt(latitude: f64) -> f64 { (0.87 * latitude.abs()).clamp(5.0, 60.0).round() }

/// Design temperatures for string sizing, °C (cell temperature).
pub const T_CELL_MIN: f64 = -10.0;
pub const T_CELL_MAX: f64 = 70.0;
/// Target DC/AC ratio for derived inverter sizing.
pub const TARGET_DC_AC: f64 = 1.2;

/// Module Voc and Vmp at a cell temperature, from the fitted model.
pub fn module_voltages(m: &crate::equipment::ModuleSpec, t_cell: f64) -> (f64, f64) {
    match m.desoto() {
        Some(p) => { let d = diode::calcparams_desoto(1000.0, t_cell, &p); (d.voc(), d.mpp().0) }
        None => (m.v_oc + m.beta_voc * (t_cell - 25.0), m.v_mp * (1.0 + m.gamma_pmp / 100.0 * (t_cell - 25.0))),
    }
}

pub fn derive(p: &mut Project) {
    // Mounting default: rooftop for small systems, open rack for large ones.
    let total_kwp: f64 = p.arrays.iter().map(|a| a.target_kwp.value).sum();
    if !p.simulation.mounting.is_user() {
        p.simulation.mounting.derive(if total_kwp > 100.0 { crate::Mounting::OpenRack } else { crate::Mounting::RoofMounted });
    }
    let (u0, u1) = p.simulation.mounting.value.faiman();
    p.simulation.faiman_u0.derive(u0);
    p.simulation.faiman_u1.derive(u1);
    if !p.site.elevation.is_user() && p.site.elevation.value == 0.0 { /* filled by weather import */ }
    for i in 0..p.arrays.len() {
        let lat = p.site.latitude.value;
        let a = &mut p.arrays[i];
        a.tilt.derive(if a.tilt.is_user() { a.tilt.value } else { default_tilt(lat) });
        if !a.azimuth.is_user() { a.azimuth.derive(if lat >= 0.0 { 180.0 } else { 0.0 }); }
        let Some(module) = p.library.module(&a.module_id.value).cloned() else { continue };
        // Roof layout: module size comes from the selected module; recompute only when inputs change.
        if let Some(roof) = a.roof.as_mut() {
            roof.params.module_length = module.length_m.max(module.width_m);
            roof.params.module_width = module.length_m.min(module.width_m);
            let key = serde_json::to_string(&(&roof.face, &roof.params)).unwrap_or_default();
            if key != roof.inputs_key {
                let l = solarcraft_geometry::layout::fill(&roof.face, &roof.params);
                roof.modules = l.modules;
                roof.orientation = l.orientation;
                roof.inputs_key = key;
            }
            let count = roof.modules.len() as f64;
            if !a.target_kwp.is_user() { a.target_kwp.derive((count * module.pmp_stc() / 1000.0 * 1000.0).round() / 1000.0); }
        }
        // Inverter: generic size nearest TARGET_DC_AC unless user-chosen.
        let target_kw = if a.modules_per_string.is_user() && a.strings.is_user() {
            a.modules_per_string.value as f64 * a.strings.value as f64 * module.pmp_stc() / 1000.0
        } else { a.target_kwp.value };
        // Large arrays: split across several equal inverters (≤ 100 kW each).
        if !a.inverter_count.is_user() && !a.inverter_id.is_user() {
            a.inverter_count.derive(((target_kw / TARGET_DC_AC) / 100.0).ceil().max(1.0) as u32);
        }
        let n_inv = a.inverter_count.value.max(1) as f64;
        if !a.inverter_id.is_user() {
            let want = target_kw / TARGET_DC_AC / n_inv;
            let size = GENERIC_INVERTER_SIZES_KW.iter().copied().min_by(|x, y| (x - want).abs().partial_cmp(&(y - want).abs()).unwrap()).unwrap();
            let id = p.library.ensure_inverter(InverterSpec::generic(size));
            p.arrays[i].inverter_id.derive(id);
        }
        let a = &mut p.arrays[i];
        let Some(inv) = p.library.inverter(&a.inverter_id.value).cloned() else { continue };
        // Modules per string: largest count with cold Voc ≤ Vdc,max and hot Vmp ≥ MPPT min.
        if !a.modules_per_string.is_user() {
            let (voc_cold, _) = module_voltages(&module, T_CELL_MIN);
            let (_, vmp_hot) = module_voltages(&module, T_CELL_MAX);
            let n_max = (inv.v_dc_max / voc_cold).floor().max(1.0) as u32;
            let n_min = (inv.mppt_v_min / vmp_hot).ceil().max(1.0) as u32;
            let modules_total = (a.target_kwp.value * 1000.0 / module.pmp_stc()).round().max(1.0) as u32;
            // String capacity of the inverter(s): MPPTs × parallel strings per MPPT.
            let per_mppt = (inv.i_max_per_mppt / module.i_sc).floor().max(1.0) as u32;
            let capacity = inv.mppt_count * per_mppt * a.inverter_count.value.max(1);
            // Among lengths in [n_min, n_max] whose string count fits, minimise unused
            // modules, then prefer longer strings (fewer strings, lower current).
            let n = (n_min..=n_max.max(n_min))
                .filter(|&n| modules_total.div_ceil(n) <= capacity || n == n_max)
                .min_by_key(|&n| { let s = (modules_total as f64 / n as f64).round().max(1.0) as u32; ((s * n).abs_diff(modules_total), u32::MAX - n) })
                .unwrap_or(n_max)
                .min(modules_total.max(1));
            a.modules_per_string.derive(n);
        }
        if !a.strings.is_user() {
            let per_string = a.modules_per_string.value.max(1) as f64 * module.pmp_stc();
            a.strings.derive((a.target_kwp.value * 1000.0 / per_string).round().max(1.0) as u32);
        }
        // In Detailed mode the actual kWp follows the counts.
        if !a.target_kwp.is_user() || (a.modules_per_string.is_user() && a.strings.is_user()) {
            let kwp = a.modules_per_string.value as f64 * a.strings.value as f64 * module.pmp_stc() / 1000.0;
            if a.modules_per_string.is_user() && a.strings.is_user() { a.target_kwp.origin = crate::Origin::Derived; }
            a.target_kwp.derive((kwp * 1000.0).round() / 1000.0);
        }
    }
}
