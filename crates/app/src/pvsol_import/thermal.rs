// SPDX-License-Identifier: GPL-3.0-or-later
//! Import the mixed storage tank, solar electricity heater and fuel boiler.
use super::{children, flag, number, path_node, value, Channel, HOURS, WS_PER_KWH};
use crate::{e, Result};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use solarcraft_engine::{calendar::local_time, SimResult};
use solarcraft_model::{
    demand::{BuildingHeatGains, HeatStore, Heating, HotWaterWeights, SpaceHeatSchedule, TankSanitation, ThermalControls, ThermalEmitter},
    plant::Variant,
    workflow::SystemType,
    Project,
};
use solarcraft_weather::Weather;
use std::collections::BTreeMap;
use std::f64::consts::PI;

const PUMP_ELECTRIC: &str = "heat_pump_electricity";
const PUMP_SOLAR: &str = "heat_pump_direct_solar";
const PUMP_HEAT: &str = "heat_pump_heat";
const BOILER_HEAT: &str = "boiler_heat";
const TANK_LOSSES: &str = "thermal_storage_losses";
const ROD_ELECTRIC: &str = "heating_rod_electricity";
// A reporting cutoff for classifying hourly means, independent of pump controls.
const SWITCHING_THRESHOLD_W: f64 = 1.0;
const SWITCHING_THRESHOLD_KWH: f64 = SWITCHING_THRESHOLD_W / 1000.0;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct ArchivedSwitching {
    hourly_rows: usize,
    reporting_threshold_w: f64,
    pump_on_hours: usize,
    pv_surplus_hours: usize,
    pump_on_with_surplus_hours: usize,
    pump_on_without_surplus_hours: usize,
    pump_kwh_covered_by_concurrent_pv: f64,
    pump_kwh_beyond_concurrent_pv: f64,
    source: String,
}

pub(super) fn read_reference(root: Node<'_, '_>, saved: &mut BTreeMap<String, Channel>) {
    let Some(thermal) = path_node(root, &["ThermalResult"]) else { return; };
    for (channel, tag) in [
        (PUMP_ELECTRIC, "HeatpumpElectricalEnergyConsumption"),
        (PUMP_SOLAR, "HeatpumpElectricalSolarEnergyConsumption"),
        (PUMP_HEAT, "HeatpumpThermalEnergyDelivery"),
        (BOILER_HEAT, "BoilerEnergyDelivery"),
        (TANK_LOSSES, "StorageLosses"),
        (ROD_ELECTRIC, "EnergyUsedInHeatingRod"),
    ] {
        let Some(node) = path_node(thermal, &[tag]) else { continue; };
        let energy = |n, path: &[&str]| number::<f64>(n, path)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v / WS_PER_KWH);
        let annual_kwh = energy(node, &["YearlyValue"]);
        let monthly_kwh = children(node, "MonthlyValues").map(|n| energy(n, &["value"]))
            .collect::<Option<Vec<_>>>().filter(|months| months.len() == 12);
        if annual_kwh.is_some() || monthly_kwh.is_some() {
            saved.insert(channel.into(), Channel { annual_kwh, monthly_kwh, ..Default::default() });
        }
    }
}

pub(super) fn read_hourly_reference(hours: Option<Vec<f64>>, pv: Option<&[f64]>, base_load: Option<&[f64]>,
    saved: &mut BTreeMap<String, Channel>) -> Option<ArchivedSwitching>
{
    let hours = hours.filter(|hours| hours.len() == HOURS && hours.iter().all(|v| v.is_finite() && *v >= 0.0)
        && hours.iter().any(|v| *v > 0.0))?;
    let switching = pv.zip(base_load).and_then(|(pv, base_load)| archived_switching(&hours, pv, base_load));
    let channel = saved.entry(PUMP_ELECTRIC.into()).or_default();
    channel.hourly_kwh = Some(hours);
    channel.hourly_source = Some("OecForeCast.json / HeatPumpElectricalConsumption".into());
    switching
}

