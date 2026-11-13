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
    /// Maximum DC input power, W (None: no separate DC limit).
    #[serde(default)]
    pub pdc_max_w: Option<f64>,
    /// Part-load efficiency curve: (P_dc / P_dc,nominal, efficiency) knots, ascending.
    /// When absent the PVWatts curve with `eta_nominal` is used.
    #[serde(default)]
    pub efficiency_curve: Vec<[f64; 2]>,
    /// DC battery port of a hybrid inverter.
    #[serde(default)]
    pub battery_port: Option<BatteryPort>,
    #[serde(default)]
    pub datasheet_ref: String,
}

/// DC battery input of a hybrid inverter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryPort {
    pub v_min: f64,
    pub v_max: f64,
    pub p_charge_max_w: f64,
    pub p_discharge_max_w: f64,
    /// DC/DC conversion efficiency between the PV bus and the battery.
    pub dc_dc_efficiency: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chemistry { Lfp, Nmc, LeadAcid, Other }

/// A battery unit (pack or modular system with its BMS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatterySpec {
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub provenance: Provenance,
    pub chemistry: Chemistry,
    pub capacity_kwh: f64,
    pub nominal_voltage_v: f64,
    /// Hardware SOC limits, % of nominal capacity.
    pub soc_min_pct: f64,
    pub soc_max_pct: f64,
    pub p_charge_max_kw: f64,
    pub p_discharge_max_kw: f64,
    /// DC round-trip efficiency of the cells/BMS.
    pub roundtrip_efficiency: f64,
    pub self_discharge_pct_per_month: f64,
    /// Full equivalent cycles to 80 % remaining capacity.
    pub cycle_life: f64,
    #[serde(default)]
    pub datasheet_ref: String,
}

/// A bidirectional battery inverter on the AC bus (AC coupling).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryInverterSpec {
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub provenance: Provenance,
    pub p_ac_max_w: f64,
    /// (P / P_rated, efficiency) knots; used in both directions.
    pub efficiency_curve: Vec<[f64; 2]>,
    pub standby_w: f64,
    #[serde(default)]
    pub datasheet_ref: String,
}

/// Interpolate an efficiency curve at a load fraction (0 at zero load, flat beyond the last knot).
pub fn curve_efficiency(curve: &[[f64; 2]], x: f64) -> f64 {
    if curve.is_empty() || x <= 0.0 { return 0.0; }
    let mut prev = [0.0, 0.0];
    for k in curve {
        if x <= k[0] { let t = if k[0] > prev[0] { (x - prev[0]) / (k[0] - prev[0]) } else { 1.0 }; return prev[1] + t * (k[1] - prev[1]); }
        prev = *k;
    }
    prev[1]
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
            model: format!("string inverter {pac_kw} kW"),
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
            pdc_max_w: None,
            efficiency_curve: vec![],
            battery_port: None,
            datasheet_ref: String::new(),
        }
    }

    /// SolarCraft's generic hybrid inverter (illustrative values, not a real product).
    pub fn generic_hybrid(pac_kw: f64) -> Self {
        let mut i = InverterSpec::generic(pac_kw);
        i.id = format!("generic-hybrid-{pac_kw}kw");
        i.model = format!("hybrid inverter {pac_kw} kW");
        i.mppt_count = 2;
        i.i_max_per_mppt = 16.0;
        i.pdc_max_w = Some(pac_kw * 1500.0);
        i.efficiency_curve = vec![[0.05, 0.90], [0.10, 0.935], [0.20, 0.955], [0.30, 0.965], [0.50, 0.97], [0.75, 0.968], [1.0, 0.965]];
        i.battery_port = Some(BatteryPort { v_min: 90.0, v_max: 500.0, p_charge_max_w: pac_kw * 1000.0, p_discharge_max_w: pac_kw * 1000.0, dc_dc_efficiency: 0.98 });
        i
    }

    /// AC efficiency at a DC input power (W), before clipping.
    pub fn efficiency(&self, pdc_w: f64) -> f64 {
        if pdc_w <= 0.0 { return 0.0; }
        if self.efficiency_curve.is_empty() {
            return solarcraft_pv::inverter::pvwatts_unclipped(pdc_w, self.pdc0_w(), self.eta_nominal, 0.9637) / pdc_w;
        }
        curve_efficiency(&self.efficiency_curve, pdc_w / self.pdc0_w())
    }
}

