// SPDX-License-Identifier: GPL-3.0-or-later
//! Hourly energy flows: inverters, consumption, battery dispatch and grid.
//!
//! The battery's usable energy is split into three allocations (backup,
//! self-consumption, grid services). Every stored watt-hour belongs to exactly
//! one allocation, so the same energy can never be promised twice. Charging
//! fills the allocations in the configured priority order; each use may only
//! discharge the allocations it is entitled to:
//!
//! | Use | May discharge |
//! |---|---|
//! | Normal consumption | self-consumption |
//! | Peak shaving / export window | grid services |
//! | Outage (critical load only, grid absent) | self-consumption, then grid services, then backup |
//!
//! Sign convention: all flows are non-negative magnitudes in Wh per hour.
//! The system boundary balance closes every hour:
//! PV DC + import = load served + standby + export + curtailed + clipped +
//! inverter losses + AC wiring losses + battery conversion losses + Δstored + self-discharge.
use serde::{Deserialize, Serialize};
use solarcraft_model::equipment::{BatteryInverterSpec, BatterySpec, InverterSpec};
use solarcraft_model::plant::{BatteryModel, Coupling, EnergyUse, FeedIn, GridService, Storage};
use solarcraft_model::Project;

/// One inverter's hourly DC input after DC losses.
pub struct UnitInput<'a> { pub id: &'a str, pub spec: &'a InverterSpec, pub dc_w: Vec<f64> }

/// Inverter AC output (before AC wiring) and the DC power actually converted.
pub fn inverter_ac(spec: &InverterSpec, pdc: f64) -> (f64, f64) {
    if pdc <= 0.0 { return (0.0, 0.0); }
    let pdc_lim = spec.pdc_max_w.map_or(pdc, |m| pdc.min(m));
    let ac = pdc_lim * spec.efficiency(pdc_lim);
    if ac <= spec.pac_max_w { return (ac, pdc_lim); }
    let dc = dc_for_ac(spec, spec.pac_max_w);
    (spec.pac_max_w, dc)
}

