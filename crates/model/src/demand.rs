// SPDX-License-Identifier: GPL-3.0-or-later
//! Configurations for additional hourly demand and stand-alone dispatch.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeStrategy {
    #[default]
    Immediate,
    SolarPreferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ev {
    pub daily_kwh: f64,
    pub charger_kw: f64,
    pub efficiency: f64,
    pub arrival_hour: u32,
    pub departure_hour: u32,
    #[serde(default)]
    pub strategy: ChargeStrategy,
}
impl Ev {
    pub fn valid(&self) -> bool {
        (0.01..=2000.0).contains(&self.daily_kwh) && (0.01..=1000.0).contains(&self.charger_kw)
            && (0.05..=1.0).contains(&self.efficiency) && self.arrival_hour < 24 && self.departure_hour < 24
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Heating {
    pub setpoint_c: f64,
    pub heat_loss_kw_per_k: f64,
    /// Constant thermal capacity above the minimum operating temperature.
    pub thermal_kw: f64,
    pub cutoff_c: f64,
    pub cop_at_0c: f64,
    pub cop_at_10c: f64,
    pub resistance_backup_kw: f64,
}
impl Heating {
    pub fn valid(&self) -> bool {
        (-10.0..=40.0).contains(&self.setpoint_c) && (0.001..=5000.0).contains(&self.heat_loss_kw_per_k)
            && (0.01..=10_000.0).contains(&self.thermal_kw) && (-80.0..=40.0).contains(&self.cutoff_c)
            && [self.cop_at_0c, self.cop_at_10c].iter().all(|cop| (1.0..=12.0).contains(cop))
            && (0.0..=10_000.0).contains(&self.resistance_backup_kw)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandPlan {
    pub minimum_generator_pct: f64,
    pub minimum_run_hours: u32,
    /// Enable generator charging through the AC battery converter when SOC crosses start_soc_pct.
    pub charge_battery: bool,
    pub start_soc_pct: f64,
    pub stop_soc_pct: f64,
    pub fuel_price: f64,
    pub fuel_escalation_pct: f64,
    pub generator_investment: f64,
}
impl IslandPlan {
    pub fn valid(&self) -> bool {
        (0.0..=100.0).contains(&self.minimum_generator_pct) && self.minimum_run_hours <= 24
            && (self.minimum_run_hours == 0 || self.minimum_generator_pct > 0.0)
            && (0.0..=100.0).contains(&self.start_soc_pct) && (0.0..=100.0).contains(&self.stop_soc_pct)
            && (!self.charge_battery || self.start_soc_pct < self.stop_soc_pct)
            && (0.0..=1e8).contains(&self.fuel_price) && (-99.0..=100.0).contains(&self.fuel_escalation_pct)
            && (0.0..=1e12).contains(&self.generator_investment)
    }
}
