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
    #[serde(default)]
    pub catalog_ref: Option<crate::plant::CatalogRef>,
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
    /// Maximum system voltage, V (limits string Voc together with the inverter).
    #[serde(default)]
    pub max_system_voltage_v: Option<f64>,
    /// Number of bypass diodes (substrings) and their forward voltage, V.
    #[serde(default)]
    pub bypass_diodes: Option<u32>,
    #[serde(default)]
    pub bypass_diode_voltage_v: Option<f64>,
    #[serde(default)]
    pub half_cell: bool,
    /// Voc increase before stabilisation (fraction), applied in the cold-voltage check.
    #[serde(default)]
    pub voc_initial_increase: Option<f64>,
    /// Only transformer inverters may be used with this module.
    #[serde(default)]
    pub requires_transformer_inverter: bool,
    #[serde(default)]
    pub nameplate_w: Option<f64>,
    #[serde(default)]
    pub description: String,
    /// Rear/front efficiency ratio for bifacial modules.
    #[serde(default)]
    pub bifaciality: Option<f64>,
    /// Second (low-irradiance) datasheet operating point at 25 °C.
    #[serde(default)]
    pub low_light: Option<LowLightPoint>,
    #[serde(default)]
    pub thickness_m: Option<f64>,
    #[serde(default)]
    pub weight_kg: Option<f64>,
    /// Source-specific values kept verbatim whose meaning is not yet verified (not used in calculations).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub unverified: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub datasheet_ref: String,
}

/// A datasheet operating point at reduced irradiance (cell temperature 25 °C).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LowLightPoint { pub irradiance_w_m2: f64, pub v_mp: f64, pub i_mp: f64, pub v_oc: f64, pub i_sc: f64 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelParams {
    pub i_l_ref: f64, pub i_o_ref: f64, pub r_s: f64, pub r_sh_ref: f64, pub a_ref: f64, pub adjust: f64,
    pub fit_exact: bool, pub fit_acceptable: bool, pub pmp_rel_err: f64, pub gamma_abs_err: f64, pub beta_voc_abs_err: f64,
    /// Relative error of the fitted model's MPP power at the datasheet low-light point (when given).
    #[serde(default)]
    pub low_light_pmp_rel_err: Option<f64>,
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
            low_light_pmp_rel_err: None,
        });
        // Check the fit against the datasheet's low-irradiance point, when it has a usable one.
        if let (Some(ll), Some(dp)) = (self.low_light.clone(), self.desoto()) {
            let p_sheet = ll.v_mp * ll.i_mp;
            if ll.irradiance_w_m2 > 0.0 && p_sheet > 0.0 {
                let p_model = diode::calcparams_desoto(ll.irradiance_w_m2, 25.0, &dp).mpp().2;
                if let Some(mp) = self.model_params.as_mut() { mp.low_light_pmp_rel_err = Some(p_model / p_sheet - 1.0); }
            }
        }
        Ok(())
    }
    pub fn desoto(&self) -> Option<DeSotoRef> {
        self.model_params.as_ref().map(|m| DeSotoRef { alpha_sc: self.alpha_sc, a_ref: m.a_ref, i_l_ref: m.i_l_ref, i_o_ref: m.i_o_ref, r_sh_ref: m.r_sh_ref, r_s: m.r_s, eg_ref: 1.121, d_eg_dt: -0.0002677, adjust: m.adjust })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InverterSpec {
    #[serde(default)]
    pub catalog_ref: Option<crate::plant::CatalogRef>,
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
    /// Per-input limits when the MPP trackers differ (expanded: one entry per tracker).
    /// Empty: every tracker uses the inverter-level window and current.
    #[serde(default)]
    pub inputs: Vec<MpptInput>,
    /// DC power the efficiency curve's load fractions refer to, W (default: pac_max_w / eta_nominal).
    #[serde(default)]
    pub pdc_nominal_w: Option<f64>,
    #[serde(default)]
    pub v_dc_nominal: Option<f64>,
    #[serde(default)]
    pub phases: Option<u32>,
    #[serde(default)]
    pub standby_w: Option<f64>,
    #[serde(default)]
    pub transformer: Option<bool>,
    /// Inverter start threshold, W DC (source "minimum feed-in power").
    #[serde(default)]
    pub start_power_w: Option<f64>,
    /// Efficiency change per 100 V deviation from `v_dc_nominal` (decrease for positive values).
    #[serde(default)]
    pub eta_change_per_100v: Option<f64>,
    /// MPP adaptation efficiency below and above 20 % of the nominal DC power.
    #[serde(default)]
    pub mpp_matching: Option<[f64; 2]>,
    /// MPPT interconnection rule: 0 none allowed, 1 any, 2 none or all.
    #[serde(default)]
    pub mppt_interconnection: Option<u32>,
    /// Aggregate DC operating and short-circuit current limits, A.
    #[serde(default)]
    pub i_dc_max: Option<f64>,
    #[serde(default)]
    pub i_sc_dc_max: Option<f64>,
    /// Physical DC input connectors (string inputs).
    #[serde(default)]
    pub dc_connectors: Option<u32>,
    #[serde(default)]
    pub v_ac: Option<f64>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub unverified: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub datasheet_ref: String,
}

/// Limits of one MPP tracker input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MpptInput {
    pub v_min: f64,
    pub v_max: f64,
    /// Maximum operating input current, A.
    pub i_max: f64,
    /// Maximum short-circuit current the input accepts, A.
    #[serde(default)]
    pub i_sc_max: Option<f64>,
    /// Maximum DC power on this input, W.
    #[serde(default)]
    pub p_max_w: Option<f64>,
}