/// DC input needed for an AC output (bisection; the output is monotonic in input).
pub fn dc_for_ac(spec: &InverterSpec, ac: f64) -> f64 {
    if ac <= 0.0 { return 0.0; }
    let hi_lim = spec.pdc_max_w.unwrap_or(spec.pdc0_w() * 3.0);
    let (mut lo, mut hi) = (0.0, hi_lim);
    if hi * spec.efficiency(hi) < ac { return hi; }
    for _ in 0..60 { let m = 0.5 * (lo + hi); if m * spec.efficiency(m) < ac { lo = m } else { hi = m } }
    hi
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Flows {
    pub pv_dc: Vec<f32>,
    pub pv_ac: Vec<f32>,
    pub clipped: Vec<f32>,
    pub load: Vec<f32>,
    pub load_served: Vec<f32>,
    pub pv_to_load: Vec<f32>,
    pub batt_charge: Vec<f32>,
    pub batt_charge_grid: Vec<f32>,
    pub batt_discharge: Vec<f32>,
    pub batt_to_grid: Vec<f32>,
    pub import: Vec<f32>,
    pub export: Vec<f32>,
    pub curtailed: Vec<f32>,
    pub unserved: Vec<f32>,
    pub soc_pct: Vec<f32>,
    pub backup_wh: Vec<f32>,
    pub self_wh: Vec<f32>,
    pub grid_services_wh: Vec<f32>,
    pub outage: Vec<bool>,
    /// Hourly balance residual (should be ~0), Wh.
    pub residual: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Balance {
    pub pv_dc_kwh: f64,
    pub pv_ac_kwh: f64,
    pub clipped_kwh: f64,
    pub curtailed_kwh: f64,
    pub inverter_loss_kwh: f64,
    pub ac_wiring_loss_kwh: f64,
    pub standby_kwh: f64,
    pub load_kwh: f64,
    pub load_served_kwh: f64,
    pub pv_to_load_kwh: f64,
    pub batt_charge_kwh: f64,
    pub batt_charge_grid_kwh: f64,
    pub batt_discharge_kwh: f64,
    pub batt_to_grid_kwh: f64,
    pub batt_conversion_loss_kwh: f64,
    pub self_discharge_kwh: f64,
    pub stored_delta_kwh: f64,
    pub import_kwh: f64,
    pub export_kwh: f64,
    pub unserved_kwh: f64,
    pub shed_noncritical_kwh: f64,
    pub outage_hours: u32,
    /// Share of PV AC energy used on site (directly or via the battery).
    pub self_consumption_rate: f64,
    /// Share of consumption not supplied by the grid.
    pub autarky: f64,
    pub battery_full_cycles: f64,
    /// Remaining capacity after this year's throughput, % (detailed model).
    pub battery_capacity_after_pct: f64,
    pub max_abs_residual_wh: f64,
}

/// Stored energy above the hardware floor, split into allocations.
struct Accounts { cap: [f64; 3], e: [f64; 3] }
const IDX: fn(EnergyUse) -> usize = |u| match u { EnergyUse::Backup => 0, EnergyUse::SelfConsumption => 1, EnergyUse::GridServices => 2 };

impl Accounts {
    fn total(&self) -> f64 { self.e.iter().sum() }
    fn charge(&mut self, mut wh: f64, order: &[EnergyUse]) -> f64 {
        let start = wh;
        for &u in order { let i = IDX(u); let take = (self.cap[i] - self.e[i]).max(0.0).min(wh); self.e[i] += take; wh -= take; }
        start - wh
    }
    fn charge_one(&mut self, wh: f64, u: EnergyUse) -> f64 { self.charge(wh, &[u]) }
    fn available(&self, uses: &[EnergyUse]) -> f64 { uses.iter().map(|&u| self.e[IDX(u)]).sum() }
    fn discharge(&mut self, mut wh: f64, uses: &[EnergyUse]) -> f64 {
        let start = wh;
        for &u in uses { let i = IDX(u); let take = self.e[i].min(wh); self.e[i] -= take; wh -= take; }
        start - wh
    }
    fn room(&self, uses: &[EnergyUse]) -> f64 { uses.iter().map(|&u| (self.cap[IDX(u)] - self.e[IDX(u)]).max(0.0)).sum() }
}

const NORMAL: [EnergyUse; 1] = [EnergyUse::SelfConsumption];
const SERVICES: [EnergyUse; 1] = [EnergyUse::GridServices];
const OUTAGE: [EnergyUse; 3] = [EnergyUse::SelfConsumption, EnergyUse::GridServices, EnergyUse::Backup];

struct Batt<'a> {
    st: &'a Storage,
    spec: &'a BatterySpec,
    acc: Accounts,
    e_cap: f64,
    floor: f64,
    eta_cell: f64,
    p_ch: f64,
    p_dis: f64,
    backup_recharging: bool,
    throughput: f64,
}

impl Batt<'_> {
    fn soc(&self) -> f64 { (self.floor + self.acc.total()) / self.e_cap }
    /// Rel. position in the operating window (0..1).
    fn window_pos(&self) -> f64 { let u: f64 = self.acc.cap.iter().sum(); if u > 0.0 { self.acc.total() / u } else { 0.0 } }
    fn ch_limit(&self) -> f64 {
        if self.st.model == BatteryModel::Detailed { let x = self.window_pos(); if x > 0.9 { return self.p_ch * (0.2 + 0.8 * (1.0 - x) / 0.1); } }
        self.p_ch
    }
    fn dis_limit(&self) -> f64 {
        if self.st.model == BatteryModel::Detailed { let x = self.window_pos(); if x < 0.1 { return self.p_dis * (0.2 + 0.8 * x / 0.1); } }
        self.p_dis
    }
}

pub struct EnergyResult { pub flows: Flows, pub balance: Balance }

