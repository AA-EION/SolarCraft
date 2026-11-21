// SPDX-License-Identifier: GPL-3.0-or-later
use super::{bill, Bill};
use crate::energy::Flows;
use serde::{Deserialize, Serialize};
use solarcraft_model::plant::{Consumer, ExportCompensation, Tariff};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerBill {
    pub consumer: Option<Consumer>,
    pub name: String,
    pub bill_without: Bill,
    pub bill_with: Bill,
}

pub(crate) struct Demand<'a> {
    pub consumer: Consumer,
    pub actual: &'a [f32],
    pub reference: &'a [f32],
}

fn add_bill(total: &mut Bill, meter: &Bill) {
    total.energy += meter.energy;
    total.demand += meter.demand;
    total.fixed += meter.fixed;
    total.export_credit += meter.export_credit;
    total.total += meter.total;
    total.import_kwh += meter.import_kwh;
    total.export_kwh += meter.export_kwh;
}

/// Allocate common-bus grid energy to consumers by concurrent requested load.
/// Any import outside the load ledger, including grid battery charging, stays on the site bill.
pub(crate) fn bills(
    tariff: &Tariff,
    demands: &[Demand<'_>],
    flows: &Flows,
    reference: &[f32],
    hours: &[u32],
) -> (Bill, Bill, Vec<ConsumerBill>) {
    let mut remainder = flows.import.clone();
    let mut reference_remainder = reference.to_vec();
    let mut meters = Vec::new();
    let no_exports = vec![0.0; hours.len()];
    if !matches!(tariff.export, ExportCompensation::NetMetering { .. }) {
        for rate in &tariff.consumer_rates {
            let Some(demand) = demands.iter().find(|d| d.consumer == rate.consumer && !d.actual.is_empty()) else { continue; };
            let mut import = Vec::with_capacity(hours.len());
            let mut prior_import = Vec::with_capacity(hours.len());
            for h in 0..hours.len() {
                let total_load = flows.load[h];
                let share = if total_load > 0.0 { (demand.actual[h] / total_load).clamp(0.0, 1.0) } else { 0.0 };
                let after = (flows.grid_to_load[h] * share).clamp(0.0, remainder[h]);
                let before = demand.reference[h].clamp(0.0, reference_remainder[h]);
                remainder[h] -= after;
                reference_remainder[h] -= before;
                import.push(after);
                prior_import.push(before);
            }
            let contract = Tariff {
                name: rate.name.clone(), currency: tariff.currency.clone(),
                import_price: rate.import_price, zones: rate.zones.clone(),
                fixed_per_month: rate.fixed_per_month,
                demand_charge_per_kw_month: rate.demand_charge_per_kw_month,
                export: ExportCompensation::None, ..Default::default()
            };
            meters.push(ConsumerBill {
                consumer: Some(rate.consumer), name: rate.name.clone(),
                bill_without: bill(&contract, &prior_import, &no_exports, hours),
                bill_with: bill(&contract, &import, &no_exports, hours),
            });
        }
    }
    let mut without = bill(tariff, &reference_remainder, &no_exports, hours);
    let mut with = bill(tariff, &remainder, &flows.export, hours);
    if !meters.is_empty() {
        let site = ConsumerBill {
            consumer: None, name: tariff.name.clone(), bill_without: without.clone(), bill_with: with.clone(),
        };
        for meter in &meters {
            add_bill(&mut without, &meter.bill_without);
            add_bill(&mut with, &meter.bill_with);
        }
        meters.insert(0, site);
    }
    (without, with, meters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solarcraft_model::plant::ConsumerTariff;

    #[test]
    fn consumer_bills_close_to_meter_including_grid_charging_and_export() {
        let tariff = Tariff {
            import_price: 0.3, export: ExportCompensation::Fixed { price: 0.05 },
            consumer_rates: vec![ConsumerTariff {
                consumer: Consumer::ElectricVehicles, name: "EV".into(), import_price: 0.1,
                fixed_per_month: 2.0, demand_charge_per_kw_month: 0.0, zones: None, catalog_ref: None,
            }], ..Default::default()
        };
        let flows = Flows { load: vec![4000.0, 500.0], grid_to_load: vec![2000.0, 0.0],
            import: vec![2500.0, 0.0], export: vec![0.0, 250.0], ..Default::default() };
        let demand = Demand { consumer: Consumer::ElectricVehicles, actual: &[1000.0, 0.0], reference: &[2000.0, 0.0] };
        let (before, after, meters) = bills(&tariff, &[demand], &flows, &[5000.0, 500.0], &[0, 1]);
        assert_eq!(meters.len(), 2);
        assert_eq!(meters[0].bill_with.import_kwh, 2.0);
        assert_eq!(meters[1].bill_with.import_kwh, 0.5);
        assert_eq!(before.import_kwh, 5.5);
        assert_eq!(after.import_kwh, 2.5);
        assert_eq!(after.fixed, 24.0);
        assert!((after.total - 24.6375).abs() < 1e-10);
        assert!((before.total - 25.25).abs() < 1e-10);
    }
}
