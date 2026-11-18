// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{energy::{self, Balance, EnergyResult, UnitInput}, wiring::{Delivery, Routed}};
use serde::{Deserialize, Serialize};
use solarcraft_model::Project;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterResult {
    pub name: String,
    pub storage_kwh: f64,
    pub fuel_unit: Option<String>,
    pub balance: Balance,
}

pub(crate) struct Run {
    pub energy: EnergyResult,
    pub clusters: Vec<ClusterResult>,
    pub unserved_wh: Vec<Vec<f32>>,
    pub soc_pct: Vec<Vec<f32>>,
}

pub(crate) fn simulate(p: &Project, units: &[UnitInput], load: &[f64], local: &[u32], ac: Delivery, routed: &Routed) -> Result<Run, String> {
    let mut combined: Option<EnergyResult> = None;
    let mut storage_total = 0.0;
    let mut clusters = vec![];
    let mut unserved_wh = vec![];
    let mut soc_pct = vec![];
    let mut fuel_unit: Option<String> = None;
    let mut mixed_fuels = false;
    for cluster in &p.island_clusters {
        let local_units: Vec<_> = units.iter().filter(|u| cluster.inverter_units.iter().any(|id| id == u.id)).cloned().collect();
        let demand: Vec<_> = load.iter().map(|&w| w * cluster.load_share_pct / 100.0).collect();
        let mut design = p.clone();
        design.island_clusters.clear();
        design.storage = Some(cluster.storage.clone());
        design.island_plan = cluster.controls.clone();
        let mut run = energy::dispatch_bus(&design, &local_units, &demand, local, ac, &[], routed)?;
        let storage = p.library.battery(&cluster.storage.battery_id).ok_or("cluster battery missing")?
            .capacity_kwh * f64::from(cluster.storage.units);
        let unit = cluster.storage.generator_id.as_ref().and_then(|id| p.library.generator(id)).map(|g| g.fuel_unit.clone());
        if let Some(u) = &unit {
            mixed_fuels |= fuel_unit.as_ref().is_some_and(|prior| prior != u);
            fuel_unit.get_or_insert_with(|| u.clone());
        }
        clusters.push(ClusterResult { name: cluster.name.clone(), storage_kwh: storage, fuel_unit: unit, balance: run.balance.clone() });
        unserved_wh.push(run.flows.unserved.clone());
        soc_pct.push(run.flows.soc_pct.clone());
        for soc in &mut run.flows.soc_pct { *soc *= storage as f32; }
        run.balance.battery_full_cycles *= storage;
        run.balance.battery_capacity_after_pct *= storage;
        run.balance.self_consumption_rate *= run.balance.pv_generation_kwh;
        storage_total += storage;
        if let Some(total) = &mut combined { merge(total, run); } else { combined = Some(run); }
    }
    let mut energy = combined.ok_or("missing islands")?;
    let b = &mut energy.balance;
    b.autarky = if b.load_kwh > 0.0 { (b.load_served_kwh / b.load_kwh).clamp(0.0, 1.0) } else { 0.0 };
    b.self_consumption_rate = if b.pv_generation_kwh > 0.0 { b.self_consumption_rate / b.pv_generation_kwh } else { 0.0 };
    b.battery_full_cycles /= storage_total;
    b.battery_capacity_after_pct /= storage_total;
    for soc in &mut energy.flows.soc_pct { *soc /= storage_total as f32; }
    if mixed_fuels { b.generator_fuel = 0.0; }
    Ok(Run { energy, clusters, unserved_wh, soc_pct })
}

fn merge(total: &mut EnergyResult, next: EnergyResult) {
    macro_rules! sum_hours {
        ($($field:ident),+ $(,)?) => {
            $(for (a, b) in total.flows.$field.iter_mut().zip(next.flows.$field) { *a += b; })+
        };
    }
    sum_hours!(pv_dc, pv_ac, clipped, load, load_served, pv_to_load, battery_to_load, grid_to_load, generator_to_load, ev_to_load,
        batt_charge, batt_charge_grid, batt_discharge, batt_to_grid, import, export, curtailed, unserved, generator,
        generator_charge, ac_cable_loss, soc_pct, backup_wh, self_wh, grid_services_wh, residual);
    total.flows.routed_ac_wh.extend(next.flows.routed_ac_wh);
    let (a, b) = (&mut total.balance, next.balance);
    macro_rules! sum_balance {
        ($($field:ident),+ $(,)?) => { $(a.$field += b.$field;)+ };
    }
    sum_balance!(pv_dc_kwh, pv_ac_kwh, pv_to_battery_dc_kwh, pv_generation_kwh, clipped_kwh, curtailed_kwh, inverter_loss_kwh,
        ac_wiring_loss_kwh, standby_kwh, load_kwh, load_served_kwh, pv_to_load_kwh, battery_to_load_kwh, grid_to_load_kwh,
        generator_to_load_kwh, ev_to_load_kwh, batt_charge_kwh, batt_charge_grid_kwh, batt_discharge_kwh, batt_to_grid_kwh,
        batt_conversion_loss_kwh, self_discharge_kwh, stored_delta_kwh, import_kwh, export_kwh, unserved_kwh, generator_kwh,
        generator_fuel, generator_starts, generator_hours, generator_charge_kwh, shed_noncritical_kwh,
        self_consumption_rate, battery_full_cycles, battery_capacity_after_pct);
    a.max_abs_residual_wh += b.max_abs_residual_wh;
}