/// DC battery input of a hybrid inverter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryPort {
    pub v_min: f64,
    pub v_max: f64,
    pub p_charge_max_w: f64,
    pub p_discharge_max_w: f64,
    /// DC/DC conversion efficiency between the PV bus and the battery
    /// (used when no part-load curve is given, and by the simple battery model).
    pub dc_dc_efficiency: f64,
    /// Part-load efficiency of the DC/DC stage: (P / `p_rated_w`, efficiency) knots,
    /// ascending load. Empty: the constant `dc_dc_efficiency`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efficiency_curve: Vec<[f64; 2]>,
    /// Power the curve's load fractions refer to, W (default: the charge limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p_rated_w: Option<f64>,
}

impl BatteryPort {
    /// DC/DC efficiency at a transfer of `w` watts (linear between knots, held flat outside them).
    pub fn efficiency(&self, w: f64) -> f64 {
        let c = &self.efficiency_curve;
        if c.is_empty() { return self.dc_dc_efficiency; }
        let x = w.abs() / self.p_rated_w.unwrap_or(self.p_charge_max_w).max(1e-9);
        if x <= c[0][0] { return c[0][1]; }
        for k in c.windows(2) {
            if x <= k[1][0] { return k[0][1] + (k[1][1] - k[0][1]) * (x - k[0][0]) / (k[1][0] - k[0][0]).max(1e-12); }
        }
        c[c.len() - 1][1]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chemistry { Lfp, Nmc, LeadAcid, Other }

/// A battery unit (pack or modular system with its BMS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatterySpec {
    #[serde(default)]
    pub catalog_ref: Option<crate::plant::CatalogRef>,
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
    /// Cell or block data behind the pack (detailed model).
    #[serde(default)]
    pub cells: Option<BatteryCells>,
    /// Built-in converter of an integrated battery system (AC or DC side).
    #[serde(default)]
    pub converter: Option<Converter>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub unverified: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub datasheet_ref: String,
}

/// Cells (or blocks) of a pack: `series` × `parallel` per string, `strings` in parallel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryCells {
    pub chemistry_code: Option<u32>,
    pub cell_voltage_v: f64,
    pub series: u32,
    pub parallel: u32,
    #[serde(default = "one")]
    pub strings: u32,
    /// Internal resistance per cell, Ω.
    pub internal_resistance_ohm: f64,
    /// (discharge time h, capacity Ah) per cell, ascending time.
    pub capacity_ah_by_hours: Vec<[f64; 2]>,
    /// (depth of discharge %, cycles to end of life).
    pub cycles_by_dod: Vec<[f64; 2]>,
}
fn one() -> u32 { 1 }

impl BatteryCells {
    /// Pack nominal voltage, V.
    pub fn pack_voltage(&self) -> f64 { self.cell_voltage_v * self.series as f64 }
    /// Pack resistance, Ω (series cells add, parallel cells and strings divide).
    pub fn pack_resistance(&self) -> f64 { self.internal_resistance_ohm * self.series as f64 / (self.parallel.max(1) * self.strings.max(1)) as f64 }
    /// Capacity at a discharge time (h), Ah per cell, interpolated in log(time).
    pub fn capacity_ah(&self, hours: f64) -> f64 {
        let k = &self.capacity_ah_by_hours;
        if k.is_empty() { return 0.0; }
        if hours <= k[0][0] { return k[0][1]; }
        for w in k.windows(2) {
            if hours <= w[1][0] { let t = (hours.ln() - w[0][0].ln()) / (w[1][0].ln() - w[0][0].ln()); return w[0][1] + t * (w[1][1] - w[0][1]); }
        }
        k[k.len() - 1][1]
    }
    /// Pack energy at the 10-hour rate, kWh.
    pub fn energy_kwh(&self) -> f64 { self.pack_voltage() * self.capacity_ah(10.0) * (self.parallel.max(1) * self.strings.max(1)) as f64 / 1000.0 }
    /// Cycles to end of life at a depth of discharge (%), interpolated.
    pub fn cycles_at(&self, dod_pct: f64) -> Option<f64> {
        let k = &self.cycles_by_dod;
        if k.is_empty() { return None; }
        if dod_pct <= k[0][0] { return Some(k[0][1]); }
        for w in k.windows(2) { if dod_pct <= w[1][0] { let t = (dod_pct - w[0][0]) / (w[1][0] - w[0][0]); return Some(w[0][1] + t * (w[1][1] - w[0][1])); } }
        Some(k[k.len() - 1][1])
    }
}