impl BatterySpec {
    pub fn generic_lfp(kwh: f64) -> Self {
        BatterySpec { id: format!("generic-bat-lfp-{kwh}kwh"), manufacturer: "Generic".into(), model: format!("LFP battery {kwh} kWh"), provenance: Provenance::Generic,
            chemistry: Chemistry::Lfp, capacity_kwh: kwh, nominal_voltage_v: 204.8, soc_min_pct: 5.0, soc_max_pct: 100.0,
            p_charge_max_kw: kwh * 0.5, p_discharge_max_kw: kwh * 0.5, roundtrip_efficiency: 0.95, self_discharge_pct_per_month: 2.0, cycle_life: 6000.0, datasheet_ref: String::new() }
    }
}

impl BatteryInverterSpec {
    pub fn generic(kw: f64) -> Self {
        BatteryInverterSpec { id: format!("generic-batinv-{kw}kw"), manufacturer: "Generic".into(), model: format!("battery inverter {kw} kW"), provenance: Provenance::Generic,
            p_ac_max_w: kw * 1000.0, efficiency_curve: vec![[0.05, 0.85], [0.10, 0.91], [0.20, 0.94], [0.50, 0.955], [1.0, 0.95]], standby_w: 5.0, datasheet_ref: String::new() }
    }
    pub fn efficiency(&self, p_w: f64) -> f64 { curve_efficiency(&self.efficiency_curve, p_w.abs() / self.p_ac_max_w) }
}

pub const GENERIC_INVERTER_SIZES_KW: [f64; 15] = [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 15.0, 20.0, 25.0, 30.0, 50.0, 60.0, 100.0];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Library {
    pub modules: Vec<ModuleSpec>,
    pub inverters: Vec<InverterSpec>,
    #[serde(default)]
    pub batteries: Vec<BatterySpec>,
    #[serde(default)]
    pub battery_inverters: Vec<BatteryInverterSpec>,
}

impl Library {
    /// SolarCraft's generic defaults (illustrative values, not real products).
    pub fn generic() -> Self {
        let mut m = ModuleSpec {
            id: "generic-mod-400-mono".into(), manufacturer: "Generic".into(), model: "400 W mono 72-cell".into(),
            provenance: Provenance::Generic, technology: "mono-c-Si".into(), cells_in_series: 72, length_m: 2.0, width_m: 1.0,
            v_oc: 48.9, i_sc: 10.6, v_mp: 40.4, i_mp: 9.9, alpha_sc: 0.0053, beta_voc: -0.140, gamma_pmp: -0.36, model_params: None,
        };
        m.fit().expect("generic module fits");
        Library { modules: vec![m], inverters: vec![], batteries: vec![], battery_inverters: vec![] }
    }
    pub fn module(&self, id: &str) -> Option<&ModuleSpec> { self.modules.iter().find(|m| m.id == id) }
    pub fn inverter(&self, id: &str) -> Option<&InverterSpec> { self.inverters.iter().find(|i| i.id == id) }
    pub fn battery(&self, id: &str) -> Option<&BatterySpec> { self.batteries.iter().find(|i| i.id == id) }
    pub fn battery_inverter(&self, id: &str) -> Option<&BatteryInverterSpec> { self.battery_inverters.iter().find(|i| i.id == id) }
    pub fn ensure_inverter(&mut self, inv: InverterSpec) -> String {
        if self.inverter(&inv.id).is_none() { self.inverters.push(inv.clone()); }
        inv.id
    }
}