fn archived_switching(pump: &[f64], pv: &[f64], base_load: &[f64]) -> Option<ArchivedSwitching> {
    if pv.len() != HOURS || base_load.len() != HOURS
        || pv.iter().any(|v| !v.is_finite()) || base_load.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    let mut observed = ArchivedSwitching {
        hourly_rows: HOURS, reporting_threshold_w: SWITCHING_THRESHOLD_W, pump_on_hours: 0,
        pv_surplus_hours: 0, pump_on_with_surplus_hours: 0, pump_on_without_surplus_hours: 0,
        pump_kwh_covered_by_concurrent_pv: 0.0, pump_kwh_beyond_concurrent_pv: 0.0,
        source: "OecForeCast.json / HeatPumpElectricalConsumption, PvErzeugungHourly, PowerConsumptionHourly".into(),
    };
    for ((&pump, &pv), &base_load) in pump.iter().zip(pv).zip(base_load) {
        let surplus = (pv - base_load).max(0.0);
        let pump_on = pump > SWITCHING_THRESHOLD_KWH;
        let has_surplus = surplus > SWITCHING_THRESHOLD_KWH;
        observed.pump_on_hours += usize::from(pump_on);
        observed.pv_surplus_hours += usize::from(has_surplus);
        observed.pump_on_with_surplus_hours += usize::from(pump_on && has_surplus);
        observed.pump_on_without_surplus_hours += usize::from(pump_on && !has_surplus);
        observed.pump_kwh_covered_by_concurrent_pv += pump.min(surplus);
        observed.pump_kwh_beyond_concurrent_pv += (pump - surplus).max(0.0);
    }
    Some(observed)
}

pub(super) fn add_pv_surplus_variant(project: &mut Project, saved: &BTreeMap<String, Channel>, warnings: &mut Vec<String>) {
    let Some((electric, solar)) = saved.get(PUMP_ELECTRIC).and_then(Channel::annual)
        .zip(saved.get(PUMP_SOLAR).and_then(Channel::annual))
        .filter(|(electric, solar)| *electric > 0.0 && *solar <= *electric && *solar / *electric >= 0.99)
    else { return; };
    let mut variant = project.clone();
    let Some(controls) = variant.heat_store.as_mut().and_then(|tank| tank.controls.as_mut()) else { return; };
    let ThermalEmitter::HeatPump { pv_only, .. } = &mut controls.emitter else { return; };
    if *pv_only { return; }
    *pv_only = true;
    project.variants.push(Variant {
        id: "var-1".into(),
        name: "PV-surplus heat pump (sensitivity)".into(),
        created_unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs() as i64),
        design: variant.design_snapshot(),
    });
    warnings.push(format!("The project records Standard heat pump dispatch. Saved results attribute {:.2}% of pump electricity to direct PV. The PV-surplus heat pump variant tests that dispatch assumption and can be edited.", 100.0 * solar / electric));
}

