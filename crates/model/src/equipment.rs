// SPDX-License-Identifier: GPL-3.0-or-later
//! Equipment records. The same schema is used for user-authored items,
//! imports and catalog items (brief: "works identically"). Generic items are
//! SolarCraft's own illustrative defaults, not real products.
use serde::{Deserialize, Serialize};
use solarcraft_pv::diode::{self, Datasheet, DeSotoRef};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance { Generic, User, Imported, Catalog }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleSpec {
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub provenance: Provenance,
    pub technology: String,
    pub cells_in_series: u32,
    pub length_m: f64,
    pub width_m: f64,
    pub v_oc: f64,
    pub i_sc: f64,
    pub v_mp: f64,
    pub i_mp: f64,
    /// A/K
    pub alpha_sc: f64,
    /// V/K
    pub beta_voc: f64,
    /// %/K
    pub gamma_pmp: f64,
    /// Fitted CEC six-parameter model (set by [`ModuleSpec::fit`]).
    pub model_params: Option<ModelParams>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelParams {
    pub i_l_ref: f64, pub i_o_ref: f64, pub r_s: f64, pub r_sh_ref: f64, pub a_ref: f64, pub adjust: f64,
    pub fit_exact: bool, pub fit_acceptable: bool, pub pmp_rel_err: f64, pub gamma_abs_err: f64, pub beta_voc_abs_err: f64,
}

impl ModuleSpec {
    pub fn pmp_stc(&self) -> f64 { self.v_mp * self.i_mp }
    pub fn area(&self) -> f64 { self.length_m * self.width_m }
    pub fn datasheet(&self) -> Datasheet {
        Datasheet { v_oc: self.v_oc, i_sc: self.i_sc, v_mp: self.v_mp, i_mp: self.i_mp, alpha_sc: self.alpha_sc, beta_voc: self.beta_voc, cells_in_series: self.cells_in_series, gamma_pmp: self.gamma_pmp }
    }
    /// Fit the electrical model from datasheet values (CEC six-parameter method).
    pub fn fit(&mut self) -> Result<(), String> {
        let d = self.datasheet();
        let (p, r) = diode::fit_cec(&d).map_err(|e| format!("cannot fit module {}: {e:?}", self.id))?;
        self.model_params = Some(ModelParams {
            i_l_ref: p.i_l_ref, i_o_ref: p.i_o_ref, r_s: p.r_s, r_sh_ref: p.r_sh_ref, a_ref: p.a_ref, adjust: p.adjust,
            fit_exact: r.exact, fit_acceptable: r.acceptable(&d), pmp_rel_err: r.pmp_rel_err, gamma_abs_err: r.gamma_abs_err, beta_voc_abs_err: r.beta_voc_abs_err,
        });
        Ok(())
    }
    pub fn desoto(&self) -> Option<DeSotoRef> {
        self.model_params.as_ref().map(|m| DeSotoRef { alpha_sc: self.alpha_sc, a_ref: m.a_ref, i_l_ref: m.i_l_ref, i_o_ref: m.i_o_ref, r_sh_ref: m.r_sh_ref, r_s: m.r_s, eg_ref: 1.121, d_eg_dt: -0.0002677, adjust: m.adjust })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InverterSpec {
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub provenance: Provenance,
    /// Maximum AC output, W.
    pub pac_max_w: f64,
    /// Nominal (European/CEC-like) efficiency used by the PVWatts curve.
    pub eta_nominal: f64,
    pub mppt_v_min: f64,
    pub mppt_v_max: f64,
    pub v_dc_max: f64,
    pub mppt_count: u32,
    /// Maximum input current per MPPT, A.
    pub i_max_per_mppt: f64,
    /// Night tare consumption, W.
    pub night_w: f64,
}

impl InverterSpec {
    /// DC rating such that eta_nominal · pdc0 = pac_max (PVWatts convention).
    pub fn pdc0_w(&self) -> f64 { self.pac_max_w / self.eta_nominal }

    pub fn generic(pac_kw: f64) -> Self {
        let large = pac_kw > 30.0;
        let mppt = if pac_kw <= 6.0 { 2 } else if pac_kw <= 15.0 { 3 } else if pac_kw <= 30.0 { 4 } else { 6 };
        InverterSpec {
            id: format!("generic-inv-{pac_kw}kw"),
            manufacturer: "Generic".into(),
            model: format!("Generic string inverter {pac_kw} kW"),
            provenance: Provenance::Generic,
            pac_max_w: pac_kw * 1000.0,
            eta_nominal: 0.965,
            mppt_v_min: if large { 200.0 } else { 160.0 },
            mppt_v_max: if large { 1000.0 } else { 850.0 },
            v_dc_max: if large { 1100.0 } else { 1000.0 },
            mppt_count: mppt,
            // ≤ 6 kW: one string per MPPT; larger: two (and more for > 30 kW).
            i_max_per_mppt: if large { 40.0 } else if pac_kw > 6.0 { 32.0 } else { 16.0 },
            night_w: 1.0,
        }
    }
}

pub const GENERIC_INVERTER_SIZES_KW: [f64; 15] = [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 25.0, 30.0, 50.0, 60.0, 100.0];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Library {
    pub modules: Vec<ModuleSpec>,
    pub inverters: Vec<InverterSpec>,
}

impl Library {
    /// SolarCraft's generic defaults (illustrative values, not real products).
    pub fn generic() -> Self {
        let mut m = ModuleSpec {
            id: "generic-mod-400-mono".into(), manufacturer: "Generic".into(), model: "Generic 400 W mono 72-cell".into(),
            provenance: Provenance::Generic, technology: "mono-c-Si".into(), cells_in_series: 72, length_m: 2.0, width_m: 1.0,
            v_oc: 48.9, i_sc: 10.6, v_mp: 40.4, i_mp: 9.9, alpha_sc: 0.0053, beta_voc: -0.140, gamma_pmp: -0.36, model_params: None,
        };
        m.fit().expect("generic module fits");
        Library { modules: vec![m], inverters: vec![] }
    }
    pub fn module(&self, id: &str) -> Option<&ModuleSpec> { self.modules.iter().find(|m| m.id == id) }
    pub fn inverter(&self, id: &str) -> Option<&InverterSpec> { self.inverters.iter().find(|i| i.id == id) }
    pub fn ensure_inverter(&mut self, inv: InverterSpec) -> String {
        if self.inverter(&inv.id).is_none() { self.inverters.push(inv.clone()); }
        inv.id
    }
}
