// SPDX-License-Identifier: GPL-3.0-or-later
use super::{calendar, calendar_order, TankRun};
use solarcraft_model::demand::{HeatStore, Heating, HotWaterWeights, ThermalControls, ThermalEmitter};

fn water_profile(weights: &HotWaterWeights, annual_kwh: f64) -> Vec<f64> {
    let mut month_weight = [0.0; 12];
    let month_sum: f64 = weights.month.iter().sum();
    let hour_sum = weights.hour.each_ref().map(|hours| hours.iter().sum::<f64>());
    // The archived water generator uses the weekday of a fixed 2021 calendar.
    let calendar = |h| {
        let time = calendar::local_time(h);
        (time.month as usize - 1, (time.day_of_year as usize + 4) % 7, time.hour as usize)
    };
    for day in 0..365 {
        let (month, weekday, _) = calendar(day * 24);
        month_weight[month] += weights.weekday[weekday];
    }
    (0..8760).map(|h| {
        let (month, weekday, hour) = calendar(h);
        if weights.weekday[weekday] == 0.0 { return 0.0; }
        annual_kwh * weights.month[month] / month_sum * weights.weekday[weekday] / month_weight[month]
            * weights.hour[weekday][hour] / hour_sum[weekday]
    }).collect()
}

fn heat_pump_cop(outside_c: f64, a_minus7: f64, a0: f64, a2: f64, a7: f64, a10: f64) -> f64 {
    let ambient_c = [-7.0, 0.0, 2.0, 7.0, 10.0, 15.0, 20.0, 25.0, 35.0];
    let cop = [a_minus7, a0, a2, a7, a10, 1.12 * a10, 1.2 * a10, 1.25 * a10, 1.3 * a10];
    let upper = ambient_c.partition_point(|&t| t < outside_c);
    if upper == 0 {
        return if outside_c == -7.0 { a_minus7 } else { a2 * 33.0 / (35.0 - outside_c) };
    }
    let fraction = (outside_c - ambient_c[upper - 1]) / (ambient_c[upper] - ambient_c[upper - 1]);
    cop[upper - 1] + fraction * (cop[upper] - cop[upper - 1])
}

fn electric_heat(heating: &Heating, emitter: &ThermalEmitter, hour: usize, outside: f64,
    room: f64, spare_kw: f64, charge_limit: f64) -> (f64, f64)
{
    let limit = charge_limit.min(heating.thermal_kw).min(room);
    match emitter {
        ThermalEmitter::Resistance { modulating, steps_kw } => {
            let available_kw = spare_kw.min(charge_limit).min(heating.thermal_kw);
            let output = if *modulating { available_kw } else {
                steps_kw.iter().copied().filter(|&kw| kw <= available_kw).fold(0.0, f64::max)
            }.min(room);
            (output, output)
        }
        ThermalEmitter::HeatPump { pv_only, cop_a_minus7, cop_a2, cop_a7, blocked_seconds } => {
            let second = hour as u32 % 24 * 3600;
            if outside < heating.cutoff_c || outside > 35.0
                || blocked_seconds.iter().any(|[from, to]| (*from..*to).contains(&second)) {
                return (0.0, 0.0);
            }
            let cop = heat_pump_cop(outside, *cop_a_minus7, heating.cop_at_0c, *cop_a2, *cop_a7, heating.cop_at_10c);
            let output = if *pv_only { limit.min(spare_kw * cop) } else { limit };
            (output, output / cop)
        }
    }
}