pub(super) fn compare_reference(project: &Project, run: &SimResult, saved: &BTreeMap<String, Channel>, comparison: &mut Map<String, Value>) {
    let Some(controls) = project.heat_store.as_ref().and_then(|tank| tank.controls.as_ref()) else { return; };
    let (Some(heating), Some(tank)) = (&run.heating, &run.heat_store) else { return; };
    let mut compare = |name: &str, modeled: f64, basis: &str, with_hours: bool| {
        let Some(channel) = saved.get(name) else { return; };
        let pvsol = channel.annual();
        let delta = pvsol.map(|value| modeled - value);
        let hours = with_hours.then(|| aligned_electricity(run)).flatten();
        let monthly = hours.as_ref().zip(channel.monthly()).map(|(hours, reference)| {
            let mut months = [0.0; 12];
            for (hour, value) in hours.iter().enumerate() {
                months[local_time(hour as u32).month as usize - 1] += value;
            }
            months.iter().zip(reference).enumerate().map(|(month, (actual, saved))| {
                json!({"month":month + 1, "solarcraft_kwh":actual, "pvsol_kwh":saved, "delta_kwh":actual - saved})
            }).collect::<Vec<_>>()
        });
        let aligned = hours.as_ref().zip(channel.hourly_kwh.as_ref().filter(|hours| hours.len() == HOURS));
        let hourly = aligned.map(|(hours, reference)| {
            let differences = hours.iter().zip(reference).map(|(actual, saved)| actual - saved);
            let mae = differences.clone().map(f64::abs).sum::<f64>() / HOURS as f64;
            let rmse = (differences.map(|delta| delta * delta).sum::<f64>() / HOURS as f64).sqrt();
            json!({"rows":HOURS, "mae_kwh":mae, "rmse_kwh":rmse, "source":channel.hourly_source,
                "alignment":"ordered local standard-time hour from January 1"})
        });
        let mut row = json!({"solarcraft_kwh":modeled, "pvsol_kwh":pvsol, "solarcraft_basis":basis,
            "delta_kwh":delta, "delta_pct":delta.zip(pvsol.filter(|v| *v > 1e-9)).map(|(d, v)| d / v * 100.0),
            "monthly":monthly, "hourly":hourly});
        if name == PUMP_ELECTRIC {
            if let Some((hours, reference)) = aligned {
                row["switching"] = compare_switching(hours, reference);
            }
        }
        comparison.insert(name.into(), row);
    };
    match controls.emitter {
        ThermalEmitter::HeatPump { pv_only, .. } => {
            let basis = if pv_only { "heat pump electricity, limited to direct PV surplus" } else { "heat pump electricity, Standard dispatch" };
            compare(PUMP_ELECTRIC, heating.pump_electric_kwh, basis, true);
            compare(PUMP_HEAT, heating.pump_heat_kwh, "heat delivered by the heat pump", false);
            if let Some(solar) = tank.direct_solar_electric_kwh {
                compare(PUMP_SOLAR, solar, "pump electricity covered by concurrent PV after base load", false);
            }
        }
        ThermalEmitter::Resistance { .. } => compare(ROD_ELECTRIC, heating.backup_kwh, "resistance heater electricity", true),
    }
    if let Some(boiler) = tank.boiler_heat_kwh {
        compare(BOILER_HEAT, boiler, "heat delivered by the fuel boiler", false);
    }
    compare(TANK_LOSSES, tank.standing_loss_kwh, "thermal tank standing loss", false);
}

fn compare_switching(modeled: &[f64], saved: &[f64]) -> Value {
    let mut matched_on = 0;
    let mut missed_on = 0;
    let mut extra_on = 0;
    for (&modeled, &saved) in modeled.iter().zip(saved) {
        match (modeled > SWITCHING_THRESHOLD_KWH, saved > SWITCHING_THRESHOLD_KWH) {
            (true, true) => matched_on += 1,
            (false, true) => missed_on += 1,
            (true, false) => extra_on += 1,
            (false, false) => {}
        }
    }
    json!({"reporting_threshold_w":SWITCHING_THRESHOLD_W, "hourly_rows":HOURS,
        "pvsol_pump_on_hours":matched_on + missed_on, "solarcraft_pump_on_hours":matched_on + extra_on,
        "matched_on_hours":matched_on, "missed_on_hours":missed_on, "extra_on_hours":extra_on})
}

fn aligned_electricity(run: &SimResult) -> Option<Vec<f64>> {
    if run.local_hour.len() != HOURS || run.hourly.heating_wh.len() != HOURS { return None; }
    let mut ordered = vec![f64::NAN; HOURS];
    for (&hour, &wh) in run.local_hour.iter().zip(&run.hourly.heating_wh) {
        let slot = ordered.get_mut(hour as usize)?;
        if !slot.is_nan() || !wh.is_finite() { return None; }
        *slot = f64::from(wh) / 1000.0;
    }
    Some(ordered)
}

