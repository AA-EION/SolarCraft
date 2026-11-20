// SPDX-License-Identifier: GPL-3.0-or-later
use crate::calendar;
use serde::{Deserialize, Serialize};
use solarcraft_model::demand::{ChargeStrategy, Fleet, FleetGroup, HeatStore, Heating};

mod thermal;

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
    #[serde(default)]
    pub v2g_kwh: f64,
}

pub struct FleetRun {
    pub result: FleetResult,
    pub charge: Vec<f32>,
    pub discharge: Vec<f32>,
    pub export: Vec<f32>,
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
pub fn fleet(plan: &Fleet, local_hour: &[u32], load_w: &[f64], pv_w: &[f32], grid_online: &[bool], bidirectional: bool, site_export_limit_w: Option<f64>) -> FleetRun {
    let n = local_hour.len();
    let mut vehicles: Vec<_> = plan.groups.iter().map(|g| Vehicle::new(g, n)).collect();
    let mut run = FleetRun { result: FleetResult::default(), charge: vec![0.0; n], discharge: vec![0.0; n], export: vec![0.0; n], soc: vec![] };
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
        let local = calendar::local_time(h as u32);
        let net = f64::from(pv_w[i]).max(0.0) - load_w[i];
        let scheduled_export = plan.v2g.as_ref().filter(|p| bidirectional && p.active(local.hour, local.weekday))
            .map_or(0.0, |p| {
                let headroom = site_export_limit_w.map_or(f64::INFINITY, |limit| (limit - net.max(0.0)).max(0.0));
                (p.limit_kw * 1000.0).min(headroom)
            });
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
            let can_discharge = bidirectional && plan.site_discharge_kw > 0.0 && v.model.discharge_kw > 0.0;
            let room = (v.capacity - v.stored).max(0.0) / v.model.charge_efficiency;
            let available_solar = if scheduled_export > 0.0 { 0.0 } else { spare };
            let desire = match v.model.strategy {
                ChargeStrategy::Immediate => v.target_input().max(if can_discharge { available_solar.min(room) } else { 0.0 }),
                ChargeStrategy::SolarPreferred => available_solar.min(if can_discharge { room } else { v.target_input() }),
            };
            let amount = desire.min(v.limit - charged[idx]).min(left);
            v.charge(amount);
            left -= amount;
            spare = (spare - amount).max(0.0);
        }
        let charge = (plan.site_charge_kw * 1000.0 - left).max(0.0);
        let mut discharge = 0.0;
        let deficit = (-net).max(0.0);
        if charge < 1e-7 && bidirectional && grid_online[i] {
            let mut available = (deficit + scheduled_export).min(plan.site_discharge_kw * 1000.0);
            for &idx in &connected {
                let v = &mut vehicles[idx];
                let export = ((v.stored - v.target).max(0.0) * v.model.discharge_efficiency)
                    .min(v.model.discharge_kw * f64::from(v.model.vehicles) * 1000.0).min(available);
                let draw = export / v.model.discharge_efficiency;
                v.stored -= draw;
                v.result.discharged_kwh += export / 1000.0;
                v.result.conversion_loss_kwh += (draw - export) / 1000.0;
                available -= export;
                discharge += export;
            }
        }
        run.charge[i] = charge as f32;
        run.discharge[i] = discharge.min(deficit) as f32;
        run.export[i] = (discharge - deficit).max(0.0) as f32;
        run.result.charge_kwh += charge / 1000.0;
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
    #[serde(default)]
    pub end_top_c: Option<f64>,
    #[serde(default)]
    pub end_bottom_c: Option<f64>,
    #[serde(default)]
    pub tank_to_hot_water_kwh: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boiler_heat_kwh: Option<f64>,
    /// Electricity covered by simultaneous PV after base load, before batteries and EV charging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_solar_electric_kwh: Option<f64>,
}

pub struct TankRun {
    pub demand: super::demand::HeatingResult,
    pub result: TankResult,
    pub electrical: Vec<f32>,
    pub stored: Vec<f32>,
    pub top_c: Vec<f32>,
    pub bottom_c: Vec<f32>,
}

/// Each node stores heat above room temperature. Capacity at the target gives
/// its heat capacity in kWh per kelvin.
pub(crate) enum StoreState<'a> {
    Mixed(f64),
    Layered { spec: &'a solarcraft_model::demand::TankLayers, kwh: [f64; 2], per_k: [f64; 2] },
}

