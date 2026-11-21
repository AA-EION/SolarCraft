// SPDX-License-Identifier: GPL-3.0-or-later
//! Stationary AC battery groups on one point of common coupling.
use super::{Batt, BatteryBankFlow, TransferBudget, NORMAL, OUTAGE, SERVICES};
use solarcraft_model::equipment::{BatteryInverterSpec, BatterySpec, Provenance};
use solarcraft_model::plant::{BatteryModel, Coupling, EnergyUse, GridService, Storage, TimedBatteryOperation};
use solarcraft_model::Project;

struct Bank<'a> {
    state: Batt<'a>,
    converter: std::borrow::Cow<'a, BatteryInverterSpec>,
    charge_w: f64,
    discharge_w: f64,
    meter: BatteryBankFlow,
}

struct BankHour<'a> {
    charge: TransferBudget,
    discharge: TransferBudget,
    timed_charge: Option<&'a TimedBatteryOperation>,
    timed_discharge: Option<&'a TimedBatteryOperation>,
    charge_wh: f64,
    grid_charge_wh: f64,
    discharge_wh: f64,
    loss_wh: f64,
}

pub(super) struct AcFleet<'a> {
    banks: Vec<Bank<'a>>,
    nominal_power_w: f64,
    capacity_wh: f64,
}

#[derive(Default)]
pub(super) struct FleetHour {
    pub balance_wh: f64,
    pub pv_charge_wh: f64,
    pub grid_charge_wh: f64,
    pub discharge_wh: f64,
    pub export_wh: f64,
    pub loss_wh: f64,
    pub self_discharge_wh: f64,
}

impl<'a> Bank<'a> {
    fn new(p: &'a Project, st: &'a Storage, spec: &'a BatterySpec, hours: usize) -> Result<Self, String> {
        use std::borrow::Cow;
        let copies = f64::from(st.units);
        let (converter, charge_w, discharge_w) = match &st.coupling {
            Coupling::Integrated => {
                let c = spec.converter.as_ref().ok_or("integrated converter missing")?;
                let converter = BatteryInverterSpec {
                    id: String::new(), manufacturer: String::new(), model: String::new(),
                    catalog_ref: None, provenance: Provenance::Generic,
                    p_ac_max_w: c.p_rated_w * copies, efficiency_curve: c.efficiency_curve.clone(),
                    standby_w: c.standby_w * copies, battery_v_min: None, battery_v_max: None,
                    battery_i_max: None, datasheet_ref: String::new(),
                };
                (Cow::Owned(converter), c.p_charge_max_w * copies, c.p_discharge_max_w * copies)
            }
            Coupling::AcCoupled { battery_inverter_id } => {
                let c = p.library.battery_inverter(battery_inverter_id).ok_or("battery inverter missing")?;
                (Cow::Borrowed(c), c.p_ac_max_w, c.p_ac_max_w)
            }
            Coupling::DcHybrid { .. } => return Err("Multiple battery groups require AC coupling.".into()),
        };
        if !converter.p_ac_max_w.is_finite() || converter.p_ac_max_w <= 0.0 || st.units == 0 {
            return Err("Each battery group needs a positive converter rating and at least one unit.".into());
        }
        Ok(Self {
            state: Batt::new(st, spec), converter, charge_w, discharge_w,
            meter: BatteryBankFlow { battery_id: st.battery_id.clone(), units: st.units,
                charge_wh: Vec::with_capacity(hours), charge_grid_wh: Vec::with_capacity(hours),
                discharge_wh: Vec::with_capacity(hours), soc_pct: Vec::with_capacity(hours) },
        })
    }