fn field(node: Node<'_, '_>, name: &str) -> Result<f64> {
    number::<f64>(node, &[name]).filter(|n| n.is_finite())
        .ok_or_else(|| e("PVSOL_IMPORT", format!("A thermal setting is missing or invalid: {name}.")))
}

fn weights<const N: usize>(node: Node<'_, '_>, name: &str) -> Option<[f64; N]> {
    value(node, &[name])?.split_ascii_whitespace().map(str::parse).collect::<std::result::Result<Vec<_>, _>>().ok()?.try_into().ok()
}

fn hot_water_weights(node: Node<'_, '_>) -> Option<HotWaterWeights> {
    let days = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let mut hour = [[0.0; 24]; 7];
    for (row, day) in hour.iter_mut().zip(days) { *row = weights(node, day)?; }
    Some(HotWaterWeights { month: weights(node, "Year")?, weekday: weights(node, "Week")?, hour })
}

pub(super) fn apply(project: &mut Project, root: Node<'_, '_>, weather: &Weather, warnings: &mut Vec<String>) -> Result<()> {
    let node = path_node(root, &["ThermalSystem"]).ok_or_else(|| e("PVSOL_IMPORT", "The thermal system record is missing."))?;
    let (emitter, thermal_kw, cop_at_0c, cop_at_10c) = match value(node, &["SystemType"]) {
        Some("HeizstabWarmwasserHeizung") => {
            let steps_kw: Vec<f64> = path_node(node, &["StorageTankHeatingRodPowerSteps"]).into_iter()
                .flat_map(|n| children(n, "Power"))
                .filter_map(|n| number::<f64>(n, &[]).map(|w| w / 1000.0)).collect();
            let thermal_kw = steps_kw.iter().copied().fold(0.0, f64::max);
            (ThermalEmitter::Resistance { modulating: flag(node, &["StorageTankHasHeatingRodSteplessPowerOutput"]), steps_kw }, thermal_kw, 1.0, 1.0)
        }
        Some("WaermepumpeWarmwasserHeizung") => {
            if number::<u32>(node, &["HeatPumpType"]) != Some(1) {
                return Err(e("PVSOL_IMPORT", "Only the archived air-source heat pump is supported."));
            }
            let cop_a_minus7 = field(node, "HeatPumpCOP_Am7_W35")?;
            let cop_a2 = field(node, "HeatPumpNominalCOP")?;
            let cop_a7 = field(node, "HeatPumpCOP_A7_W35")?;
            let cop_at_10c = field(node, "HeatPumpCOP_A10_W35")?;
            let pv_only = match number::<u32>(node, &["HeatPumpOperationMode"]) {
                Some(0) => false,
                Some(1) => {
                    warnings.push("The project records legacy PV-optimized heat pump dispatch. SolarCraft approximates it by limiting the pump to PV surplus.".into());
                    true
                }
                _ => return Err(e("PVSOL_IMPORT", "The heat pump operation mode is unsupported.")),
            };
            let periods = if flag(node, &["HeatPumpHasSwitchOff"]) {
                number::<u32>(node, &["HeatPumpNumberOfSwitchOffPhases"]).unwrap_or(0)
            } else { 0 };
            if periods > 3 { return Err(e("PVSOL_IMPORT", "The heat pump has more than three blocked periods.")); }
            let mut blocked_seconds = Vec::new();
            for slot in 1..=periods {
                let bound = |name| number::<u32>(node, &[&format!("HeatPumpSwitchOff{slot}{name}")]);
                let (start, end) = bound("Start").zip(bound("End"))
                    .ok_or_else(|| e("PVSOL_IMPORT", "A heat pump blocked interval is missing."))?;
                blocked_seconds.push([start, end]);
            }
            (ThermalEmitter::HeatPump { pv_only, cop_a_minus7, cop_a2, cop_a7, blocked_seconds },
                field(node, "HeatPumpNominalPower")? / 1000.0, cop_a2 + (cop_a_minus7 - cop_a2) * 2.0 / 9.0, cop_at_10c)
        }
        _ => return Err(e("PVSOL_IMPORT", "The archived thermal system type is unsupported.")),
    };
    let ambient_c = field(node, "StorageTankMeanTemperatureInHeatedSideRooms")?;
    let solar_target_c = field(node, "StorageTankMaxTemperatureForHeatingRod")?;
    let boiler_target_c = field(node, "StorageTankMaxTemperatureForBoiler")?;
    let diameter = field(node, "StorageTankDiameter")?;
    let height = field(node, "StorageTankHeight")?;
    let insulation = field(node, "StorageTankIsolationThickness")?;
    if diameter <= 0.0 || height <= 0.0 || insulation < 0.0 || solar_target_c <= ambient_c {
        return Err(e("PVSOL_IMPORT", "The archived thermal tank dimensions or temperatures are invalid."));
    }
    let base_m2 = PI * diameter.powi(2) / 4.0;
    let heat_capacity_kwh_per_k = 0.9 * base_m2 * height * 994.0 * 4187.0 / 3_600_000.0;
    let surface_m2 = 2.0 * base_m2 + PI * diameter * height;
    let resistance = insulation / (0.037 + 0.0002 * (boiler_target_c + ambient_c) / 2.0) + 6.25e-5;
    let hourly_loss_rate = surface_m2 / resistance / 1000.0 / heat_capacity_kwh_per_k;
    let boiler_limit_kw = flag(node, &["BoilerHasLimitedHeatingPower"])
        .then(|| field(node, "BoilerMaximumHeatingPower").map(|w| w / 1000.0)).transpose()?;
    let sanitation = flag(node, &["StorageTankIsLegionellaControlActive"]).then(|| {
        let seconds = |name| number::<u32>(node, &[name]).ok_or_else(|| e("PVSOL_IMPORT", "A tank sanitation interval is missing."));
        Ok(TankSanitation {
            target_c: field(node, "StorageTankLegionellaTargetTemperature")?,
            start_second: seconds("StorageTankLegionellaControlStartTime")?,
            interval_seconds: seconds("StorageTankLegionellaControlInterval")?,
        })
    }).transpose()?;
    let setpoint_c = field(node, "HeatingProfileTRoomNominal")? - 273.15;
    let nominal_kw = field(node, "HeatingProfileHeatingLoadAtNominalTemperature")? / 1000.0;
    let nominal_outside_c = field(node, "HeatingProfileTAmbNominal")? - 273.15;
    let nominal_difference_c = setpoint_c - nominal_outside_c;
    let schedule = flag(node, &["StorageTankHasHeatingConnected"]).then(|| {
        let range = |start, end| number::<u32>(node, &[start]).zip(number::<u32>(node, &[end]))
            .map(|(start, end)| [start, end]).ok_or_else(|| e("PVSOL_IMPORT", "A thermal schedule interval is missing."));
        let building_gains = number::<f64>(node, &["HeatingProfileAreaEffective"])
            .filter(|area| area.is_finite() && *area > 0.0 && nominal_kw > 0.0 && nominal_difference_c > 0.0)
            .map(|effective_floor_area_m2| BuildingHeatGains {
                effective_floor_area_m2, design_temperature_difference_c: nominal_difference_c,
            });
        Ok(SpaceHeatSchedule {
            threshold_c: field(node, "HeatingProfileTHeatingThreshold")? - 273.15,
            inactive_days: range("HeatingProfileHeatingInactiveStart", "HeatingProfileHeatingInactiveEnd")?,
            night_seconds: flag(node, &["HeatingProfileIsNightTemperatureLowered"])
                .then(|| range("HeatingProfileLoweredNightStart", "HeatingProfileLoweredNightEnd")).transpose()?,
            night_setback_c: field(node, "HeatingProfileDeltaTRoomNight")?,
            building_gains,
        })
    }).transpose()?;
    let annual_kwh = field(node, "HeatingProfileEHeatingTargetYear")? / 3_600_000.0;
    let has_building_gains = schedule.as_ref().is_some_and(|shape| shape.building_gains.is_some());
    let nominal_loss = (nominal_kw / nominal_difference_c.max(1.0)).max(0.001);
    let heat_loss_kw_per_k = if has_building_gains { nominal_kw / nominal_difference_c } else {
        let degree_hours: f64 = schedule.as_ref().map_or(0.0, |shape|
            shape.degree_hours(setpoint_c, weather.hours.iter().map(|h| h.temp_air)).iter().sum());
        if annual_kwh > 0.0 && degree_hours > 0.0 { annual_kwh / degree_hours } else { nominal_loss }
    };
    let daily_hot_water_kwh = if flag(node, &["StorageTankHasDrinkingWaterConnected"]) {
        field(node, "WarmWaterProfileYearlyEnergyDemand")? / (3_600_000.0 * 365.0)
    } else { 0.0 };
    let hot_water = if daily_hot_water_kwh > 0.0 {
        let weights = path_node(node, &["WarmwaterProfile"]).and_then(hot_water_weights)
            .ok_or_else(|| e("PVSOL_IMPORT", "The saved hot water profile is incomplete."))?;
        Some(weights)
    } else { None };
    let heating = Heating { setpoint_c, heat_loss_kw_per_k, thermal_kw, cutoff_c: -20.0, cop_at_0c, cop_at_10c, resistance_backup_kw: 0.0 };
    let heat_store = HeatStore {
        capacity_kwh: heat_capacity_kwh_per_k * (solar_target_c - ambient_c),
        initial_kwh: heat_capacity_kwh_per_k * (boiler_target_c - ambient_c),
        charge_kw: thermal_kw, discharge_kw: nominal_kw.max(thermal_kw),
        standing_loss_pct_per_day: -100.0 * (-24.0 * hourly_loss_rate).exp_m1(),
        daily_hot_water_kwh, hot_water_hour: 19, layers: None,
        controls: Some(ThermalControls { ambient_c, solar_target_c, boiler_target_c, boiler_limit_kw,
            emitter, space: schedule, hot_water, sanitation, insulation_thickness_m: Some(insulation) }),
    };
    if !heating.valid() || !heat_store.valid() {
        return Err(e("PVSOL_IMPORT", "The archived heating or thermal tank settings are outside supported limits."));
    }
    project.workflow.system = SystemType::GridThermal;
    project.heating = Some(heating);
    project.heat_store = Some(heat_store);
    warnings.push("Imported the solar heater, mixed tank, fuel boiler and saved hourly water profile. Tank standing losses account for the temperature of the water and insulation.".into());
    if has_building_gains {
        warnings.push("Modeled space-heating demand uses the saved design heat load and effective floor area with solar gains on four vertical façades and daytime internal gains.".into());
    } else if project.heat_store.as_ref().and_then(|tank| tank.controls.as_ref()).is_some_and(|controls| controls.space.is_some()) {
        warnings.push("The space-heating schedule uses the saved annual demand and outdoor temperatures to estimate hourly demand.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_hourly_switching_from_concurrent_pv_attribution() {
        let mut pump = vec![0.0; HOURS];
        let mut pv = vec![0.0; HOURS];
        let mut base_load = vec![0.0; HOURS];
        pump[..4].copy_from_slice(&[0.2, 0.35, 0.01, 0.001]);
        pv[..4].copy_from_slice(&[0.7, 0.5, -0.2, 0.201]);
        base_load[..4].copy_from_slice(&[0.4, 0.3, 0.1, 0.001]);
        let mut saved = BTreeMap::new();
        let observed = read_hourly_reference(Some(pump.clone()), Some(&pv), Some(&base_load), &mut saved).unwrap();
        assert_eq!(observed.pump_on_hours, 3);
        assert_eq!(observed.pv_surplus_hours, 3);
        assert_eq!(observed.pump_on_with_surplus_hours, 2);
        assert_eq!(observed.pump_on_without_surplus_hours, 1);
        assert!((observed.pump_kwh_covered_by_concurrent_pv - 0.401).abs() < 1e-10);
        assert!((observed.pump_kwh_beyond_concurrent_pv - 0.160).abs() < 1e-10);

        let mut modeled = pump.clone();
        modeled[2] = 0.0;
        modeled[4] = 0.1;
        let switching = compare_switching(&modeled, &pump);
        assert_eq!(switching["matched_on_hours"], 2);
        assert_eq!(switching["missed_on_hours"], 1);
        assert_eq!(switching["extra_on_hours"], 1);

        let missing = read_hourly_reference(Some(pump), Some(&pv[..HOURS - 1]), Some(&base_load), &mut saved);
        assert!(missing.is_none());
        assert_eq!(saved[PUMP_ELECTRIC].hourly_kwh.as_ref().map(Vec::len), Some(HOURS));
    }

    #[test]
    fn saved_solar_dispatch_keeps_standard_as_active_design() {
        let archive = roxmltree::Document::parse(r#"
            <Results><ThermalResult>
                <HeatpumpElectricalEnergyConsumption>
                    <YearlyValue>3600000000</YearlyValue>
                </HeatpumpElectricalEnergyConsumption>
                <HeatpumpElectricalSolarEnergyConsumption>
                    <YearlyValue>3592800000</YearlyValue>
                </HeatpumpElectricalSolarEnergyConsumption>
            </ThermalResult></Results>
        "#).unwrap();
        let mut saved = BTreeMap::new();
        read_reference(archive.root_element(), &mut saved);
        assert!(read_hourly_reference(Some(vec![0.0; HOURS - 1]), None, None, &mut saved).is_none());
        assert_eq!(saved[PUMP_ELECTRIC].annual(), Some(1000.0));
        assert_eq!(saved[PUMP_SOLAR].annual(), Some(998.0));
        assert!(saved[PUMP_ELECTRIC].hourly_kwh.is_none());

        let mut project = Project::new("thermal-test", "Thermal test", 52.0, 13.0);
        project.heat_store = Some(HeatStore {
            capacity_kwh: 10.0, initial_kwh: 8.0, charge_kw: 6.0, discharge_kw: 10.0,
            standing_loss_pct_per_day: 0.0, daily_hot_water_kwh: 3.0, hot_water_hour: 1, layers: None,
            controls: Some(ThermalControls {
                ambient_c: 20.0, solar_target_c: 70.0, boiler_target_c: 60.0, boiler_limit_kw: None,
                emitter: ThermalEmitter::HeatPump { pv_only: false, cop_a_minus7: 2.5, cop_a2: 3.0,
                    cop_a7: 3.5, blocked_seconds: vec![] },
                space: None, hot_water: None, sanitation: None, insulation_thickness_m: None,
            }),
        });
        let active = project.design_snapshot();
        let mut warnings = Vec::new();
        add_pv_surplus_variant(&mut project, &saved, &mut warnings);
        assert_eq!(project.design_snapshot(), active);
        assert_eq!(project.variants.len(), 1);
        assert_eq!(warnings.len(), 1);
        let variant = project.variants[0].design.clone();
        project.apply_snapshot(&variant).unwrap();
        assert!(matches!(
            project.heat_store.unwrap().controls.unwrap().emitter,
            ThermalEmitter::HeatPump { pv_only: true, .. },
        ));
    }
}
