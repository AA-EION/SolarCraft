// SPDX-License-Identifier: GPL-3.0-or-later
//! Tariff-zone energy credits and billed demand after settlement.
use super::{import_price, Bill};
use crate::calendar::local_time;
use serde::{Deserialize, Serialize};
use solarcraft_model::plant::{NetMeteringPeriod, NetMeteringTerms, Tariff};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MeteringBill {
    pub offset_kwh: f64,
    pub billed_kwh: f64,
    /// Export remaining after offsetting imports, including balances that expired.
    pub unused_export_kwh: f64,
    /// Reduction in the energy charge due to kWh credits, excluding surplus payouts.
    pub energy_credit: f64,
    /// Energy costs in calendar order when the meter has one flat price and no exports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_flat_cost: Option<[f64; 12]>,
}

#[derive(Clone, Copy, Default)]
struct Energy {
    imported: f64,
    exported: f64,
    gross_cost: f64,
}

fn settle(
    result: &mut MeteringBill,
    period: Energy,
    previous_credit: f64,
    rate: impl Fn(f64) -> f64,
) -> (f64, f64) {
    let available = period.exported + previous_credit;
    let offset = period.imported.min(available);
    let billed = period.imported - offset;
    let cost = billed * rate(billed);
    result.offset_kwh += offset;
    result.billed_kwh += billed;
    result.energy_credit += period.imported * rate(period.imported) - cost;
    (cost, available - offset)
}

pub(super) fn bill(
    tariff: &Tariff,
    terms: &NetMeteringTerms,
    surplus_price: f64,
    import_wh: &[f32],
    export_wh: &[f32],
    local_hour: &[u32],
) -> Bill {
    let prices = tariff.zones.as_ref().map_or(&[][..], |zones| zones.zone_prices.as_slice());
    // The last bucket handles any slot using the ordinary tariff instead of a preset zone.
    let mut by_zone = vec![[Energy::default(); 12]; prices.len() + 1];
    let mut peak = [0.0_f64; 12];
    let mut monthly_cost = [0.0; 12];
    let mut bill = Bill { fixed: tariff.fixed_per_month * 12.0, ..Bill::default() };
    for (k, &hoy) in local_hour.iter().enumerate() {
        let lt = local_time(hoy);
        let month = lt.month as usize - 1;
        let zone = tariff.zones.as_ref()
            .and_then(|z| z.zone(lt.weekday, lt.month - 1, lt.hour))
            .and_then(|code| code.checked_sub(1))
            .map(|i| i as usize)
            .filter(|&i| i < prices.len())
            .unwrap_or(prices.len());
        let imported = f64::from(import_wh.get(k).copied().unwrap_or_default()) / 1000.0;
        let exported = f64::from(export_wh.get(k).copied().unwrap_or_default()) / 1000.0;
        let slot = &mut by_zone[zone][month];
        slot.imported += imported;
        slot.exported += exported;
        let cost = imported * import_price(tariff, hoy);
        slot.gross_cost += cost;
        monthly_cost[month] += cost;
        peak[month] = peak[month].max(imported);
        bill.import_kwh += imported;
        bill.export_kwh += exported;
    }
    bill.demand = peak.iter().sum::<f64>() * tariff.demand_charge_per_kw_month;
    let mut metering = MeteringBill::default();
    if bill.export_kwh == 0.0 && prices.len() == 1
        && terms.zone_rates.first().is_some_and(|rate| rate.tiers.len() == 1)
    {
        metering.monthly_flat_cost = Some(monthly_cost);
    }
    for (zone, months) in by_zone.into_iter().enumerate() {
        let price = |period: Energy, quantity| {
            if let Some(&flat) = prices.get(zone) {
                terms.rate(zone, flat, quantity)
            } else if period.imported > 0.0 {
                period.gross_cost / period.imported
            } else {
                tariff.import_price
            }
        };
        match terms.period {
            NetMeteringPeriod::Annual => {
                let year = months.into_iter().fold(Energy::default(), |a, b| Energy {
                    imported: a.imported + b.imported,
                    exported: a.exported + b.exported,
                    gross_cost: a.gross_cost + b.gross_cost,
                });
                let (cost, unused) = settle(&mut metering, year, 0.0, |kwh| price(year, kwh));
                bill.energy += cost;
                metering.unused_export_kwh += unused;
            }
            NetMeteringPeriod::Monthly => {
                let mut credit = 0.0;
                for month in months {
                    let (cost, remainder) = settle(&mut metering, month, credit, |kwh| price(month, kwh));
                    bill.energy += cost;
                    if terms.rollover {
                        credit = remainder;
                    } else {
                        metering.unused_export_kwh += remainder;
                    }
                }
                metering.unused_export_kwh += credit;
            }
        }
    }
    bill.export_credit = metering.unused_export_kwh * surplus_price;
    bill.total = bill.energy + bill.demand + bill.fixed - bill.export_credit;
    bill.metering = Some(metering);
    bill
}

