// SPDX-License-Identifier: GPL-3.0-or-later
//! Electricity bills and cash flow.
//!
//! Year 1 bills are computed hour by hour from the simulated grid import and
//! export on the local reference calendar, once with the plant and once for
//! the same consumption without it. Later years scale the plant's bill
//! reduction and export revenue by PV degradation and the tariff escalation
//! rates (the self-consumption share is assumed constant over the years).
//!
//! Cash flow: year 0 carries the net investment (minus any loan payout);
//! years 1..N carry savings and export revenue minus operation and
//! maintenance, replacements and loan annuities. The final year adds the
//! residual value of replacements by remaining life. Net present value
//! discounts year y by (1 + r)^y; LCOE divides discounted lifetime costs by
//! discounted AC energy.
use crate::calendar::local_time;
use serde::{Deserialize, Serialize};
use solarcraft_model::plant::{Economics, ExportCompensation, Tariff};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Bill { pub energy: f64, pub demand: f64, pub fixed: f64, pub export_credit: f64, pub total: f64, pub import_kwh: f64, pub export_kwh: f64 }

pub fn import_price(t: &Tariff, hoy: u32) -> f64 {
    let lt = local_time(hoy);
    let weekend = lt.weekday >= 5;
    for p in &t.tou {
        if !p.months.is_empty() && !p.months.contains(&lt.month) { continue; }
        if (weekend && !p.weekends) || (!weekend && !p.weekdays) { continue; }
        let inside = if p.start_hour <= p.end_hour { lt.hour >= p.start_hour && lt.hour < p.end_hour } else { lt.hour >= p.start_hour || lt.hour < p.end_hour };
        if inside { return p.price; }
    }
    t.import_price
}

