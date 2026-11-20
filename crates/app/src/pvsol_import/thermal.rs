// SPDX-License-Identifier: GPL-3.0-or-later
//! Import the mixed storage tank, solar electricity heater and fuel boiler.
use super::{children, flag, number, path_node, value};
use crate::{e, Result};
use roxmltree::Node;
use solarcraft_model::{
    demand::{HeatStore, Heating, HotWaterWeights, SpaceHeatSchedule, TankSanitation, ThermalControls, ThermalEmitter},
    workflow::SystemType,
    Project,
};
use solarcraft_weather::Weather;
use std::f64::consts::PI;

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
                Some(1) => true,
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
    let schedule = flag(node, &["StorageTankHasHeatingConnected"]).then(|| {
        let range = |start, end| number::<u32>(node, &[start]).zip(number::<u32>(node, &[end]))
            .map(|(start, end)| [start, end]).ok_or_else(|| e("PVSOL_IMPORT", "A thermal schedule interval is missing."));
        Ok(SpaceHeatSchedule {
            threshold_c: field(node, "HeatingProfileTHeatingThreshold")? - 273.15,
            inactive_days: range("HeatingProfileHeatingInactiveStart", "HeatingProfileHeatingInactiveEnd")?,
            night_seconds: flag(node, &["HeatingProfileIsNightTemperatureLowered"])
                .then(|| range("HeatingProfileLoweredNightStart", "HeatingProfileLoweredNightEnd")).transpose()?,
            night_setback_c: field(node, "HeatingProfileDeltaTRoomNight")?,
        })
    }).transpose()?;
    let nominal_kw = field(node, "HeatingProfileHeatingLoadAtNominalTemperature")? / 1000.0;
    let nominal_outside_c = field(node, "HeatingProfileTAmbNominal")? - 273.15;
    let annual_kwh = field(node, "HeatingProfileEHeatingTargetYear")? / 3_600_000.0;
    let degree_hours = schedule.as_ref().map_or(0.0, |shape|
        shape.degree_hours(setpoint_c, weather.hours.iter().map(|h| h.temp_air)).iter().sum());
    let heat_loss_kw_per_k = if annual_kwh > 0.0 && degree_hours > 0.0 {
        annual_kwh / degree_hours
    } else { (nominal_kw / (setpoint_c - nominal_outside_c).max(1.0)).max(0.001) };
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
            emitter, space: schedule, hot_water, sanitation }),
    };
    if !heating.valid() || !heat_store.valid() {
        return Err(e("PVSOL_IMPORT", "The archived heating or thermal tank settings are outside supported limits."));
    }
    project.workflow.system = SystemType::GridThermal;
    project.heating = Some(heating);
    project.heat_store = Some(heat_store);
    warnings.push("Imported the solar heater, mixed tank, fuel boiler and saved hourly water profile. The space-heating schedule uses the saved annual demand and outdoor temperatures to estimate hourly demand.".into());
    Ok(())
}