    fn limits(&self, lh: u32, outage: bool) -> BankHour<'a> {
        let bt = &self.state;
        let current_limit = self.converter.battery_i_max.map_or(f64::INFINITY, |amps| amps * bt.v_pack);
        let (timed_charge, timed_discharge) = bt.timed(lh, outage);
        let mut charge = TransferBudget { terminal: self.charge_w, cells: (bt.ch_limit() * bt.eta_cell).min(current_limit) };
        let mut discharge = TransferBudget { terminal: self.discharge_w, cells: (bt.dis_limit() / bt.eta_cell).min(current_limit) };
        if let Some(t) = timed_charge {
            charge.terminal = charge.terminal.min(t.power_kw * 1000.0);
            charge.cells = charge.cells.min(((t.target_soc_pct / 100.0 - bt.soc()) * bt.e_cap).max(0.0));
        }
        if let Some(t) = timed_discharge {
            discharge.terminal = discharge.terminal.min(t.power_kw * 1000.0);
            discharge.cells = discharge.cells.min(((bt.soc() - t.target_soc_pct / 100.0) * bt.e_cap).max(0.0));
        }
        BankHour { charge, discharge, timed_charge, timed_discharge,
            charge_wh: 0.0, grid_charge_wh: 0.0, discharge_wh: 0.0, loss_wh: 0.0 }
    }

    fn efficiency(&self, power_w: f64) -> f64 {
        let converter_eta = if self.state.st.model == BatteryModel::Detailed {
            self.converter.efficiency(power_w).max(0.5)
        } else {
            self.converter.efficiency(self.converter.p_ac_max_w * 0.5)
        };
        converter_eta * self.state.eta_at(power_w)
    }

    fn charge(&mut self, step: &mut BankHour<'_>, available: f64, reserve_only: bool) -> f64 {
        let terminal = available.min(step.charge.terminal);
        let eta = self.efficiency(terminal);
        if terminal <= 0.0 || eta <= 0.0 { return 0.0; }
        let requested = (terminal * eta).min(step.charge.cells);
        let stored = if reserve_only {
            self.state.acc.charge_one(requested, EnergyUse::Backup)
        } else {
            self.state.acc.charge(requested, &self.state.st.charge_priority)
        };
        let drawn = stored / eta;
        step.charge.used(drawn, stored);
        step.charge_wh += drawn;
        step.loss_wh += drawn - stored;
        drawn
    }

    fn discharge(&mut self, step: &mut BankHour<'_>, demand: f64, uses: &[EnergyUse]) -> f64 {
        let terminal = demand.min(step.discharge.terminal);
        let eta = self.efficiency(terminal);
        if terminal <= 0.0 || eta <= 0.0 { return 0.0; }
        let requested = (terminal / eta).min(step.discharge.cells);
        let stored = self.state.acc.discharge(requested, uses);
        let supplied = stored * eta;
        step.discharge.used(supplied, stored);
        step.discharge_wh += supplied;
        step.loss_wh += stored - supplied;
        supplied
    }

    fn finish(&mut self, hour: BankHour<'_>) -> f64 {
        self.state.throughput += hour.discharge_wh;
        let loss = self.state.dissipate(self.state.spec.self_discharge_pct_per_month / 100.0 / 730.0);
        self.meter.charge_wh.push(hour.charge_wh as f32);
        self.meter.charge_grid_wh.push(hour.grid_charge_wh as f32);
        self.meter.discharge_wh.push(hour.discharge_wh as f32);
        self.meter.soc_pct.push((100.0 * self.state.soc()) as f32);
        loss
    }
}

impl<'a> AcFleet<'a> {
    pub(super) fn new(p: &'a Project, hours: usize) -> Result<Option<Self>, String> {
        if p.additional_storage.is_empty() { return Ok(None); }
        if p.storage.is_none() || p.additional_storage.len() > 31 || p.workflow.system.islanded()
            || p.storage_banks().any(|st| st.generator_id.is_some()) {
            return Err("Multiple battery groups require a primary battery and a shared grid-connected AC bus.".into());
        }
        let banks = p.storage_banks().map(|st| {
            let spec = p.library.battery(&st.battery_id).ok_or("battery missing")?;
            Bank::new(p, st, spec, hours)
        }).collect::<Result<Vec<_>, String>>()?;
        let nominal_power_w = banks.iter().map(|bank| bank.converter.p_ac_max_w).sum();
        let capacity_wh = banks.iter().map(|bank| bank.state.e_cap).sum();
        Ok(Some(Self { banks, nominal_power_w, capacity_wh }))
    }

    pub(super) fn standby(&self) -> f64 {
        self.banks.iter().map(|bank| bank.converter.standby_w).sum()
    }

    pub(super) fn stored(&self) -> f64 {
        self.banks.iter().map(|bank| bank.state.acc.total()).sum()
    }

    pub(super) fn soc_pct(&self) -> f64 {
        let total: f64 = self.banks.iter().map(|bank| bank.state.floor + bank.state.acc.total()).sum();
        100.0 * total / self.capacity_wh
    }

    pub(super) fn allocations(&self) -> [f64; 3] {
        self.banks.iter().fold([0.0; 3], |sum, bank| std::array::from_fn(|i| sum[i] + bank.state.acc.e[i]))
    }