impl<'a> StoreState<'a> {
    fn new(tank: &'a HeatStore) -> Self {
        match &tank.layers {
            Some(spec) => {
                let top = tank.capacity_kwh * spec.top_fraction;
                let delta = spec.target_c - spec.ambient_c;
                Self::Layered {
                    spec, kwh: [tank.initial_kwh.min(top), (tank.initial_kwh - top).max(0.0)],
                    per_k: [top / delta, (tank.capacity_kwh - top) / delta],
                }
            }
            None => Self::Mixed(tank.initial_kwh),
        }
    }

    fn total(&self) -> f64 { match self { Self::Mixed(kwh) => *kwh, Self::Layered { kwh, .. } => kwh.iter().sum() } }

    fn temperatures(&self) -> Option<[f64; 2]> {
        let Self::Layered { spec, kwh, per_k } = self else { return None; };
        Some([spec.ambient_c + kwh[0] / per_k[0], spec.ambient_c + kwh[1] / per_k[1]])
    }

    /// Conduction is integrated analytically for one hour without external
    /// effects. An inverted gradient mixes to equilibrium at fixed total heat.
    fn settle(&mut self) {
        let Self::Layered { spec, kwh, per_k } = self else { return; };
        let difference = kwh[0] / per_k[0] - kwh[1] / per_k[1];
        let coupling = 1.0 / per_k[0] + 1.0 / per_k[1];
        let equilibrium_transfer = difference / coupling;
        let fraction = if difference < 0.0 { 1.0 } else { -(-spec.conductance_w_per_k / 1000.0 * coupling).exp_m1() };
        kwh[0] -= equilibrium_transfer * fraction;
        kwh[1] += equilibrium_transfer * fraction;
    }

    fn lose(&mut self, fraction: f64) -> f64 {
        let drop = |energy: &mut f64| { let loss = *energy * fraction; *energy -= loss; loss };
        match self { Self::Mixed(kwh) => drop(kwh), Self::Layered { kwh, .. } => kwh.iter_mut().map(drop).sum() }
    }

    fn draw(&mut self, water: f64, space: f64, limit: f64) -> (f64, f64) {
        match self {
            Self::Mixed(kwh) => {
                let from_water = water.min(limit).min(*kwh);
                let from_space = space.min(limit - from_water).min(*kwh - from_water);
                *kwh -= from_water + from_space;
                (from_water, from_space)
            }
            Self::Layered { spec, kwh, per_k } => {
                let take = |stored: &mut f64, floor: f64, request: f64| {
                    let used = (*stored - floor).max(0.0).min(request);
                    *stored -= used;
                    used
                };
                let from_water = take(&mut kwh[0], (spec.hot_water_supply_c - spec.ambient_c) * per_k[0], water.min(limit));
                let space = space.min(limit - from_water);
                let low = take(&mut kwh[1], (spec.space_supply_c - spec.ambient_c) * per_k[1], space);
                let high = take(&mut kwh[0], (spec.space_supply_c - spec.ambient_c) * per_k[0], space - low);
                // Mix only an inversion created by this draw. Normal conduction
                // is applied once at the start of the next hour.
                if kwh[0] / per_k[0] < kwh[1] / per_k[1] {
                    let level = (kwh[0] + kwh[1]) / (per_k[0] + per_k[1]);
                    *kwh = [level * per_k[0], level * per_k[1]];
                }
                (from_water, low + high)
            }
        }
    }

    fn charge(&mut self, heat: f64) {
        match self {
            Self::Mixed(kwh) => *kwh += heat,
            Self::Layered { spec, kwh, per_k } => {
                let first = heat.min(((spec.target_c - spec.ambient_c) * per_k[0] - kwh[0]).max(0.0));
                kwh[0] += first;
                kwh[1] += heat - first;
            }
        }
    }
}