/// Power converter of an integrated battery system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Converter {
    pub p_rated_w: f64,
    pub p_charge_max_w: f64,
    pub p_discharge_max_w: f64,
    /// (P / P_rated, efficiency) knots.
    pub efficiency_curve: Vec<[f64; 2]>,
    #[serde(default)]
    pub standby_w: f64,
}

/// A bidirectional battery inverter on the AC bus (AC coupling).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatteryInverterSpec {
    #[serde(default)]
    pub catalog_ref: Option<crate::plant::CatalogRef>,
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub provenance: Provenance,
    pub p_ac_max_w: f64,
    /// (P / P_rated, efficiency) knots; used in both directions.
    pub efficiency_curve: Vec<[f64; 2]>,
    pub standby_w: f64,
    /// Battery-side limits: voltage window and maximum current.
    #[serde(default)]
    pub battery_v_min: Option<f64>,
    #[serde(default)]
    pub battery_v_max: Option<f64>,
    #[serde(default)]
    pub battery_i_max: Option<f64>,
    #[serde(default)]
    pub datasheet_ref: String,
}

/// A backup generator with its fuel-rate curve.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratorSpec {
    #[serde(default)]
    pub catalog_ref: Option<crate::plant::CatalogRef>,
    pub id: String,
    pub manufacturer: String,
    pub model: String,
    pub provenance: Provenance,
    /// Rated and maximum apparent power, VA (real power assumes unity power factor).
    pub rated_va: f64,
    pub max_va: f64,
    pub phases: u32,
    pub voltage_v: f64,
    /// (load fraction of rated, fuel per hour) knots; the fuel volume unit comes from the source.
    pub fuel_curve: Vec<[f64; 2]>,
    #[serde(default)]
    pub fuel_unit: String,
    #[serde(default)]
    pub datasheet_ref: String,
}

impl GeneratorSpec {
    /// Fuel per hour at a delivered power (W); zero when idle.
    pub fn fuel_rate(&self, p_w: f64) -> f64 {
        if p_w <= 0.0 { return 0.0; }
        let x = p_w / self.rated_va;
        let k = &self.fuel_curve;
        if k.is_empty() { return 0.0; }
        if k.len() == 1 || x <= k[0][0] { return k[0][1]; }
        for w in k.windows(2) { if x <= w[1][0] { let t = (x - w[0][0]) / (w[1][0] - w[0][0]); return w[0][1] + t * (w[1][1] - w[0][1]); } }
        let n = k.len(); let (a, b) = (k[n - 2], k[n - 1]);
        b[1] + (x - b[0]) * (b[1] - a[1]) / (b[0] - a[0]).max(1e-9)
    }
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

    /// Limits of MPP tracker `i` (0-based).
    pub fn input(&self, i: usize) -> MpptInput {
        self.inputs.get(i).cloned().unwrap_or(MpptInput { v_min: self.mppt_v_min, v_max: self.mppt_v_max, i_max: self.i_max_per_mppt, i_sc_max: None, p_max_w: None })
    }

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
            inputs: vec![], pdc_nominal_w: None, v_dc_nominal: None, phases: None, standby_w: None, transformer: None, unverified: Default::default(),
            catalog_ref: None, start_power_w: None, eta_change_per_100v: None, mpp_matching: None, mppt_interconnection: None, i_dc_max: None, i_sc_dc_max: None, dc_connectors: None, v_ac: None,
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
        i.battery_port = Some(BatteryPort { v_min: 90.0, v_max: 500.0, p_charge_max_w: pac_kw * 1000.0, p_discharge_max_w: pac_kw * 1000.0, dc_dc_efficiency: 0.98, efficiency_curve: vec![], p_rated_w: None });
        i
    }

    /// AC efficiency at a DC input power (W) and DC voltage (V): the part-load curve, the
    /// voltage correction per 100 V from nominal, and the MPP adaptation efficiency.
    pub fn efficiency_v(&self, pdc_w: f64, v_dc: Option<f64>) -> f64 {
        let mut eta = self.efficiency(pdc_w);
        if let (Some(c), Some(vn), Some(v)) = (self.eta_change_per_100v, self.v_dc_nominal, v_dc) { if v > 0.0 { eta -= c * (v - vn).abs() / 100.0; } }
        if let Some([lo, hi]) = self.mpp_matching { let pn = self.pdc_nominal_w.unwrap_or_else(|| self.pdc0_w()); eta *= if pdc_w < 0.2 * pn { lo } else { hi }; }
        eta.clamp(0.0, 1.0)
    }

    /// AC efficiency at a DC input power (W), before clipping.
    pub fn efficiency(&self, pdc_w: f64) -> f64 {
        if pdc_w <= 0.0 { return 0.0; }
        if self.efficiency_curve.is_empty() {
            return solarcraft_pv::inverter::pvwatts_unclipped(pdc_w, self.pdc0_w(), self.eta_nominal, 0.9637) / pdc_w;
        }
        curve_efficiency(&self.efficiency_curve, pdc_w / self.pdc_nominal_w.unwrap_or_else(|| self.pdc0_w()))
    }
}

