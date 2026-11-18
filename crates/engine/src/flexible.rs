// SPDX-License-Identifier: GPL-3.0-or-later
use crate::calendar;
use serde::{Deserialize, Serialize};
use solarcraft_model::demand::{ChargeStrategy, Fleet, FleetGroup, HeatStore, Heating};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VehicleResult {
    pub name: String,
    pub vehicles: u32,
    pub departures: u32,
    pub departures_below_target: u32,
    pub charged_kwh: f64,
    pub discharged_kwh: f64,
    pub travelled_kwh: f64,
    pub unmet_travel_kwh: f64,
    pub initial_kwh: f64,
    pub final_kwh: f64,
    pub conversion_loss_kwh: f64,
    pub min_soc_pct: f64,
    pub max_soc_pct: f64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FleetResult {
    pub groups: Vec<VehicleResult>,
    pub charge_kwh: f64,
    pub v2h_kwh: f64,
}

pub struct FleetRun {
    pub result: FleetResult,
    pub charge: Vec<f32>,
    pub discharge: Vec<f32>,
    /// Per group, end-of-hour SOC in weather order.
    pub soc: Vec<Vec<f32>>,
}

struct Vehicle<'a> {
    model: &'a FleetGroup,
    capacity: f64,
    target: f64,
    stored: f64,
    limit: f64,
    trip: f64,
    departure: Vec<bool>,
    remaining: Vec<u32>,
    result: VehicleResult,
    soc: Vec<f32>,
}

impl<'a> Vehicle<'a> {
    fn new(model: &'a FleetGroup, n: usize) -> Self {
        let mut departure = vec![false; n];
        let mut remaining = vec![0; n];
        let window = (model.departure_hour + 24 - model.arrival_hour) % 24;
        let window = if window == 0 { 24 } else { window as i32 };
        for day in -1..((n / 24) as i32) {
            let weekday = (day + 2).rem_euclid(7) as u32;
            if !model.weekdays.contains(&weekday) { continue; }
            let start = day * 24 + model.arrival_hour as i32;
            let end = start + window;
            if (0..n as i32).contains(&end) { departure[end as usize] = true; }
            for hour in start.max(0)..end.min(n as i32) { remaining[hour as usize] = (end - hour) as u32; }
        }
        let capacity = model.battery_kwh * f64::from(model.vehicles) * 1000.0;
        let trip = model.trip_kwh * f64::from(model.vehicles) * 1000.0;
        let stored = capacity * model.initial_soc_pct / 100.0;
        Self {
            model, capacity, trip, target: (capacity * model.departure_soc_pct / 100.0).max(trip),
            stored, limit: model.charge_kw * f64::from(model.vehicles) * 1000.0, departure, remaining,
            result: VehicleResult { name: model.name.clone(), vehicles: model.vehicles,
                initial_kwh: stored / 1000.0, min_soc_pct: model.initial_soc_pct, max_soc_pct: model.initial_soc_pct, ..Default::default() },
            soc: vec![0.0; n],
        }
    }

    fn target_input(&self) -> f64 { (self.target - self.stored).max(0.0) / self.model.charge_efficiency }

    fn charge(&mut self, wh: f64) {
        self.stored += wh * self.model.charge_efficiency;
        self.result.charged_kwh += wh / 1000.0;
        self.result.conversion_loss_kwh += wh * (1.0 - self.model.charge_efficiency) / 1000.0;
    }
}

fn calendar_order(local_hour: &[u32]) -> Vec<usize> {
    let mut order = vec![0; local_hour.len()];
    for (i, &local) in local_hour.iter().enumerate() { order[local as usize] = i; }
    order
}

