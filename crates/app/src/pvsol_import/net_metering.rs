// SPDX-License-Identifier: GPL-3.0-or-later
//! Restore saved net-metering terms and tariff-zone energy bands.
use super::{children, number, path_node};
use roxmltree::Node;
use serde_json::Value;
use solarcraft_catalog::tariff::{encode_htnt, zone_schedule};
use solarcraft_model::{
    plant::{CatalogTierRate, DegradationArray, ExportCompensation, MeteringDegradation, NetMeteringPeriod, NetMeteringTerms, ZoneSchedule},
    Project,
};

fn nonnegative(value: &Value) -> Option<f64> {
    value.as_f64().filter(|x| x.is_finite() && *x >= 0.0)
}

fn prices(demand: &Value) -> Option<(ZoneSchedule, Vec<CatalogTierRate>)> {
    let htnt = demand.get("HTNT")?;
    if htnt["TimePlusXMinutes"].as_i64().unwrap_or_default() != 0 {
        return None;
    }
    let mut zones = zone_schedule(htnt, &encode_htnt(htnt)).ok()?;
    if zones.rows.iter().flatten().any(|&code| code == 0) {
        return None;
    }
    let zoning = demand["Zoning"].as_bool().unwrap_or_default();
    let mut rates = Vec::new();
    for (zone, group) in htnt["EnergyTariffRanges"].as_array()?.iter().enumerate() {
        let mut tiers = Vec::new();
        for v in group["RangeValues"].as_array()? {
            tiers.push([nonnegative(&v["Key"])?, nonnegative(&v["Range"])?, nonnegative(&v["Value"])?]);
        }
        tiers.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let first = tiers.first()?;
        if first[0] != 0.0 || tiers.windows(2).any(|r| r[0][0] == r[1][0]) {
            return None;
        }
        let applied_rate = first[2];
        zones.zone_prices[zone] = applied_rate;
        rates.push(CatalogTierRate { tiers, zoning, applied_rate, unit_scale: 1.0 });
    }
    (!rates.is_empty()).then_some((zones, rates))
}

fn degradation(p: &Project, root: Node<'_, '_>) -> Option<MeteringDegradation> {
    let fields: Vec<_> = children(path_node(root, &["PVModule"])?, "Modulfelder").collect();
    if fields.len() != p.arrays.len() { return None; }
    let mut arrays = Vec::new();
    for (array, field) in p.arrays.iter().zip(fields) {
        let kwp = array.target_kwp.value;
        if kwp <= 0.0 { continue; }
        let losses = path_node(field, &["Verluste"])?;
        if number::<u32>(losses, &["ModuldegradationsArt"])? != 0 { return None; }
        let years = children(losses, "DegradationFurtherPointsJahre")
            .map(|n| n.text()?.trim().parse::<u32>().ok()).collect::<Option<Vec<_>>>()?;
        let outputs = children(losses, "DegradationFurtherPointsVerluste")
            .map(|n| n.text()?.trim().parse::<f64>().ok()).collect::<Option<Vec<_>>>()?;
        if years.is_empty() || years.len() != outputs.len() { return None; }
        let mut points: Vec<_> = years.into_iter().zip(outputs.into_iter().map(|v| v / 100.0)).collect();
        points.push((0, 1.0));
        points.sort_by_key(|p| p.0);
        if points.windows(2).any(|pair| pair[0].0 == pair[1].0)
            || points.iter().any(|p| !(0.0..=1.0).contains(&p.1)) {
            return None;
        }
        arrays.push(DegradationArray { kwp, points });
    }
    (!arrays.is_empty()).then_some(MeteringDegradation {
        reference_annual_pct: p.economics.degradation_pct_per_year,
        arrays,
    })
}

pub(super) fn apply(p: &mut Project, economy: &Value, demand: &Value, root: Node<'_, '_>, warnings: &mut Vec<String>) {
    if let Some(name) = demand["Name"].as_str().filter(|s| !s.trim().is_empty()) {
        p.tariff.name = name.to_owned();
    }
    let period = match demand["PeriodicDuration"].as_str() {
        Some("Jaehrlich") => NetMeteringPeriod::Annual,
        Some("Monatlichneu") => NetMeteringPeriod::Monthly,
        _ => {
            warnings.push("The archived net-metering settlement period is missing or unsupported.".into());
            return;
        }
    };
    if ["MinimumPrice", "DemandTariff", "ThresholdValue_1"].iter().any(|&key| {
        !demand[key].is_null() && nonnegative(&demand[key]) != Some(0.0)
    }) || demand["ThresholdType_1"].as_str().is_some_and(|kind| kind != "none") {
        warnings.push("The archived net-metering settlement requires a minimum, demand charge, or credit cap; review these billing terms.".into());
        return;
    }
    let Some((zones, zone_rates)) = prices(demand) else {
        warnings.push("The archived net-metering settlement has missing or unsupported zone rates.".into());
        return;
    };
    p.tariff.import_price = zones.zone_prices[0];
    p.tariff.zones = Some(zones);
    p.tariff.fixed_per_month = nonnegative(&demand["BaseTariff"]).unwrap_or_default();
    if let Some(escalation) = economy["StrompreisaenderungsFaktor"].as_f64().filter(|x| (-1.0..=10.0).contains(x)) {
        p.tariff.import_escalation_pct = escalation * 100.0;
    }
    p.tariff.export = ExportCompensation::NetMetering { surplus_price: 0.0 };
    p.tariff.net_metering = Some(NetMeteringTerms {
        period,
        rollover: demand["SaldoRollover"].as_bool().unwrap_or_default(),
        zone_rates,
        savings_degradation: degradation(p, root),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn contract_period_overrides_the_legacy_balance_display_period() {
        let e = json!({"BilanzierungsModel":{"Data":{"Bilanzzeitraum":1}},"StrompreisaenderungsFaktor":0.03});
        let d = json!({
            "PeriodicDuration": "Jaehrlich",
            "SaldoRollover": true,
            "ThresholdType_1": "none",
            "HTNT": {
                "WochenTage": ["a", "a", "a", "a", "a", "a", "a"],
                "TarriffZoneDefList": [vec![1; 288]],
                "EnergyTariffRanges": [{"Tarifname":"energy","RangeValues":[
                    {"Key": 0, "Range": 100, "Value": 0.1637},
                    {"Key": 10, "Range": 100, "Value": 0.32}
                ]}]
            },
            "Zoning": true
        });
        let mut p = Project::new("a", "z", 0.0, 0.0);
        let mut warnings = vec![];
        let xml = roxmltree::Document::parse("<Project/>").unwrap();
        apply(&mut p, &e, &d, xml.root_element(), &mut warnings);
        assert!(warnings.is_empty());
        assert_eq!(p.tariff.net_metering.as_ref().unwrap().period, NetMeteringPeriod::Annual);
        assert_eq!(p.tariff.import_price, 0.1637);
        assert_eq!(p.tariff.import_escalation_pct, 3.0);
        assert_eq!(p.tariff.net_metering.unwrap().zone_rates[0].tiers.len(), 2);
    }
}
