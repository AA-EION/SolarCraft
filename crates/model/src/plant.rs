// SPDX-License-Identifier: GPL-3.0-or-later
//! Plant configuration beyond the PV arrays: explicit inverter/MPPT stringing,
//! grid connection, consumption, battery storage with allocated reserves,
//! tariffs, economics and design variants.
//!
//! Every section is optional on disk (`#[serde(default)]`), so schema-1
//! projects open unchanged and behave exactly as before.
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- stringing

/// Where an equipment record or tariff came from in an external catalog, pinned to one revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogRef {
    /// Entity family (module, pvinverter, batterysystem, batteryinverter, generator, tariff, …).
    pub family: String,
    pub id: String,
    pub version: i64,
    /// SHA-256 of the imported source file and its dataset name.
    pub source_sha256: String,
    pub dataset: String,
}

/// A group of identical strings from one array on one MPPT input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StringGroup {
    pub array_id: String,
    pub modules_per_string: u32,
    pub strings: u32,
    /// Segments bearing the same ID within this MPPT share one series current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_chain: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optimizer: Option<BuckOptimizer>,
}

/// Archived module-level buck converter and its documented operating envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuckOptimizer {
    pub catalog_ref: CatalogRef,
    pub model: String,
    pub rated_w: f64,
    pub mppt_min_v: f64,
    pub mppt_max_v: f64,
    pub input_max_v: f64,
    pub input_max_a: f64,
    pub output_max_v: f64,
    pub output_max_a: f64,
    pub efficiency_curve: Vec<[f64; 2]>,
}

impl BuckOptimizer {
    pub fn valid(&self) -> bool {
        !self.model.trim().is_empty() && !self.catalog_ref.id.is_empty()
            && [self.rated_w, self.mppt_min_v, self.mppt_max_v, self.input_max_v,
                self.input_max_a, self.output_max_v, self.output_max_a].into_iter()
                .all(|v| (0.01..=100_000.0).contains(&v))
            && self.mppt_min_v < self.mppt_max_v && self.mppt_max_v <= self.input_max_v
            && !self.efficiency_curve.is_empty() && self.efficiency_curve.len() <= 32
            && self.efficiency_curve.iter().all(|p| (0.0..=1.0).contains(&p[0]) && (0.01..=1.0).contains(&p[1]))
            && self.efficiency_curve.windows(2).all(|p| p[1][0] > p[0][0])
    }
}

/// Positions of each physical string pattern in a single MPPT. Ordinary
/// groups still represent separate strings wired in parallel.
pub fn physical_strings(groups: &[StringGroup]) -> Vec<Vec<usize>> {
    let mut chains = std::collections::HashMap::new();
    let mut strings: Vec<Vec<usize>> = vec![];
    for (index, group) in groups.iter().enumerate() {
        let string = group.series_chain.map_or(strings.len(), |id| *chains.entry(id).or_insert(strings.len()));
        if string == strings.len() { strings.push(vec![]); }
        strings[string].push(index);
    }
    strings
}

/// One physical inverter and the strings on each of its MPPT inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InverterUnit {
    pub id: String,
    pub name: String,
    /// Equipment record in the project library.
    pub inverter_id: String,
    /// `mppts[i]` lists the string groups on MPPT input `i`.
    pub mppts: Vec<Vec<StringGroup>>,
    /// MPPT inputs connected in parallel to act as one tracker (0-based indices per group).
    #[serde(default)]
    pub interconnected: Vec<Vec<usize>>,
}

/// Electrical design. With `manual == false` the stringing is derived from
/// each array's string count and inverter selection (Easy mode); with
/// `manual == true` the user's assignment is used as entered and validated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Electrical {
    pub manual: bool,
    pub inverters: Vec<InverterUnit>,
}