/// Forward dispatch with a departure guard: allocate shared charger power to its
/// earliest deadlines first, then use remaining headroom for immediate or solar charging.
pub fn fleet(plan: &Fleet, local_hour: &[u32], load_w: &[f64], pv_w: &[f32], grid_online: &[bool], v2h_enabled: bool) -> FleetRun {
    let n = local_hour.len();
    let mut vehicles: Vec<_> = plan.groups.iter().map(|g| Vehicle::new(g, n)).collect();
    let mut run = FleetRun { result: FleetResult::default(), charge: vec![0.0; n], discharge: vec![0.0; n], soc: vec![] };
    let order = calendar_order(local_hour);
    let mut online_prefix = Vec::with_capacity(n + 1);
    online_prefix.push(0u32);
    for &i in &order {
        online_prefix.push(online_prefix.last().copied().unwrap_or(0) + u32::from(grid_online[i]));
    }
    for (h, i) in order.into_iter().enumerate() {
        let mut connected = vec![];
        for (idx, v) in vehicles.iter_mut().enumerate() {
            if v.departure[h] {
                v.result.departures += v.model.vehicles;
                if v.stored + 1e-6 < v.target { v.result.departures_below_target += v.model.vehicles; }
                let moved = v.trip.min(v.stored);
                v.stored -= moved;
                v.result.travelled_kwh += moved / 1000.0;
                v.result.unmet_travel_kwh += (v.trip - moved) / 1000.0;
            }
            if v.remaining[h] > 0 && grid_online[i] { connected.push(idx); }
        }
        connected.sort_by_key(|&idx| vehicles[idx].remaining[h]);
        let future_hours = |remaining: u32| f64::from(
            online_prefix[(h + remaining as usize).min(n)] - online_prefix[h + 1]
        );
        let mut left = plan.site_charge_kw * 1000.0;
        let mut due = 0.0;
        let mut shared_required: f64 = 0.0;
        for &idx in &connected {
            let v = &vehicles[idx];
            due += v.target_input();
            shared_required = shared_required.max(due - future_hours(v.remaining[h]) * left);
        }
        let mut spare = (f64::from(pv_w[i]) - load_w[i]).max(0.0);
        let mut charged = vec![0.0; vehicles.len()];
        for &idx in &connected {
            let v = &mut vehicles[idx];
            let future = future_hours(v.remaining[h]) * v.limit.min(plan.site_charge_kw * 1000.0);
            let amount = (v.target_input() - future).max(0.0).min(v.limit).min(left);
            v.charge(amount);
            charged[idx] = amount;
            left -= amount;
            spare = (spare - amount).max(0.0);
            shared_required -= amount;
        }
        for &idx in &connected {
            let v = &mut vehicles[idx];
            let amount = shared_required.max(0.0).min(v.target_input()).min(v.limit - charged[idx]).min(left);
            v.charge(amount);
            charged[idx] += amount;
            shared_required -= amount;
            left -= amount;
            spare = (spare - amount).max(0.0);
        }
        for &idx in &connected {
            let v = &mut vehicles[idx];
            let can_discharge = v2h_enabled && plan.site_discharge_kw > 0.0 && v.model.discharge_kw > 0.0;
            let room = (v.capacity - v.stored).max(0.0) / v.model.charge_efficiency;
            let desire = match v.model.strategy {
                ChargeStrategy::Immediate => v.target_input().max(if can_discharge { spare.min(room) } else { 0.0 }),
                ChargeStrategy::SolarPreferred => spare.min(if can_discharge { room } else { v.target_input() }),
            };
            let amount = desire.min(v.limit - charged[idx]).min(left);
            v.charge(amount);
            left -= amount;
            spare = (spare - amount).max(0.0);
        }
        let charge = (plan.site_charge_kw * 1000.0 - left).max(0.0);
        let mut discharge = 0.0;
        if charge < 1e-7 && v2h_enabled && grid_online[i] {
            let mut deficit = (load_w[i] - f64::from(pv_w[i]).max(0.0)).max(0.0).min(plan.site_discharge_kw * 1000.0);
            for &idx in &connected {
                let v = &mut vehicles[idx];
                let export = ((v.stored - v.target).max(0.0) * v.model.discharge_efficiency)
                    .min(v.model.discharge_kw * f64::from(v.model.vehicles) * 1000.0).min(deficit);
                let draw = export / v.model.discharge_efficiency;
                v.stored -= draw;
                v.result.discharged_kwh += export / 1000.0;
                v.result.conversion_loss_kwh += (draw - export) / 1000.0;
                deficit -= export;
                discharge += export;
            }
        }
        run.charge[i] = charge as f32;
        run.discharge[i] = discharge as f32;
        run.result.charge_kwh += charge / 1000.0;
        run.result.v2h_kwh += discharge / 1000.0;
        for v in &mut vehicles {
            let soc = (100.0 * v.stored / v.capacity).clamp(0.0, 100.0);
            v.soc[i] = soc as f32;
            v.result.min_soc_pct = v.result.min_soc_pct.min(soc);
            v.result.max_soc_pct = v.result.max_soc_pct.max(soc);
        }
    }
    for mut v in vehicles {
        v.result.final_kwh = v.stored / 1000.0;
        run.result.groups.push(v.result);
        run.soc.push(v.soc);
    }
    run
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TankResult {
    pub charge_heat_kwh: f64,
    pub charge_electric_kwh: f64,
    pub discharge_heat_kwh: f64,
    pub standing_loss_kwh: f64,
    pub end_kwh: f64,
    pub hot_water_kwh: f64,
}

pub struct TankRun {
    pub demand: super::demand::HeatingResult,
    pub result: TankResult,
    pub electrical: Vec<f32>,
    pub stored: Vec<f32>,
}

pub fn thermal_store(heat: &Heating, tank: &HeatStore, temp: &[f64], local_hour: &[u32], load: &[f64], pv: &[f32]) -> TankRun {
    let n = local_hour.len();
    let mut run = TankRun { demand: Default::default(), result: Default::default(), electrical: vec![0.0; n], stored: vec![0.0; n] };
    let loss_fraction = 1.0 - (1.0 - tank.standing_loss_pct_per_day / 100.0).powf(1.0 / 24.0);
    let mut stored = tank.initial_kwh;
    for (h, i) in calendar_order(local_hour).into_iter().enumerate() {
        let loss = stored * loss_fraction;
        stored -= loss;
        let hot_water = if calendar::local_time(h as u32).hour == tank.hot_water_hour { tank.daily_hot_water_kwh } else { 0.0 };
        let demand = heat.heat_loss_kw_per_k * (heat.setpoint_c - temp[i]).max(0.0) + hot_water;
        let capacity = if temp[i] >= heat.cutoff_c { heat.thermal_kw } else { 0.0 };
        let cop = super::demand::cop(heat, temp[i]);
        let spare = ((f64::from(pv[i]) - load[i]) / 1000.0).max(0.0);
        let solar_heat = demand.min(capacity).min(spare * cop);
        let discharge = (demand - solar_heat).max(0.0).min(stored).min(tank.discharge_kw);
        let pump = (demand - discharge).min(capacity);
        let backup = (demand - discharge - pump).max(0.0).min(heat.resistance_backup_kw);
        let charge = if discharge <= 1e-9 {
            ((spare - pump / cop - backup).max(0.0) * cop).min((capacity - pump).max(0.0))
                .min(tank.capacity_kwh - stored).min(tank.charge_kw)
        } else { 0.0 };
        stored += charge - discharge;
        run.stored[i] = stored as f32;
        run.electrical[i] = ((pump / cop + backup + charge / cop) * 1000.0) as f32;
        run.demand.demand_kwh += demand;
        run.demand.pump_heat_kwh += pump;
        run.demand.pump_electric_kwh += (pump + charge) / cop;
        run.demand.backup_kwh += backup;
        run.demand.unmet_heat_kwh += (demand - discharge - pump - backup).max(0.0);
        run.result.charge_heat_kwh += charge;
        run.result.charge_electric_kwh += charge / cop;
        run.result.discharge_heat_kwh += discharge;
        run.result.standing_loss_kwh += loss;
        run.result.hot_water_kwh += hot_water;
    }
    run.result.end_kwh = stored;
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_deadlines_leave_time_on_the_shared_charger() {
        let group = FleetGroup {
            name: "First".into(), vehicles: 1, battery_kwh: 20.0, initial_soc_pct: 0.0,
            departure_soc_pct: 20.0, trip_kwh: 1.0, weekdays: vec![2],
            arrival_hour: 1, departure_hour: 5, charge_kw: 2.0, charge_efficiency: 1.0,
            discharge_kw: 0.0, discharge_efficiency: 1.0, strategy: ChargeStrategy::SolarPreferred,
        };
        let mut second = group.clone();
        second.name = "Second".into();
        let plan = Fleet { groups: vec![group, second], site_charge_kw: 2.0, site_discharge_kw: 0.0 };
        let hours: Vec<_> = (0..24).collect();
        let run = fleet(&plan, &hours, &[0.0; 24], &[0.0; 24], &[true; 24], false);
        assert_eq!(&run.charge[1..5], &[2000.0; 4]);
        assert!(run.result.groups.iter().all(|r| r.departures == 1 && r.departures_below_target == 0));
        let mut blackout = [true; 24];
        blackout[4] = false;
        let run = fleet(&plan, &hours, &[0.0; 24], &[0.0; 24], &blackout, true);
        assert_eq!(run.charge[4], 0.0);
        assert_eq!(run.result.groups.iter().map(|r| r.departures_below_target).sum::<u32>(), 1);
    }
}
