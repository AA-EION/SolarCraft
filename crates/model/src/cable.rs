// SPDX-License-Identifier: GPL-3.0-or-later
//! Resistive cable estimates at an entered operating point.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind { DcString, DcMain, AcSingle, AcThree }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metal { Copper, Aluminium }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Circuit {
    pub label: String,
    pub kind: Kind,
    pub metal: Metal,
    /// One-way distance, per conductor.
    pub length_m: f64,
    /// Additional series conductor length (for example module jumpers).
    #[serde(default)]
    pub extra_m: f64,
    pub area_mm2: f64,
    pub temperature_c: f64,
    /// DC, line-to-neutral (single-phase) or line-to-line (three-phase).
    pub voltage_v: f64,
    /// Operating current per circuit, per phase for three phase.
    pub current_a: f64,
    /// Identical independent circuits at this operating point.
    pub circuits: u32,
    pub target_drop_pct: f64,
    /// Source route key for a row imported from the 3D design.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_route: Option<String>,
    /// Published ampacity for the entered conductor area and base installation condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ampacity_a: Option<f64>,
    #[serde(default = "unity")]
    pub derating: f64,
    /// Optional published cross sections and base ampacities for this conductor family.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Conductor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Conductor { pub area_mm2: f64, pub ampacity_a: f64 }

fn unity() -> f64 { 1.0 }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HourlyLosses { pub dc_pct: Option<f64>, pub ac_pct: Option<f64> }

impl HourlyLosses {
    pub fn is_empty(&self) -> bool { self.dc_pct.is_none() && self.ac_pct.is_none() }
}

#[derive(Debug, Serialize)]
pub struct Estimate {
    pub drop_v: f64,
    pub drop_pct: f64,
    pub loss_w: f64,
    pub conductor_m: f64,
    pub area_for_target_mm2: f64,
    pub required_base_ampacity_a: f64,
    pub allowed_a: Option<f64>,
    pub ampacity_ok: Option<bool>,
    pub drop_ok: bool,
    pub suggested: Option<Conductor>,
}

impl Circuit {
    pub fn is_dc(&self) -> bool { matches!(self.kind, Kind::DcString | Kind::DcMain) }

    pub fn estimate(&self) -> Result<Estimate, &'static str> {
        if self.label.trim().is_empty() || self.label.len() > 200 { return Err("Enter a cable name under 200 bytes."); }
        if !(0.0..=100_000.0).contains(&self.length_m) || !(0.0..=100_000.0).contains(&self.extra_m)
            || self.length_m + self.extra_m <= 0.0 || (!self.is_dc() && self.extra_m != 0.0)
            || !(0.01..=2_000.0).contains(&self.area_mm2) || !(-50.0..=150.0).contains(&self.temperature_c)
            || !(0.01..=20_000.0).contains(&self.voltage_v) || !(0.01..=20_000.0).contains(&self.current_a)
            || self.candidates.len() > 100 || self.candidates.iter().any(|s| !(0.01..=2_000.0).contains(&s.area_mm2) || !(0.01..=20_000.0).contains(&s.ampacity_a))
            || !(0.01..=1.0).contains(&self.derating) || self.ampacity_a.is_some_and(|a| !(0.01..=20_000.0).contains(&a))
            || !(1..=10_000).contains(&self.circuits) || !(0.01..=100.0).contains(&self.target_drop_pct) {
            return Err("Check circuit length, area, voltage, current, temperature, count and voltage-drop target.");
        }
        let (rho20, alpha) = match self.metal { Metal::Copper => (0.017241, 0.00393), Metal::Aluminium => (0.028264, 0.00403) };
        let rho = rho20 * (1.0 + alpha * (self.temperature_c - 20.0));
        let three = self.kind == Kind::AcThree;
        let loop_m = 2.0 * self.length_m + self.extra_m;
        let drop_m = if three { 3.0_f64.sqrt() * self.length_m } else { loop_m };
        let loss_m = if three { 3.0 * self.length_m } else { loop_m };
        let drop_v = self.current_a * rho * drop_m / self.area_mm2;
        let drop_pct = 100.0 * drop_v / self.voltage_v;
        let allowed_a = self.ampacity_a.map(|base| base * self.derating);
        let area_for_target_mm2 = self.area_mm2 * drop_pct / self.target_drop_pct;
        let required_base_ampacity_a = self.current_a / self.derating;
        let suggested = self.candidates.iter().filter(|s| s.area_mm2 >= area_for_target_mm2 && s.ampacity_a >= required_base_ampacity_a)
            .min_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2)).copied();
        Ok(Estimate {
            drop_v, drop_pct, loss_w: self.current_a.powi(2) * rho * loss_m / self.area_mm2 * f64::from(self.circuits),
            conductor_m: loss_m * f64::from(self.circuits),
            area_for_target_mm2, required_base_ampacity_a, suggested,
            allowed_a, ampacity_ok: allowed_a.map(|a| a >= self.current_a), drop_ok: drop_pct <= self.target_drop_pct,
        })
    }
}