pub fn thermal_store(heat: &Heating, tank: &HeatStore, temp: &[f64], local_hour: &[u32], load: &[f64], pv: &[f32]) -> TankRun {
    if let Some(controls) = &tank.controls {
        return thermal::mixed_tank(heat, tank, controls, temp, local_hour, load, pv);
    }
    let n = local_hour.len();
    let mut run = TankRun { demand: Default::default(), result: Default::default(), electrical: vec![0.0; n], stored: vec![0.0; n],
        top_c: vec![], bottom_c: vec![] };
    let loss_fraction = 1.0 - (1.0 - tank.standing_loss_pct_per_day / 100.0).powf(1.0 / 24.0);
    let mut stored = StoreState::new(tank);
    if tank.layers.is_some() { run.top_c = vec![0.0; n]; run.bottom_c = vec![0.0; n]; }
    for (h, i) in calendar_order(local_hour).into_iter().enumerate() {
        stored.settle();
        let loss = stored.lose(loss_fraction);
        let hot_water = if calendar::local_time(h as u32).hour == tank.hot_water_hour { tank.daily_hot_water_kwh } else { 0.0 };
        let space = heat.heat_loss_kw_per_k * (heat.setpoint_c - temp[i]).max(0.0);
        let demand = space + hot_water;
        let capacity = if temp[i] >= heat.cutoff_c { heat.thermal_kw } else { 0.0 };
        let cop = super::demand::cop(heat, temp[i]);
        let spare = ((f64::from(pv[i]) - load[i]) / 1000.0).max(0.0);
        let solar_heat = demand.min(capacity).min(spare * cop);
        let (water_draw, space_draw) = stored.draw((hot_water - solar_heat).max(0.0),
            (space - (solar_heat - hot_water).max(0.0)).max(0.0), tank.discharge_kw);
        let discharge = water_draw + space_draw;
        let pump = (demand - discharge).min(capacity);
        let backup = (demand - discharge - pump).max(0.0).min(heat.resistance_backup_kw);
        let charge = if discharge <= 1e-9 {
            ((spare - pump / cop - backup).max(0.0) * cop).min((capacity - pump).max(0.0))
                .min((tank.capacity_kwh - stored.total()).max(0.0)).min(tank.charge_kw)
        } else { 0.0 };
        stored.charge(charge);
        run.stored[i] = stored.total() as f32;
        if let Some([top, bottom]) = stored.temperatures() {
            run.top_c[i] = top as f32;
            run.bottom_c[i] = bottom as f32;
        }
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
        run.result.tank_to_hot_water_kwh += water_draw;
    }
    run.result.end_kwh = stored.total();
    if let Some([top, bottom]) = stored.temperatures() {
        run.result.end_top_c = Some(top);
        run.result.end_bottom_c = Some(bottom);
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stratified_store_keeps_heat_and_requires_the_hot_water_supply_temperature() {
        let tank = HeatStore {
            capacity_kwh: 10.0, initial_kwh: 6.0, charge_kw: 3.0, discharge_kw: 5.0, standing_loss_pct_per_day: 0.0,
            daily_hot_water_kwh: 4.0, hot_water_hour: 19,
            controls: None, layers: Some(solarcraft_model::demand::TankLayers {
                top_fraction: 0.4, ambient_c: 20.0, target_c: 65.0, space_supply_c: 32.0, hot_water_supply_c: 50.0,
                conductance_w_per_k: 20.0,
            }),
        };
        let mut state = StoreState::new(&tank);
        let initial = state.temperatures().unwrap();
        assert_eq!(initial, [65.0, 35.0]);
        state.settle();
        let actual = state.temperatures().unwrap();
        assert!(actual[0] < initial[0] && actual[1] > initial[1]);
        assert!((state.total() - 6.0).abs() < 1e-12);
        let (hot_water, space_heat) = state.draw(4.0, 1.0, 5.0);
        assert!(hot_water > 0.0 && hot_water < 4.0 && space_heat > 0.0);
        assert!((state.total() + hot_water + space_heat - 6.0).abs() < 1e-12);
        assert!(state.draw(4.0, 0.0, 5.0).0 < 1e-9);
    }

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
        let plan = Fleet { groups: vec![group, second], site_charge_kw: 2.0, site_discharge_kw: 0.0, v2g: None };
        let hours: Vec<_> = (0..24).collect();
        let run = fleet(&plan, &hours, &[0.0; 24], &[0.0; 24], &[true; 24], false, None);
        assert_eq!(&run.charge[1..5], &[2000.0; 4]);
        assert!(run.result.groups.iter().all(|r| r.departures == 1 && r.departures_below_target == 0));
        let mut blackout = [true; 24];
        blackout[4] = false;
        let run = fleet(&plan, &hours, &[0.0; 24], &[0.0; 24], &blackout, true, None);
        assert_eq!(run.charge[4], 0.0);
        assert_eq!(run.result.groups.iter().map(|r| r.departures_below_target).sum::<u32>(), 1);
    }
}