/// `load_w[k]`: consumption in hour k (UTC index); `local_hour[k]`: local hour of year (0..8759).
pub fn dispatch(p: &Project, units: &[UnitInput], load_w: &[f64], local_hour: &[u32], f_ac: f64) -> Result<EnergyResult, String> {
    let n = local_hour.len();
    let full_feed_in = p.grid.feed_in == FeedIn::Full;
    let export_limit = p.grid.export_limit_kw.map(|k| k * 1000.0);
    let crit = p.loads.critical_fraction;

    // Storage setup.
    let mut hybrid_unit: Option<usize> = None;
    let mut batinv: Option<&BatteryInverterSpec> = None;
    let mut port_eta = 1.0;
    let (mut port_ch, mut port_dis) = (f64::INFINITY, f64::INFINITY);
    let mut batt = match &p.storage {
        None => None,
        Some(st) => {
            let spec = p.library.battery(&st.battery_id).ok_or("battery missing")?;
            match &st.coupling {
                Coupling::DcHybrid { unit_id } => {
                    let i = units.iter().position(|u| u.id == unit_id).ok_or("hybrid inverter unit missing")?;
                    let port = units[i].spec.battery_port.as_ref().ok_or("inverter has no battery port")?;
                    hybrid_unit = Some(i); port_eta = port.dc_dc_efficiency; port_ch = port.p_charge_max_w; port_dis = port.p_discharge_max_w;
                }
                Coupling::AcCoupled { battery_inverter_id } => { batinv = Some(p.library.battery_inverter(battery_inverter_id).ok_or("battery inverter missing")?); }
            }
            let e_cap = spec.capacity_kwh * 1000.0 * st.units.max(1) as f64;
            let floor = e_cap * st.soc_min_pct / 100.0;
            let usable = e_cap * (st.soc_max_pct - st.soc_min_pct) / 100.0;
            let cap = [usable * st.backup_reserve_pct / 100.0, usable * st.self_consumption_pct() / 100.0, usable * st.grid_services_pct / 100.0];
            let mut acc = Accounts { cap, e: [0.0; 3] };
            let init = (e_cap * st.initial_soc_pct / 100.0 - floor).clamp(0.0, usable);
            acc.charge(init, &st.charge_priority);
            Some(Batt { st, spec, acc, e_cap, floor, eta_cell: spec.roundtrip_efficiency.sqrt(), p_ch: spec.p_charge_max_kw * 1000.0 * st.units.max(1) as f64,
                        p_dis: spec.p_discharge_max_kw * 1000.0 * st.units.max(1) as f64, backup_recharging: false, throughput: 0.0 })
        }
    };
    let outage_set: std::collections::HashSet<u32> = p.storage.iter().flat_map(|s| s.outages.iter()).flat_map(|o| (o.start_hour..o.start_hour + o.hours).map(|h| h % 8760)).collect();
    let bi_eta = |w: f64, model: BatteryModel| -> f64 { batinv.map_or(1.0, |b| if model == BatteryModel::Detailed { b.efficiency(w).max(0.5) } else { b.efficiency(b.p_ac_max_w * 0.5) }) };

    let mut f = Flows::default();
    let push = |v: &mut Vec<f32>, x: f64| v.push(x as f32);
    let mut b = Balance::default();
    let self_dis_per_h = p.storage.as_ref().and_then(|s| p.library.battery(&s.battery_id)).map_or(0.0, |s| s.self_discharge_pct_per_month / 100.0 / 730.0);

    for k in 0..n {
        let lh = local_hour[k];
        let outage = outage_set.contains(&lh);
        let load = load_w.get(k).copied().unwrap_or(0.0);
        let served_target = if outage { load * crit } else { load };
        // Inverter standby when a unit has no DC input.
        let mut standby: f64 = units.iter().enumerate().filter(|(i, u)| u.dc_w[k] <= 0.0 && Some(*i) != hybrid_unit).map(|(_, u)| u.spec.night_w).sum();
        if let Some(bi) = batinv { standby += bi.standby_w; }
        let pv_dc: f64 = units.iter().map(|u| u.dc_w[k].max(0.0)).sum();
        let e_before = batt.as_ref().map_or(0.0, |x| x.acc.total());
        let (mut inv_loss, mut clipped, mut ac_inv_out, mut hy_ac) = (0.0, 0.0, 0.0, 0.0);
        let (mut ch_in, mut ch_grid, mut dis_out, mut to_grid, mut conv_loss) = (0.0, 0.0, 0.0, 0.0, 0.0);
        // Non-hybrid units.
        let mut ac_other = 0.0;
        for (i, u) in units.iter().enumerate() {
            if Some(i) == hybrid_unit { continue; }
            let (ac, dc_used) = inverter_ac(u.spec, u.dc_w[k].max(0.0));
            clipped += u.dc_w[k].max(0.0) - dc_used; inv_loss += dc_used - ac; ac_inv_out += ac; ac_other += ac * f_ac;
        }
        // Demand the PV/battery side must meet on the AC bus.
        let demand = if full_feed_in { standby } else { served_target + standby };
        let mut supply = ac_other;
        let mut pv_ac_h = ac_other;

        if let (Some(hi), Some(bt)) = (hybrid_unit, batt.as_mut()) {
            // ---- DC-coupled battery on the hybrid inverter's port.
            let u = &units[hi];
            let pv_h = u.dc_w[k].max(0.0);
            let target_inv = ((demand - ac_other).max(0.0) / f_ac).min(u.spec.pac_max_w);
            let (ac_pv, _) = inverter_ac(u.spec, pv_h);
            let mut dc_in = pv_h;
            let mut pv_to_batt_dc = 0.0;
            if ac_pv >= target_inv - 1e-9 {
                let dc_target = dc_for_ac(u.spec, target_inv).min(pv_h);
                let surplus_dc = pv_h - dc_target;
                let eta = port_eta * bt.eta_cell;
                let want_cell = (surplus_dc.min(port_ch) * eta).min(bt.ch_limit() * bt.eta_cell).min(bt.acc.room(&bt.st.charge_priority));
                let got = bt.acc.charge(want_cell, &bt.st.charge_priority);
                pv_to_batt_dc = got / eta;
                ch_in += pv_to_batt_dc; conv_loss += pv_to_batt_dc - got;
                dc_in = pv_h - pv_to_batt_dc;
            } else if !full_feed_in || outage {
                let need = dc_for_ac(u.spec, target_inv) - pv_h;
                let uses: &[EnergyUse] = if outage { &OUTAGE } else { &NORMAL };
                let eta = port_eta * bt.eta_cell;
                let want_cell = (need.min(port_dis) / eta).min(bt.dis_limit() / bt.eta_cell).min(bt.acc.available(uses));
                let got = bt.acc.discharge(want_cell, uses);
                let out = got * eta;
                dis_out += out; conv_loss += got - out;
                dc_in = pv_h + out;
            }
            let (ac, dc_used) = inverter_ac(u.spec, dc_in);
            let clip_h = dc_in - dc_used;
            clipped += clip_h; inv_loss += dc_used - ac; ac_inv_out += ac; hy_ac = ac;
            supply += ac * f_ac;
            // PV share of the hybrid output (battery discharge is not PV).
            let pv_share = if dc_in > 0.0 { ((pv_h - pv_to_batt_dc - clip_h).max(0.0) / dc_in).min(1.0) } else { 0.0 };
            pv_ac_h += ac * f_ac * pv_share;
        }

        let mut import = 0.0;
        let mut export = 0.0;
        let mut curtailed = 0.0;
        let mut unserved = 0.0;
        // ---- AC-coupled battery and grid exchange.
        let net = supply - demand;
        if let (Some(bi), Some(bt)) = (batinv, batt.as_mut()) {
            if net > 0.0 {
                let ac_in = net.min(bi.p_ac_max_w);
                let eta = bi_eta(ac_in, bt.st.model) * bt.eta_cell;
                let want_cell = (ac_in * eta).min(bt.ch_limit() * bt.eta_cell).min(bt.acc.room(&bt.st.charge_priority));
                let got = bt.acc.charge(want_cell, &bt.st.charge_priority);
                let used = if eta > 0.0 { got / eta } else { 0.0 };
                ch_in += used; conv_loss += used - got;
                supply -= used;
            } else if net < 0.0 && (!full_feed_in || outage) {
                let uses: &[EnergyUse] = if outage { &OUTAGE } else { &NORMAL };
                let want_ac = (-net).min(bi.p_ac_max_w);
                let eta = bi_eta(want_ac, bt.st.model) * bt.eta_cell;
                let want_cell = (want_ac / eta).min(bt.dis_limit() / bt.eta_cell).min(bt.acc.available(uses));
                let got = bt.acc.discharge(want_cell, uses);
                let out = got * eta;
                dis_out += out; conv_loss += got - out;
                supply += out;
            }
        }
        let mut balance = supply - demand;
        // Grid services (never during outages).
        if let (Some(bt), false) = (batt.as_mut(), outage) {
            match bt.st.grid_service {
                GridService::PeakShaving { import_limit_kw } => {
                    let over = -balance - import_limit_kw * 1000.0;
                    if over > 0.0 {
                        let out = extra_discharge(bt, over, &SERVICES, hybrid_unit.map(|i| &units[i]), batinv, k, f_ac, port_eta, port_dis, &mut hy_ac, &mut conv_loss, &mut inv_loss, &mut ac_inv_out);
                        dis_out += out.0; balance += out.1;
                    }
                }
                GridService::ExportWindow { start_hour, end_hour, power_kw } => {
                    let h = lh % 24;
                    let inwin = if start_hour <= end_hour { h >= start_hour && h < end_hour } else { h >= start_hour || h < end_hour };
                    if inwin {
                        let out = extra_discharge(bt, power_kw * 1000.0, &SERVICES, hybrid_unit.map(|i| &units[i]), batinv, k, f_ac, port_eta, port_dis, &mut hy_ac, &mut conv_loss, &mut inv_loss, &mut ac_inv_out);
                        dis_out += out.0; balance += out.1; to_grid += out.1;
                    }
                }
                GridService::None => {}
            }
            // Backup reserve recovery from the grid (hysteresis 90 % → 100 %).
            if bt.st.grid_charge_backup {
                let (cap, e) = (bt.acc.cap[0], bt.acc.e[0]);
                if cap > 0.0 && e < 0.9 * cap { bt.backup_recharging = true; }
                if bt.backup_recharging {
                    let lim_ac = batinv.map_or(port_ch, |b| b.p_ac_max_w);
                    let eta = if batinv.is_some() { bi_eta(lim_ac, bt.st.model) } else { port_eta * hybrid_unit.map_or(1.0, |i| units[i].spec.efficiency(lim_ac)) } * bt.eta_cell;
                    let want_cell = ((cap - e).min(bt.ch_limit() * bt.eta_cell)).min(lim_ac * eta).max(0.0);
                    let got = bt.acc.charge_one(want_cell, EnergyUse::Backup);
                    let grid_in = if eta > 0.0 { got / eta } else { 0.0 };
                    ch_grid += grid_in; conv_loss += grid_in - got; balance -= grid_in;
                    if bt.acc.e[0] >= cap * 0.999 { bt.backup_recharging = false; }
                }
            }
        }
        if outage {
            if balance > 0.0 { curtailed = balance; } else { unserved = -balance; }
        } else if balance > 0.0 {
            export = balance;
            if let Some(l) = export_limit { if export > l { curtailed = export - l; export = l; } }
        } else {
            import = -balance;
        }
        if full_feed_in && !outage { import += served_target; }
        let served = if outage { served_target - unserved } else { served_target };
        let shed = if outage { load - served_target } else { 0.0 };
        // Self-discharge (proportional across allocations).
        let mut sd = 0.0;
        if let Some(bt) = batt.as_mut() {
            let tot = bt.acc.total();
            if tot > 0.0 { sd = (bt.floor + tot) * self_dis_per_h; sd = sd.min(tot); let s = sd / tot; for e in &mut bt.acc.e { *e -= *e * s; } }
            bt.throughput += dis_out;
        }
        let e_after = batt.as_ref().map_or(0.0, |x| x.acc.total());
        let ac_wiring = ac_inv_out * (1.0 - f_ac);
        // Stored energy change counts cell energy; conversion losses are booked separately.
        let resid = (pv_dc + import) - (served + standby + export + curtailed + clipped + inv_loss + ac_wiring + conv_loss + (e_after - e_before) + sd);
        let pv_to_load = if full_feed_in { 0.0 } else { pv_ac_h.min(served + standby) };
        for (v, x) in [(&mut f.pv_dc, pv_dc), (&mut f.pv_ac, pv_ac_h), (&mut f.clipped, clipped), (&mut f.load, load), (&mut f.load_served, served), (&mut f.pv_to_load, pv_to_load),
                       (&mut f.batt_charge, ch_in + ch_grid), (&mut f.batt_charge_grid, ch_grid), (&mut f.batt_discharge, dis_out), (&mut f.batt_to_grid, to_grid), (&mut f.import, import), (&mut f.export, export),
                       (&mut f.curtailed, curtailed), (&mut f.unserved, unserved), (&mut f.residual, resid)] { push(v, x); }
        match batt.as_ref() {
            Some(bt) => { push(&mut f.soc_pct, bt.soc() * 100.0); push(&mut f.backup_wh, bt.acc.e[0]); push(&mut f.self_wh, bt.acc.e[1]); push(&mut f.grid_services_wh, bt.acc.e[2]); }
            None => { for v in [&mut f.soc_pct, &mut f.backup_wh, &mut f.self_wh, &mut f.grid_services_wh] { v.push(0.0); } }
        }
        f.outage.push(outage);
        let kw = 1e-3;
        b.pv_dc_kwh += pv_dc * kw; b.pv_ac_kwh += pv_ac_h * kw; b.clipped_kwh += clipped * kw; b.curtailed_kwh += curtailed * kw; b.inverter_loss_kwh += inv_loss * kw;
        b.ac_wiring_loss_kwh += ac_wiring * kw; b.standby_kwh += standby * kw; b.load_kwh += load * kw; b.load_served_kwh += served * kw; b.pv_to_load_kwh += pv_to_load * kw;
        b.batt_charge_kwh += (ch_in + ch_grid) * kw; b.batt_charge_grid_kwh += ch_grid * kw; b.batt_discharge_kwh += dis_out * kw; b.batt_to_grid_kwh += to_grid * kw;
        b.batt_conversion_loss_kwh += conv_loss * kw; b.self_discharge_kwh += sd * kw; b.stored_delta_kwh += (e_after - e_before) * kw;
        b.import_kwh += import * kw; b.export_kwh += export * kw; b.unserved_kwh += unserved * kw; b.shed_noncritical_kwh += shed * kw;
        b.outage_hours += u32::from(outage);
        b.max_abs_residual_wh = b.max_abs_residual_wh.max(resid.abs());
    }
    b.self_consumption_rate = if b.pv_ac_kwh > 0.0 { ((b.pv_ac_kwh - b.export_kwh - b.curtailed_kwh).max(0.0) / b.pv_ac_kwh).min(1.0) } else { 0.0 };
    b.autarky = if b.load_kwh > 0.0 { (1.0 - b.import_kwh.min(b.load_kwh) / b.load_kwh).max(0.0) } else { 0.0 };
    if let Some(bt) = &batt {
        let usable: f64 = bt.acc.cap.iter().sum();
        b.battery_full_cycles = if usable > 0.0 { bt.throughput / usable } else { 0.0 };
        b.battery_capacity_after_pct = 100.0 * (1.0 - 0.2 * b.battery_full_cycles / bt.spec.cycle_life.max(1.0));
    }
    Ok(EnergyResult { flows: f, balance: b })
}