impl BatterySpec {
    pub fn generic_lfp(kwh: f64) -> Self {
        BatterySpec { id: format!("generic-bat-lfp-{kwh}kwh"), manufacturer: "Generic".into(), model: format!("LFP battery {kwh} kWh"), provenance: Provenance::Generic,
            chemistry: Chemistry::Lfp, capacity_kwh: kwh, nominal_voltage_v: 204.8, soc_min_pct: 5.0, soc_max_pct: 100.0,
            p_charge_max_kw: kwh * 0.5, p_discharge_max_kw: kwh * 0.5, roundtrip_efficiency: 0.95, self_discharge_pct_per_month: 2.0, cycle_life: 6000.0,
            cells: None, converter: None, unverified: Default::default(), datasheet_ref: String::new(), catalog_ref: None }
    }
}

impl BatteryInverterSpec {
    pub fn generic(kw: f64) -> Self {
        BatteryInverterSpec { id: format!("generic-batinv-{kw}kw"), manufacturer: "Generic".into(), model: format!("battery inverter {kw} kW"), provenance: Provenance::Generic,
            p_ac_max_w: kw * 1000.0, efficiency_curve: vec![[0.05, 0.85], [0.10, 0.91], [0.20, 0.94], [0.50, 0.955], [1.0, 0.95]], standby_w: 5.0, datasheet_ref: String::new(),
            catalog_ref: None, battery_v_min: None, battery_v_max: None, battery_i_max: None }
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
    #[serde(default)]
    pub generators: Vec<GeneratorSpec>,
}

impl Library {
    /// SolarCraft's generic defaults (illustrative values, not real products).
    pub fn generic() -> Self {
        let mut m = ModuleSpec {
            id: "generic-mod-400-mono".into(), manufacturer: "Generic".into(), model: "400 W mono 72-cell".into(),
            provenance: Provenance::Generic, technology: "mono-c-Si".into(), cells_in_series: 72, length_m: 2.0, width_m: 1.0,
            v_oc: 48.9, i_sc: 10.6, v_mp: 40.4, i_mp: 9.9, alpha_sc: 0.0053, beta_voc: -0.140, gamma_pmp: -0.36, model_params: None,
            max_system_voltage_v: Some(1000.0), bypass_diodes: Some(3), bypass_diode_voltage_v: None, half_cell: false, bifaciality: None, low_light: None,
            thickness_m: None, weight_kg: None, unverified: Default::default(), datasheet_ref: String::new(),
            catalog_ref: None, voc_initial_increase: None, requires_transformer_inverter: false, nameplate_w: None, description: String::new(),
        };
        m.fit().expect("generic module fits");
        Library { modules: vec![m], inverters: vec![], batteries: vec![], battery_inverters: vec![], generators: vec![] }
    }
    pub fn module(&self, id: &str) -> Option<&ModuleSpec> { self.modules.iter().find(|m| m.id == id) }
    pub fn inverter(&self, id: &str) -> Option<&InverterSpec> { self.inverters.iter().find(|i| i.id == id) }
    pub fn battery(&self, id: &str) -> Option<&BatterySpec> { self.batteries.iter().find(|i| i.id == id) }
    pub fn generator(&self, id: &str) -> Option<&GeneratorSpec> { self.generators.iter().find(|i| i.id == id) }
    pub fn battery_inverter(&self, id: &str) -> Option<&BatteryInverterSpec> { self.battery_inverters.iter().find(|i| i.id == id) }
    pub fn ensure_inverter(&mut self, inv: InverterSpec) -> String {
        if self.inverter(&inv.id).is_none() { self.inverters.push(inv.clone()); }
        inv.id
    }
}