pub(super) fn mixed_tank(heat: &Heating, tank: &HeatStore, controls: &ThermalControls, temp: &[f64],
    local_hour: &[u32], load: &[f64], pv: &[f32]) -> TankRun
{
    let n = local_hour.len();
    let order = calendar_order(local_hour);
    let degree_hours = controls.space.as_ref().map_or_else(|| vec![0.0; n],
        |s| s.degree_hours(heat.setpoint_c, order.iter().map(|&i| temp[i])));
    let hot_water = controls.hot_water.as_ref().map(|w| water_profile(w, tank.daily_hot_water_kwh * 365.0));
    let mut run = TankRun { demand: Default::default(), result: Default::default(), electrical: vec![0.0; n],
        stored: vec![0.0; n], top_c: vec![], bottom_c: vec![] };
    let loss_fraction = 1.0 - (1.0 - tank.standing_loss_pct_per_day / 100.0).powf(1.0 / 24.0);
    let kwh_per_k = tank.capacity_kwh / (controls.solar_target_c - controls.ambient_c);
    let heat_at = |c: f64| kwh_per_k * (c - controls.ambient_c);
    let resistance = matches!(controls.emitter, ThermalEmitter::Resistance { .. });
    let pv_only = !matches!(controls.emitter, ThermalEmitter::HeatPump { pv_only: false, .. });
    let mut stored = tank.initial_kwh;
    let mut boiler_total = 0.0;
    let mut solar_electric = 0.0;
    for (h, i) in order.into_iter().enumerate() {
        let loss = stored * loss_fraction;
        let water = hot_water.as_ref().map_or_else(
            || if h as u32 % 24 == tank.hot_water_hour { tank.daily_hot_water_kwh } else { 0.0 },
            |profile| profile[h]);
        let demand = degree_hours[h] * heat.heat_loss_kw_per_k + water;
        let request = demand.min(tank.discharge_kw);
        stored -= loss + request;
        let now = h as u32 * 3600;
        let hygiene_temperature = controls.sanitation.filter(|s| {
            let next = if now <= s.start_second { s.start_second } else {
                s.start_second + (now - s.start_second).div_ceil(s.interval_seconds) * s.interval_seconds
            };
            next < now + 3600
        }).map(|s| s.target_c);
        let boiler_target = heat_at(hygiene_temperature.unwrap_or(controls.boiler_target_c));
        let electric_target = if pv_only { heat_at(hygiene_temperature.unwrap_or(controls.solar_target_c)) } else { boiler_target };
        let spare_kw = ((f64::from(pv[i]) - load[i]) / 1000.0).max(0.0);
        let (input_heat, electric) = electric_heat(heat, &controls.emitter, h, temp[i],
            (electric_target - stored).max(0.0), spare_kw, tank.charge_kw);
        stored += input_heat;
        let boiler = (boiler_target - stored).max(0.0).min(controls.boiler_limit_kw.unwrap_or(f64::INFINITY));
        stored += boiler;
        let delivered = request + stored.min(0.0);
        stored = stored.max(0.0);

        run.electrical[i] = (electric * 1000.0) as f32;
        run.stored[i] = stored as f32;
        run.demand.demand_kwh += demand;
        run.demand.unmet_heat_kwh += demand - delivered;
        if resistance { run.demand.backup_kwh += electric; }
        else {
            run.demand.pump_heat_kwh += input_heat;
            run.demand.pump_electric_kwh += electric;
        }
        run.result.charge_heat_kwh += input_heat + boiler;
        run.result.charge_electric_kwh += electric;
        solar_electric += electric.min(spare_kw);
        run.result.discharge_heat_kwh += delivered;
        run.result.standing_loss_kwh += loss;
        run.result.hot_water_kwh += water;
        run.result.tank_to_hot_water_kwh += water.min(delivered);
        boiler_total += boiler;
    }
    run.result.end_kwh = stored;
    run.result.boiler_heat_kwh = Some(boiler_total);
    run.result.direct_solar_electric_kwh = Some(solar_electric);
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_tank_keeps_boiler_heat_out_of_electrical_load() {
        let heat = Heating { setpoint_c: 20.0, heat_loss_kw_per_k: 0.2, thermal_kw: 6.0,
            cutoff_c: -20.0, cop_at_0c: 3.0, cop_at_10c: 4.28, resistance_backup_kw: 0.0 };
        let controls = ThermalControls { ambient_c: 20.0, solar_target_c: 70.0, boiler_target_c: 60.0,
            boiler_limit_kw: None, emitter: ThermalEmitter::HeatPump { pv_only: true, cop_a_minus7: 2.31,
                cop_a2: 3.1, cop_a7: 3.98, blocked_seconds: vec![] }, space: None, hot_water: None, sanitation: None };
        let tank = HeatStore { capacity_kwh: 10.0, initial_kwh: 8.0, charge_kw: 6.0, discharge_kw: 10.0,
            standing_loss_pct_per_day: 0.0, daily_hot_water_kwh: 3.0, hot_water_hour: 1, layers: None, controls: None };
        let direct = mixed_tank(&heat, &tank, &controls, &[7.0; 3], &[0, 1, 2], &[0.0; 3], &[0.0; 3]);
        assert_eq!(direct.electrical, [0.0; 3]);
        assert_eq!(direct.result.boiler_heat_kwh, Some(3.0));
        assert_eq!(direct.result.direct_solar_electric_kwh, Some(0.0));
        let solar = mixed_tank(&heat, &tank, &controls, &[7.0; 3], &[0, 1, 2], &[0.0; 3], &[1000.0, 0.0, 0.0]);
        assert!((solar.demand.pump_heat_kwh - 2.0).abs() < 1e-9);
        assert_eq!(solar.result.direct_solar_electric_kwh, Some(solar.demand.pump_electric_kwh));
        assert!((solar.result.boiler_heat_kwh.unwrap() - 1.0).abs() < 1e-9);
        assert!((solar.result.boiler_heat_kwh.unwrap() + solar.demand.pump_heat_kwh - solar.result.discharge_heat_kwh).abs() < 1e-9);
        let mut standard = controls;
        if let ThermalEmitter::HeatPump { pv_only, .. } = &mut standard.emitter { *pv_only = false; }
        let grid = mixed_tank(&heat, &tank, &standard, &[7.0; 3], &[0, 1, 2], &[0.0; 3], &[0.0; 3]);
        assert!((grid.demand.pump_electric_kwh - 3.0 / 3.98).abs() < 1e-9);
        assert_eq!(grid.result.boiler_heat_kwh, Some(0.0));
    }
}