/// Discharge `ac_wanted` (AC Wh) from the given allocations through the battery's converter path.
/// Returns (energy out of the storage system, AC energy added to the bus).
#[allow(clippy::too_many_arguments)]
fn extra_discharge(bt: &mut Batt, ac_wanted: f64, uses: &[EnergyUse], hybrid: Option<&UnitInput>, batinv: Option<&BatteryInverterSpec>, k: usize, f_ac: f64,
                   port_eta: f64, port_dis: f64, hy_ac: &mut f64, conv_loss: &mut f64, inv_loss: &mut f64, ac_inv_out: &mut f64) -> (f64, f64) {
    if ac_wanted <= 0.0 { return (0.0, 0.0); }
    if let Some(bi) = batinv {
        let want = ac_wanted.min(bi.p_ac_max_w);
        let eta = (if bt.st.model == BatteryModel::Detailed { bi.efficiency(want).max(0.5) } else { bi.efficiency(bi.p_ac_max_w * 0.5) }) * bt.eta_cell;
        let got = bt.acc.discharge((want / eta).min(bt.dis_limit() / bt.eta_cell).min(bt.acc.available(uses)), uses);
        let out = got * eta;
        *conv_loss += got - out;
        return (out, out);
    }
    let Some(u) = hybrid else { return (0.0, 0.0) };
    let _ = k;
    // Hybrid: extra DC through the same inverter, limited by its AC headroom.
    let headroom = (u.spec.pac_max_w - *hy_ac).max(0.0);
    let ac_add = (ac_wanted / f_ac).min(headroom);
    if ac_add <= 0.0 { return (0.0, 0.0); }
    let dc_now = dc_for_ac(u.spec, *hy_ac);
    let dc_need = dc_for_ac(u.spec, *hy_ac + ac_add) - dc_now;
    let eta = port_eta * bt.eta_cell;
    let got = bt.acc.discharge((dc_need.max(0.0).min(port_dis) / eta).min(bt.dis_limit() / bt.eta_cell).min(bt.acc.available(uses)), uses);
    let out_dc = got * eta;
    *conv_loss += got - out_dc;
    let (ac_new, _) = inverter_ac(u.spec, dc_now + out_dc);
    let ac_gain = (ac_new - *hy_ac).max(0.0);
    *inv_loss += out_dc - ac_gain;
    *ac_inv_out += ac_gain;
    *hy_ac += ac_gain;
    (out_dc, ac_gain * f_ac)
}