#[cfg(test)]
mod tests {
    use super::*;
    use solarcraft_model::plant::{CatalogTierRate, ExportCompensation, ZoneSchedule};

    #[test]
    fn annual_metering_is_separate_by_zone_and_tiers_apply_to_net_demand() {
        let mut row = vec![1; 288];
        row[1] = 2;
        let tariff = Tariff {
            export: ExportCompensation::NetMetering { surplus_price: 0.0 },
            net_metering: Some(NetMeteringTerms {
                period: NetMeteringPeriod::Annual,
                rollover: true,
                zone_rates: vec![CatalogTierRate {
                    tiers: vec![[0.0, 100.0, 0.20], [5.0, 100.0, 0.50]],
                    zoning: true,
                    applied_rate: 0.20,
                    unit_scale: 1.0,
                }],
                savings_degradation: None,
            }),
            zones: Some(ZoneSchedule {
                weekday_rows: [0; 7],
                rows: vec![row],
                zone_prices: vec![0.20, 0.70],
                zone_names: vec![],
                time_plus_minutes: None,
                raw_encoded: String::new(),
            }),
            ..Tariff::default()
        };
        // February export offsets January import within zone 1. Zone 2 has only exports.
        let bill = super::super::bill(&tariff, &[10_000.0, 0.0, 0.0], &[0.0, 6000.0, 3000.0], &[0, 1, 744]);
        assert!((bill.energy - 2.0).abs() < 1e-9);
        assert_eq!(bill.export_credit, 0.0);
        let settlement = bill.metering.unwrap();
        assert_eq!(settlement.offset_kwh, 3.0);
        assert_eq!(settlement.billed_kwh, 7.0);
        assert_eq!(settlement.unused_export_kwh, 6.0);
        assert!((settlement.energy_credit - 1.5).abs() < 1e-9);
    }

    #[test]
    fn monthly_credit_carries_forward_only() {
        let mut tariff = Tariff {
            import_price: 0.2,
            export: ExportCompensation::NetMetering { surplus_price: 0.0 },
            net_metering: Some(NetMeteringTerms {
                period: NetMeteringPeriod::Monthly,
                rollover: true,
                zone_rates: vec![],
                savings_degradation: None,
            }),
            ..Tariff::default()
        };
        let hours = [0, 744, 1416];
        let imported = [7000.0, 0.0, 3000.0];
        let exported = [0.0, 8000.0, 0.0];
        let rolling = super::super::bill(&tariff, &imported, &exported, &hours);
        assert!((rolling.energy - 1.4).abs() < 1e-9);
        assert_eq!(rolling.metering.unwrap().unused_export_kwh, 5.0);
        tariff.net_metering.as_mut().unwrap().rollover = false;
        let monthly = super::super::bill(&tariff, &imported, &exported, &hours);
        assert!((monthly.energy - 2.0).abs() < 1e-9);
        assert_eq!(monthly.metering.unwrap().unused_export_kwh, 8.0);
    }
}