/// Annual bill from hourly import/export (Wh) indexed with their local hours.
pub fn bill(t: &Tariff, import_wh: &[f32], export_wh: &[f32], local_hour: &[u32]) -> Bill {
    let mut b = Bill { fixed: t.fixed_per_month * 12.0, ..Default::default() };
    let (mut imp_m, mut exp_m, mut cost_m, mut peak_m) = ([0.0f64; 12], [0.0f64; 12], [0.0f64; 12], [0.0f64; 12]);
    for (k, &h) in local_hour.iter().enumerate() {
        let m = local_time(h).month as usize - 1;
        let i = import_wh.get(k).copied().unwrap_or(0.0) as f64 / 1000.0;
        let e = export_wh.get(k).copied().unwrap_or(0.0) as f64 / 1000.0;
        imp_m[m] += i; exp_m[m] += e; cost_m[m] += i * import_price(t, h); peak_m[m] = peak_m[m].max(i);
    }
    for m in 0..12 {
        b.energy += cost_m[m];
        b.demand += peak_m[m] * t.demand_charge_per_kw_month;
        b.import_kwh += imp_m[m]; b.export_kwh += exp_m[m];
        b.export_credit += match &t.export {
            ExportCompensation::None => 0.0,
            ExportCompensation::Fixed { price } => exp_m[m] * price,
            ExportCompensation::NetMetering { surplus_price } => {
                let avg = if imp_m[m] > 0.0 { cost_m[m] / imp_m[m] } else { t.import_price };
                let offset = exp_m[m].min(imp_m[m]);
                offset * avg + (exp_m[m] - offset) * surplus_price
            }
        };
    }
    b.total = b.energy + b.demand + b.fixed - b.export_credit;
    b
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Year {
    pub year: u32,
    pub savings: f64,
    pub export_revenue: f64,
    pub om: f64,
    pub replacements: f64,
    pub loan: f64,
    pub cash_flow: f64,
    pub cumulative: f64,
    pub energy_kwh: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Finance {
    pub currency: String,
    pub investment: f64,
    pub bill_without: Bill,
    pub bill_with: Bill,
    pub years: Vec<Year>,
    pub npv: f64,
    pub irr_pct: Option<f64>,
    pub payback_years: Option<f64>,
    pub lcoe_per_kwh: f64,
    pub notes: Vec<String>,
}

pub struct Plant { pub dc_kwp: f64, pub ac_kw: f64, pub battery_kwh: f64, pub pv_ac_kwh: f64 }

pub fn cash_flow(t: &Tariff, ec: &Economics, plant: &Plant, without: Bill, with: Bill) -> Finance {
    let mut notes = vec![];
    let investment = ec.cost_fixed + ec.cost_per_kwp * plant.dc_kwp + ec.cost_per_battery_kwh * plant.battery_kwh;
    let inv_repl = if ec.inverter_replacement_cost > 0.0 { ec.inverter_replacement_cost } else { 120.0 * plant.ac_kw };
    let bat_repl = if plant.battery_kwh <= 0.0 { 0.0 } else if ec.battery_replacement_cost > 0.0 { ec.battery_replacement_cost } else { 0.6 * ec.cost_per_battery_kwh * plant.battery_kwh };
    if ec.inverter_replacement_cost <= 0.0 { notes.push(format!("Inverter replacement cost estimated at 120 per kW AC ({inv_repl:.0}).")); }
    let r = ec.discount_rate_pct / 100.0;
    let n = ec.horizon_years;
    let annuity = ec.loan.as_ref().map(|l| { let i = l.rate_pct / 100.0; if i.abs() < 1e-12 { l.amount / l.years.max(1) as f64 } else { l.amount * i / (1.0 - (1.0 + i).powi(-(l.years.max(1) as i32))) } });
    let base_import = without.energy + without.demand + without.fixed - (with.energy + with.demand + with.fixed);
    let mut years = vec![];
    let cf0 = -(investment - ec.subsidy) + ec.loan.as_ref().map_or(0.0, |l| l.amount);
    years.push(Year { year: 0, cash_flow: cf0, cumulative: cf0, ..Default::default() });
    let (mut pv_cost, mut pv_energy) = (investment - ec.subsidy, 0.0);
    let mut cum = cf0;
    for y in 1..=n {
        let d = (1.0 - ec.degradation_pct_per_year / 100.0).powi(y as i32 - 1);
        let esc_i = (1.0 + t.import_escalation_pct / 100.0).powi(y as i32 - 1);
        let esc_e = (1.0 + t.export_escalation_pct / 100.0).powi(y as i32 - 1);
        let savings = base_import * d * esc_i;
        let export_revenue = with.export_credit * d * esc_e;
        let om = investment * ec.om_pct_of_investment / 100.0 * (1.0 + ec.om_escalation_pct / 100.0).powi(y as i32 - 1);
        let mut repl = 0.0;
        if ec.inverter_replacement_year > 0 && y % ec.inverter_replacement_year == 0 && y < n { repl += inv_repl; }
        if ec.battery_replacement_year > 0 && y % ec.battery_replacement_year == 0 && y < n { repl += bat_repl; }
        // Residual value of the latest replacement at the end of the period (remaining-life share).
        if y == n {
            for (life, cost) in [(ec.inverter_replacement_year, inv_repl), (ec.battery_replacement_year, bat_repl)] {
                if life > 0 && life < n { let last = (n - 1) / life * life; let rem = (last + life).saturating_sub(n) as f64 / life as f64; repl -= cost * rem; }
            }
        }
        let loan = match (&ec.loan, annuity) { (Some(l), Some(a)) if y <= l.years => a, _ => 0.0 };
        let cf = savings + export_revenue - om - repl - loan;
        cum += cf;
        let disc = (1.0 + r).powi(y as i32);
        let energy = plant.pv_ac_kwh * d;
        pv_cost += (om + repl) / disc;
        pv_energy += energy / disc;
        years.push(Year { year: y, savings, export_revenue, om, replacements: repl, loan, cash_flow: cf, cumulative: cum, energy_kwh: energy });
    }
    let npv: f64 = years.iter().map(|y| y.cash_flow / (1.0 + r).powi(y.year as i32)).sum();
    let irr_pct = irr(&years.iter().map(|y| y.cash_flow).collect::<Vec<_>>()).map(|x| x * 100.0);
    let payback_years = years.windows(2).find(|w| w[0].cumulative < 0.0 && w[1].cumulative >= 0.0)
        .map(|w| (w[1].year - 1) as f64 + (-w[0].cumulative) / w[1].cash_flow.max(1e-9));
    if ec.loan.is_some() { notes.push("Cash flow is shown for the owner's equity: loan payout in year 0, annuities in later years.".into()); }
    Finance { currency: t.currency.clone(), investment, bill_without: without, bill_with: with, years, npv, irr_pct, payback_years,
              lcoe_per_kwh: if pv_energy > 0.0 { pv_cost / pv_energy } else { 0.0 }, notes }
}

/// Internal rate of return by bisection; None without a sign change.
pub fn irr(cf: &[f64]) -> Option<f64> {
    let npv = |r: f64| cf.iter().enumerate().map(|(y, c)| c / (1.0 + r).powi(y as i32)).sum::<f64>();
    let (mut lo, mut hi) = (-0.99, 1.0);
    let (flo, fhi) = (npv(lo), npv(hi));
    if flo.signum() == fhi.signum() { return None; }
    for _ in 0..200 { let m = 0.5 * (lo + hi); if npv(m).signum() == flo.signum() { lo = m } else { hi = m } }
    Some(0.5 * (lo + hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn irr_of_simple_annuity() {
        // −1000 then 10 × 150 → IRR ≈ 8.144 %.
        let mut cf = vec![-1000.0]; cf.extend(std::iter::repeat_n(150.0, 10));
        assert!((irr(&cf).unwrap() - 0.08144).abs() < 1e-4);
    }
    #[test]
    fn tou_and_net_metering() {
        use solarcraft_model::plant::TouPeriod;
        let mut t = Tariff { import_price: 0.2, export: ExportCompensation::NetMetering { surplus_price: 0.05 }, ..Default::default() };
        t.tou.push(TouPeriod { name: "peak".into(), price: 0.5, start_hour: 18, end_hour: 22, weekdays: true, weekends: false, months: vec![] });
        assert_eq!(import_price(&t, 19), 0.5); // 1 Jan (Wednesday) 19:00
        assert_eq!(import_price(&t, 4 * 24 + 19), 0.2); // Sunday
        let hours: Vec<u32> = (0..8760).collect();
        let imp = vec![100.0f32; 8760];
        let mut exp = vec![0.0f32; 8760]; exp[12] = 5000.0; // 5 kWh in January
        let b = bill(&t, &imp, &exp, &hours);
        assert!(b.export_credit > 5.0 * 0.2 - 1e-9 && b.export_credit < 5.0 * 0.5);
    }
}
