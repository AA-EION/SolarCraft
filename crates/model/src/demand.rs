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

/// Identical vehicles with one weekly connection pattern. Days are numbered Monday = 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetGroup {
    pub name: String,
    pub vehicles: u32,
    pub battery_kwh: f64,
    pub initial_soc_pct: f64,
    pub departure_soc_pct: f64,
    pub trip_kwh: f64,
    pub weekdays: Vec<u32>,
    pub arrival_hour: u32,
    pub departure_hour: u32,
    pub charge_kw: f64,
    pub charge_efficiency: f64,
    #[serde(default)]
    pub discharge_kw: f64,
    #[serde(default = "unity_efficiency")]
    pub discharge_efficiency: f64,
    #[serde(default)]
    pub strategy: ChargeStrategy,
}

fn unity_efficiency() -> f64 { 1.0 }

impl FleetGroup {
    pub fn valid(&self) -> bool {
        !self.name.trim().is_empty() && self.name.len() <= 100 && (1..=1000).contains(&self.vehicles)
            && (0.1..=1000.0).contains(&self.battery_kwh) && (0.0..=100.0).contains(&self.initial_soc_pct)
            && (0.0..=100.0).contains(&self.departure_soc_pct) && (0.0..=self.battery_kwh).contains(&self.trip_kwh)
            && !self.weekdays.is_empty() && self.weekdays.len() <= 7 && self.weekdays.iter().all(|&d| d < 7)
            && (0..7).filter(|d| self.weekdays.contains(d)).count() == self.weekdays.len()
            && self.arrival_hour < 24 && self.departure_hour < 24
            && (0.01..=500.0).contains(&self.charge_kw) && (0.05..=1.0).contains(&self.charge_efficiency)
            && (0.0..=500.0).contains(&self.discharge_kw) && (0.05..=1.0).contains(&self.discharge_efficiency)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fleet {
    pub groups: Vec<FleetGroup>,
    pub site_charge_kw: f64,
    /// V2H site limit. Zero disables discharge for the fleet.
    #[serde(default)]
    pub site_discharge_kw: f64,
    #[serde(default)]
    pub v2g: Option<VehicleExport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VehicleExport {
    pub limit_kw: f64,
    pub start_hour: u32,
    pub end_hour: u32,
    /// Local start-day indices, Monday = 0. Empty means every day.
    #[serde(default)]
    pub weekdays: Vec<u32>,
}

impl VehicleExport {
    pub fn valid(&self) -> bool {
        (0.01..=100_000.0).contains(&self.limit_kw) && self.start_hour < 24 && self.end_hour < 24
            && self.weekdays.iter().all(|&d| d < 7)
            && (0..7).filter(|d| self.weekdays.contains(d)).count() == self.weekdays.len()
    }

    pub fn active(&self, hour: u32, day: u32) -> bool {
        let after = hour >= self.start_hour;
        let wraps = self.end_hour <= self.start_hour;
        let in_window = if wraps { after || hour < self.end_hour } else { after && hour < self.end_hour };
        let start_day = if wraps && !after { (day + 6) % 7 } else { day };
        in_window && (self.weekdays.is_empty() || self.weekdays.contains(&start_day))
    }
}

impl Fleet {
    pub fn valid(&self) -> bool {
        (1..=32).contains(&self.groups.len()) && self.groups.iter().all(FleetGroup::valid)
            && (0.01..=100_000.0).contains(&self.site_charge_kw)
            && (0.0..=100_000.0).contains(&self.site_discharge_kw)
            && self.v2g.as_ref().is_none_or(|p| p.valid() && self.site_discharge_kw > 0.0)
            && self.groups.iter().enumerate().all(|(i, g)| self.groups[..i].iter().all(|prior| prior.name != g.name))
    }
}

/// A lumped usable heat store charged through the heat pump from PV surplus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeatStore {
    pub capacity_kwh: f64,
    pub initial_kwh: f64,
    pub charge_kw: f64,
    pub discharge_kw: f64,
    pub standing_loss_pct_per_day: f64,
    #[serde(default)]
    pub daily_hot_water_kwh: f64,
    #[serde(default)]
    pub hot_water_hour: u32,
    #[serde(default)]
    pub layers: Option<TankLayers>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<ThermalControls>,
}

/// Mixed tank with a separate combustion boiler and a solar electric heater.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThermalControls {
    pub ambient_c: f64,
    pub solar_target_c: f64,
    pub boiler_target_c: f64,
    pub boiler_limit_kw: Option<f64>,
    pub emitter: ThermalEmitter,
    pub space: Option<SpaceHeatSchedule>,
    pub hot_water: Option<HotWaterWeights>,
    pub sanitation: Option<TankSanitation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThermalEmitter {
    Resistance { modulating: bool, steps_kw: Vec<f64> },
    HeatPump { pv_only: bool, cop_a_minus7: f64, cop_a2: f64, cop_a7: f64, blocked_seconds: Vec<[u32; 2]> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceHeatSchedule {
    pub threshold_c: f64,
    /// Zero-based days of year, start included and end excluded.
    pub inactive_days: [u32; 2],
    pub night_seconds: Option<[u32; 2]>,
    pub night_setback_c: f64,
}

impl SpaceHeatSchedule {
    pub fn degree_hours(&self, setpoint_c: f64, temperatures: impl IntoIterator<Item = f64>) -> Vec<f64> {
        let mut recent = [0.0; 6];
        temperatures.into_iter().enumerate().map(|(h, temperature)| {
            recent[h % 6] = temperature;
            let count = (h + 1).min(recent.len());
            let average = recent[..count].iter().sum::<f64>() / count as f64;
            if average >= self.threshold_c || in_window(h as u32 / 24, self.inactive_days) { return 0.0; }
            let is_night = self.night_seconds.is_some_and(|range| in_window(h as u32 % 24 * 3600, range));
            let setback = if is_night { self.night_setback_c } else { 0.0 };
            (setpoint_c - setback - temperature).max(0.0)
        }).collect()
    }

    fn valid(&self) -> bool {
        (-50.0..=50.0).contains(&self.threshold_c) && self.inactive_days.iter().all(|&d| d <= 365)
            && self.night_seconds.is_none_or(|r| r.iter().all(|&s| s <= 86400))
            && (0.0..=30.0).contains(&self.night_setback_c)
    }
}

fn in_window(position: u32, [start, end]: [u32; 2]) -> bool {
    if start <= end { start <= position && position < end } else { position >= start || position < end }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotWaterWeights {
    pub month: [f64; 12],
    pub weekday: [f64; 7],
    /// Monday through Sunday, each with 24 local standard-time weights.
    pub hour: [[f64; 24]; 7],
}

impl HotWaterWeights {
    fn valid(&self) -> bool {
        let nonzero = |v: &[f64]| v.iter().all(|x| (0.0..=1e8).contains(x)) && v.iter().any(|&x| x > 0.0);
        nonzero(&self.month) && nonzero(&self.weekday)
            && self.hour.iter().zip(self.weekday).all(|(h, w)| w == 0.0 || nonzero(h))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TankSanitation {
    pub target_c: f64,
    pub start_second: u32,
    pub interval_seconds: u32,
}

impl ThermalControls {
    fn valid(&self) -> bool {
        let emitter = match &self.emitter {
            ThermalEmitter::Resistance { steps_kw, .. } => !steps_kw.is_empty() && steps_kw.len() <= 24
                && steps_kw.iter().all(|v| (0.01..=10_000.0).contains(v)),
            ThermalEmitter::HeatPump { cop_a_minus7, cop_a2, cop_a7, blocked_seconds, .. } =>
                [cop_a_minus7, cop_a2, cop_a7].into_iter().all(|cop| (1.0..=12.0).contains(cop))
                    && blocked_seconds.len() <= 3 && blocked_seconds.iter().flatten().all(|&s| s <= 86400),
        };
        (0.0..=45.0).contains(&self.ambient_c) && (self.ambient_c + 5.0..=95.0).contains(&self.solar_target_c)
            && (self.ambient_c..=self.solar_target_c).contains(&self.boiler_target_c)
            && self.boiler_limit_kw.is_none_or(|kw| (0.0..=10_000.0).contains(&kw))
            && self.space.as_ref().is_none_or(SpaceHeatSchedule::valid)
            && self.hot_water.as_ref().is_none_or(HotWaterWeights::valid)
            && self.sanitation.is_none_or(|s| (self.boiler_target_c..=self.solar_target_c).contains(&s.target_c)
                && s.start_second < s.interval_seconds && (3600..=365 * 86400).contains(&s.interval_seconds))
            && emitter
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TankLayers {
    pub top_fraction: f64,
    pub ambient_c: f64,
    pub target_c: f64,
    pub space_supply_c: f64,
    pub hot_water_supply_c: f64,
    pub conductance_w_per_k: f64,
}

impl TankLayers {
    pub fn valid(&self) -> bool {
        (0.05..=0.95).contains(&self.top_fraction) && (0.0..=40.0).contains(&self.ambient_c)
            && (20.0..=95.0).contains(&self.target_c) && self.target_c - self.ambient_c >= 5.0
            && (self.ambient_c..self.target_c).contains(&self.space_supply_c)
            && (self.space_supply_c..self.target_c).contains(&self.hot_water_supply_c)
            && (0.0..=5000.0).contains(&self.conductance_w_per_k)
    }
}

impl HeatStore {
    pub fn valid(&self) -> bool {
        (0.01..=100_000.0).contains(&self.capacity_kwh) && (0.0..=self.capacity_kwh).contains(&self.initial_kwh)
            && (0.01..=10_000.0).contains(&self.charge_kw) && (0.01..=10_000.0).contains(&self.discharge_kw)
            && (0.0..100.0).contains(&self.standing_loss_pct_per_day)
            && (0.0..=10_000.0).contains(&self.daily_hot_water_kwh) && self.hot_water_hour < 24
            && self.layers.as_ref().is_none_or(TankLayers::valid)
            && self.controls.as_ref().is_none_or(|c| self.layers.is_none() && c.valid())
    }
}

/// One electrically isolated AC island. PV inverter unit IDs cannot belong to two clusters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IslandCluster {
    pub name: String,
    pub inverter_units: Vec<String>,
    pub load_share_pct: f64,
    pub storage: crate::plant::Storage,
    #[serde(default)]
    pub controls: Option<IslandPlan>,
    /// With an explicit load, this island is excluded from the shared-percent allocation.
    #[serde(default)]
    pub load: Option<crate::plant::Loads>,
}