    pub(super) fn capacity_metrics(&self) -> (f64, f64) {
        let usable: f64 = self.banks.iter().flat_map(|bank| bank.state.acc.cap).sum();
        let throughput: f64 = self.banks.iter().map(|bank| bank.state.throughput).sum();
        let available: f64 = self.banks.iter().map(|bank| bank.state.e_cap * bank.state.remaining_capacity_pct()).sum();
        (if usable > 0.0 { throughput / usable } else { 0.0 }, available / self.capacity_wh)
    }

    pub(super) fn into_flows(self) -> Vec<BatteryBankFlow> {
        self.banks.into_iter().map(|bank| bank.meter).collect()
    }

    pub(super) fn step(&mut self, net: f64, local_hour: u32, outage: bool, full_feed_in: bool, export_limit: Option<f64>) -> FleetHour {
        let mut result = FleetHour { balance_wh: net, ..Default::default() };
        let mut hours: Vec<_> = self.banks.iter().map(|bank| bank.limits(local_hour, outage)).collect();
        for (bank, hour) in self.banks.iter_mut().zip(&mut hours) {
            // Each controller receives a share of the same pre-storage bus measurement.
            let share = net * (bank.converter.p_ac_max_w / self.nominal_power_w);
            if share > 0.0 && hour.timed_discharge.is_none() {
                let charged = bank.charge(hour, share, false);
                result.balance_wh -= charged;
                result.pv_charge_wh += charged;
            } else if share < 0.0 && (!full_feed_in || outage) && hour.timed_charge.is_none() && hour.timed_discharge.is_none() {
                let discharged = bank.discharge(hour, -share, if outage { &OUTAGE } else { &NORMAL });
                result.balance_wh += discharged;
                result.discharge_wh += discharged;
            }
        }
        if !outage && result.pv_charge_wh <= 1e-9 {
            for (bank, hour) in self.banks.iter_mut().zip(&mut hours) {
                let before = result.balance_wh;
                let headroom = export_limit.map_or(f64::INFINITY, |limit| (limit - before).max(0.0));
                let (request, accounts) = if hour.timed_discharge.is_some() {
                    (headroom, NORMAL.as_slice())
                } else if hour.timed_charge.is_some() {
                    continue;
                } else {
                    let request = match bank.state.st.grid_service {
                        GridService::PeakShaving { import_limit_kw } => (-before - import_limit_kw * 1000.0).max(0.0),
                        GridService::ExportWindow { start_hour, end_hour, power_kw }
                            if window(local_hour % 24, start_hour, end_hour) => headroom.min(power_kw * 1000.0),
                        _ => continue,
                    };
                    (request, SERVICES.as_slice())
                };
                let discharged = bank.discharge(hour, request, accounts);
                result.discharge_wh += discharged;
                result.balance_wh += discharged;
                result.export_wh += result.balance_wh.max(0.0) - before.max(0.0);
            }
        }
        // A charge request may use residual PV before importing. Suppress opposing
        // directions on the shared bus to avoid circulation between battery groups.
        if !outage && result.discharge_wh <= 1e-9 {
            for (bank, hour) in self.banks.iter_mut().zip(&mut hours) {
                let timed = hour.timed_charge.is_some();
                if !timed && (!bank.state.st.grid_charge_backup || hour.timed_discharge.is_some()) { continue; }
                let reserve = bank.state.acc.cap[0];
                if !timed {
                    if reserve > 0.0 && bank.state.acc.e[0] < 0.9 * reserve { bank.state.backup_recharging = true; }
                    if !bank.state.backup_recharging { continue; }
                }
                let input = bank.charge(hour, f64::INFINITY, !timed);
                let pv = input.min(result.balance_wh.max(0.0));
                hour.grid_charge_wh = input - pv;
                result.pv_charge_wh += pv;
                result.grid_charge_wh += hour.grid_charge_wh;
                result.balance_wh -= input;
                if bank.state.acc.e[0] >= reserve * 0.999 { bank.state.backup_recharging = false; }
            }
        }
        for (bank, hour) in self.banks.iter_mut().zip(hours) {
            result.loss_wh += hour.loss_wh;
            result.self_discharge_wh += bank.finish(hour);
        }
        result
    }
}

fn window(hour: u32, start: u32, end: u32) -> bool {
    if start <= end { (start..end).contains(&hour) } else { hour >= start || hour < end }
}
