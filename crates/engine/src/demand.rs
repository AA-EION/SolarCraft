// SPDX-License-Identifier: GPL-3.0-or-later
use serde::{Deserialize, Serialize};
use solarcraft_model::demand::{ChargeStrategy, Ev, Heating};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvResult {
    pub requested_kwh: f64,
    pub charged_kwh: f64,
    pub electrical_kwh: f64,
    pub unscheduled_kwh: f64,
    pub forecast_surplus_kwh: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HeatingResult {
    pub demand_kwh: f64,
    pub pump_heat_kwh: f64,
    pub pump_electric_kwh: f64,
    pub backup_kwh: f64,
    pub unmet_heat_kwh: f64,
}

pub(crate) fn cop(heating: &Heating, outside: f64) -> f64 {
    (heating.cop_at_0c + outside * (heating.cop_at_10c - heating.cop_at_0c) / 10.0).clamp(1.0, 12.0)
}

pub fn heating(h: &Heating, temperatures: impl Iterator<Item = f64>) -> (Vec<f32>, HeatingResult) {
    let mut result = HeatingResult::default();
    let hourly = temperatures.map(|outside| {
        let demand = h.heat_loss_kw_per_k * (h.setpoint_c - outside).max(0.0);
        let pump = if outside >= h.cutoff_c { demand.min(h.thermal_kw) } else { 0.0 };
        let cop = cop(h, outside);
        let backup = (demand - pump).min(h.resistance_backup_kw);
        result.demand_kwh += demand;
        result.pump_heat_kwh += pump;
        result.pump_electric_kwh += pump / cop;
        result.backup_kwh += backup;
        result.unmet_heat_kwh += demand - pump - backup;
        ((pump / cop + backup) * 1000.0) as f32
    }).collect();
    (hourly, result)
}

pub fn charging(ev: &Ev, local_hour: &[u32], surplus_w: &[f64]) -> (Vec<f32>, EvResult) {
    let n = local_hour.len();
    let mut by_local = vec![0usize; n];
    for (hour, &local) in local_hour.iter().enumerate() { by_local[local as usize] = hour; }
    let mut result = EvResult::default();
    let mut hourly = vec![0.0; n];
    let window = (ev.departure_hour + 24 - ev.arrival_hour) % 24;
    let window = if window == 0 { 24 } else { window };
    let limit_wh = ev.charger_kw * 1000.0;
    for day in 0..n / 24 {
        let hours = || (0..window as usize).map(|h| by_local[(day * 24 + ev.arrival_hour as usize + h) % n]);
        let request_wh = ev.daily_kwh * 1000.0 / ev.efficiency;
        let mut left = request_wh;
        if ev.strategy == ChargeStrategy::SolarPreferred {
            for hour in hours() {
                let used = surplus_w[hour].max(0.0).min(limit_wh).min(left);
                left -= used;
                hourly[hour] += used;
            }
        }
        for hour in hours() {
            let used = (limit_wh - hourly[hour]).min(left);
            left -= used;
            hourly[hour] += used;
            result.forecast_surplus_kwh += hourly[hour].min(surplus_w[hour].max(0.0)) / 1000.0;
        }
        result.requested_kwh += ev.daily_kwh;
        result.electrical_kwh += (request_wh - left) / 1000.0;
        result.unscheduled_kwh += left / 1000.0 * ev.efficiency;
    }
    result.charged_kwh = result.electrical_kwh * ev.efficiency;
    (hourly.into_iter().map(|w| w as f32).collect(), result)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charging_respects_the_departure_window_and_reports_missing_quota() {
        let ev = Ev { daily_kwh: 8.0, charger_kw: 2.0, efficiency: 0.75,
            arrival_hour: 23, departure_hour: 2, strategy: ChargeStrategy::SolarPreferred };
        let time: Vec<_> = (0..48).collect();
        let mut surplus = vec![0.0; 48];
        surplus[1] = 2000.0;
        surplus[25] = 2000.0;
        let (load, result) = charging(&ev, &time, &surplus);
        assert_eq!((load[22], load[23], load[0], load[1], load[2]), (0.0, 2000.0, 2000.0, 2000.0, 0.0));
        assert_eq!((result.charged_kwh, result.forecast_surplus_kwh), (9.0, 4.0));
        assert!((result.unscheduled_kwh - 7.0).abs() < 1e-12);
    }

    #[test]
    fn heat_pump_cutoff_and_resistance_capacity_close_the_heat_balance() {
        let h = Heating { setpoint_c: 20.0, heat_loss_kw_per_k: 0.3, thermal_kw: 6.0,
            cutoff_c: -8.0, cop_at_0c: 3.0, cop_at_10c: 4.0, resistance_backup_kw: 2.0 };
        let (load, result) = heating(&h, [0.0, -10.0].into_iter());
        assert_eq!(load, vec![2000.0, 2000.0]);
        assert_eq!(result.demand_kwh, result.pump_heat_kwh + result.backup_kwh + result.unmet_heat_kwh);
        assert_eq!(result.unmet_heat_kwh, 7.0);
    }
}