// ---------------------------------------------------------------- grid

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeedIn {
    /// Self-consumption first; only the surplus is exported.
    #[default]
    Surplus,
    /// All PV energy is exported; consumption is supplied from the grid.
    Full,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub feed_in: FeedIn,
    /// Active export limit at the connection point, kW (None = unlimited).
    pub export_limit_kw: Option<f64>,
    /// 1 or 3.
    pub phases: u32,
    /// Line-to-neutral voltage, V.
    pub voltage_v: f64,
}
impl Default for Grid { fn default() -> Self { Grid { feed_in: FeedIn::Surplus, export_limit_kw: None, phases: 1, voltage_v: 230.0 } } }

// ---------------------------------------------------------------- loads

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyntheticProfile {
    /// Morning and evening peaks, more use at weekends.
    Residential,
    /// Weekday working hours.
    Office,
    /// Opening hours Monday to Saturday.
    Retail,
    /// Constant load.
    Flat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoadProfile {
    /// No consumption modelled (pure grid feed-in plant).
    #[default]
    None,
    /// SolarCraft's own parametric daily/weekly/seasonal shape scaled to the annual total.
    Synthetic { profile: SyntheticProfile },
    /// Imported interval series stored as a project blob (kWh per interval, local standard time).
    Series {
        blob: String,
        name: String,
        step_minutes: u32,
        values: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_year: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loads {
    pub profile: LoadProfile,
    /// Annual consumption, kWh (synthetic profiles; imported series keep their own total).
    pub annual_kwh: f64,
    /// Share of the load that is critical and must be supplied during outages (0–1).
    pub critical_fraction: f64,
}
impl Default for Loads { fn default() -> Self { Loads { profile: LoadProfile::None, annual_kwh: 4000.0, critical_fraction: 0.3 } } }

// ---------------------------------------------------------------- storage

/// How the battery is electrically connected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Coupling {
    /// On the DC battery port of a hybrid inverter (`unit_id` in [`Electrical`]).
    /// PV can charge the battery before conversion, including energy the
    /// inverter would otherwise clip.
    DcHybrid { unit_id: String },
    /// On the AC bus through its own bidirectional battery inverter.
    AcCoupled { battery_inverter_id: String },
    /// Integrated battery system on the AC bus through its own built-in converter.
    Integrated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BatteryModel {
    /// Energy bucket: constant efficiencies and power limits, self-discharge.
    #[default]
    Simple,
    /// Adds the converter part-load efficiency curve, SOC-dependent charge and
    /// discharge tapering and throughput ageing.
    Detailed,
}

/// Uses of stored energy. Each kWh in the battery belongs to exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EnergyUse { Backup, SelfConsumption, GridServices }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GridService {
    #[default]
    None,
    /// Discharge the grid-services allocation to the grid in a daily window (local hours, end exclusive).
    ExportWindow { start_hour: u32, end_hour: u32, power_kw: f64 },
    /// Discharge the grid-services allocation to keep grid import below a limit.
    PeakShaving { import_limit_kw: f64 },
}

/// A grid outage. `start_hour` is the hour of the year in local standard time (0–8759).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outage { pub start_hour: u32, pub hours: u32 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Storage {
    pub battery_id: String,
    /// Identical battery units in parallel.
    pub units: u32,
    pub coupling: Coupling,
    pub model: BatteryModel,
    /// Operating SOC window, % of nominal capacity (within the battery's hardware limits).
    pub soc_min_pct: f64,
    pub soc_max_pct: f64,
    pub initial_soc_pct: f64,
    /// Allocation of the usable energy (between soc_min and soc_max), %.
    /// Self-consumption receives the remainder: 100 − backup − grid services.
    pub backup_reserve_pct: f64,
    pub grid_services_pct: f64,
    /// Order in which charging energy fills the allocations.
    pub charge_priority: Vec<EnergyUse>,
    /// Recharge the backup reserve from the grid when it falls below 90 % of its allocation
    /// (hysteresis: charging continues until full).
    pub grid_charge_backup: bool,
    pub grid_service: GridService,
    pub outages: Vec<Outage>,
    /// Backup generator that supplies critical load the battery cannot cover during outages.
    #[serde(default)]
    pub generator_id: Option<String>,
}

impl Storage {
    pub fn self_consumption_pct(&self) -> f64 { 100.0 - self.backup_reserve_pct - self.grid_services_pct }
}

// ---------------------------------------------------------------- tariff & economics

/// A time-of-use period in local standard time. Hours: `start_hour` inclusive, `end_hour`
/// exclusive (end < start wraps past midnight). `months` 1–12; empty = all months.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TouPeriod {
    pub name: String,
    pub price: f64,
    pub start_hour: u32,
    pub end_hour: u32,
    pub weekdays: bool,
    pub weekends: bool,
    #[serde(default)]
    pub months: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportCompensation {
    None,
    /// Fixed price per exported kWh.
    Fixed { price: f64 },
    /// Monthly net metering: exports offset imports of the same month at the import
    /// price; the remaining surplus is paid at `surplus_price`.
    NetMetering { surplus_price: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tariff {
    pub name: String,
    pub currency: String,
    /// Import price per kWh outside any time-of-use period.
    pub import_price: f64,
    #[serde(default)]
    pub tou: Vec<TouPeriod>,
    pub export: ExportCompensation,
    pub fixed_per_month: f64,
    /// Charge per kW of the month's highest hourly import.
    pub demand_charge_per_kw_month: f64,
    /// Annual price escalation, %.
    pub import_escalation_pct: f64,
    pub export_escalation_pct: f64,
    /// Zone schedule (time-of-use grid) that sets the import price per hour when present.
    #[serde(default)]
    pub zones: Option<ZoneSchedule>,
    #[serde(default)]
    pub catalog_ref: Option<CatalogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_catalog_ref: Option<CatalogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_import_rate: Option<CatalogTierRate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_base_charge: Option<CatalogTierRate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_export_rate: Option<CatalogTierRate>,
}

/// A pinned rate table and the value originally written to the editable tariff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogTierRate {
    pub tiers: Vec<[f64; 3]>,
    pub zoning: bool,
    pub applied_rate: f64,
    pub unit_scale: f64,
}

impl CatalogTierRate {
    pub fn rate(tiers: &[[f64; 3]], quantity: f64, zoning: bool) -> Option<f64> {
        let mut sorted = tiers.to_vec();
        sorted.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let first = sorted.first()?;
        if sorted.len() == 1 || quantity <= first[0] || quantity <= 0.0 { return Some(first[2]); }
        if !zoning {
            return sorted.iter().rev().find(|band| quantity >= band[0]).map(|band| band[2]);
        }
        let mut cost = first[0] * first[2];
        for (i, band) in sorted.iter().enumerate() {
            let end = sorted.get(i + 1).map_or(quantity, |next| next[0].min(quantity));
            cost += (end - band[0]).max(0.0) * band[2];
        }
        Some(cost / quantity)
    }

    fn resolve(&self, current_rate: f64, quantity: f64) -> f64 {
        if current_rate != self.applied_rate { return current_rate; }
        Self::rate(&self.tiers, quantity, self.zoning)
            .map_or(current_rate, |rate| rate * self.unit_scale)
    }
}

impl Tariff {
    pub fn for_run(&self, annual_load_kwh: f64, installed_kwp: f64) -> Self {
        let mut tariff = self.clone();
        if let Some(rule) = &self.catalog_import_rate {
            tariff.import_price = rule.resolve(self.import_price, annual_load_kwh);
        }
        if let Some(rule) = &self.catalog_base_charge {
            tariff.fixed_per_month = rule.resolve(self.fixed_per_month, annual_load_kwh);
        }
        if let (Some(rule), ExportCompensation::Fixed { price }) = (&self.catalog_export_rate, &mut tariff.export) {
            *price = rule.resolve(*price, installed_kwp);
        }
        tariff
    }
}

/// Rate zones per hour: rows of 12 months × 24 hours (288 slots, zero-based month × 24 + hour),
/// a row selected per weekday, zone codes (1-based) selecting a price.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneSchedule {
    /// Row index for each weekday in .NET order: Sunday = 0 … Saturday = 6.
    pub weekday_rows: [usize; 7],
    pub rows: Vec<Vec<u32>>,
    /// Price per zone code (index = code − 1) and its display name.
    pub zone_prices: Vec<f64>,
    #[serde(default)]
    pub zone_names: Vec<String>,
    /// Source offset in minutes, preserved; not applied by the hourly adapter.
    #[serde(default)]
    pub time_plus_minutes: Option<i64>,
    /// The original encoded payload (Base64 of UTF-16LE JSON).
    #[serde(default)]
    pub raw_encoded: String,
}

impl ZoneSchedule {
    /// Zone code at a weekday (0 = Monday … 6 = Sunday), zero-based month and hour.
    pub fn zone(&self, weekday_mon0: u32, month0: u32, hour: u32) -> Option<u32> {
        let dotnet = ((weekday_mon0 + 1) % 7) as usize;
        let row = self.rows.get(*self.weekday_rows.get(dotnet)?)?;
        row.get((month0 * 24 + hour) as usize).copied()
    }
    pub fn price(&self, weekday_mon0: u32, month0: u32, hour: u32) -> Option<f64> {
        let z = self.zone(weekday_mon0, month0, hour)?;
        self.zone_prices.get(z.checked_sub(1)? as usize).copied()
    }
}

impl Default for Tariff {
    fn default() -> Self {
        Tariff { name: "Flat tariff".into(), currency: "EUR".into(), import_price: 0.30, tou: vec![], export: ExportCompensation::Fixed { price: 0.08 },
                 fixed_per_month: 0.0, demand_charge_per_kw_month: 0.0, import_escalation_pct: 2.0, export_escalation_pct: 0.0, zones: None, catalog_ref: None, export_catalog_ref: None, catalog_import_rate: None, catalog_base_charge: None, catalog_export_rate: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loan { pub amount: f64, pub rate_pct: f64, pub years: u32 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Economics {
    pub horizon_years: u32,
    pub discount_rate_pct: f64,
    /// Investment: fixed part + per kWp DC + per kWh of battery.
    pub cost_fixed: f64,
    pub cost_per_kwp: f64,
    pub cost_per_battery_kwh: f64,
    /// Operation and maintenance, % of investment per year (escalated with `om_escalation_pct`).
    pub om_pct_of_investment: f64,
    pub om_escalation_pct: f64,
    /// Replacements: (year, cost). Defaults: inverters in year 13, battery in year 12.
    pub inverter_replacement_year: u32,
    pub inverter_replacement_cost: f64,
    pub battery_replacement_year: u32,
    pub battery_replacement_cost: f64,
    /// PV output degradation per year, %.
    pub degradation_pct_per_year: f64,
    pub subsidy: f64,
    pub loan: Option<Loan>,
}

impl Default for Economics {
    fn default() -> Self {
        Economics { horizon_years: 20, discount_rate_pct: 4.0, cost_fixed: 1500.0, cost_per_kwp: 1100.0, cost_per_battery_kwh: 450.0,
                    om_pct_of_investment: 1.0, om_escalation_pct: 2.0, inverter_replacement_year: 13, inverter_replacement_cost: 0.0,
                    battery_replacement_year: 12, battery_replacement_cost: 0.0, degradation_pct_per_year: 0.4, subsidy: 0.0, loan: None }
    }
}

// ---------------------------------------------------------------- variants

/// A named snapshot of the design (everything except variants and customer data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    pub id: String,
    pub name: String,
    pub created_unix: i64,
    pub design: serde_json::Value,
}
