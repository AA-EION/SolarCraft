// SPDX-License-Identifier: GPL-3.0-or-later
//! Recover the explicitly saved tariff model, with provenance only for an exact embedded revision.
use serde_json::Value;
use solarcraft_catalog::tariff::{encode_htnt, zone_schedule};
use solarcraft_model::{plant::{CatalogRef, CatalogTierRate, ExportCompensation, FeedIn}, Project};

fn reference(data: &Value, db: &Value, key: &str, family: &str, sha: &str) -> Option<CatalogRef> {
    let id = data["ID"].as_str().filter(|s| !s.is_empty())?;
    let version = data["Version"].as_i64().filter(|v| *v >= 0)?;
    db[key].as_array()?.iter().find(|r| r["Id"].as_str().or_else(|| r["ID"].as_str()).is_some_and(|s| s.eq_ignore_ascii_case(id)) && r["Version"].as_i64() == Some(version))?;
    Some(CatalogRef { family: family.into(), id: id.into(), version, source_sha256: sha.into(), dataset: key.into() })
}
fn number(v: &Value) -> Option<f64> { v.as_f64().filter(|n| n.is_finite() && *n >= 0.0) }

pub(super) fn apply(p: &mut Project, economy: Option<&Value>, db: &Value, sha: &str, kwp: f64, warnings: &mut Vec<String>) {
    let Some(e) = economy else { return; };
    let Some(mode) = e.pointer("/BilanzierungsModel/Data/Einspeisekonzept").and_then(Value::as_i64) else {
        warnings.push("The archived tariff selection is missing; tariff values need review.".into()); return;
    };
    match mode {
        0 => p.grid.feed_in = FeedIn::Full,
        1 | 2 => p.grid.feed_in = FeedIn::Surplus,
        3 => return,
        _ => {
            warnings.push("The archived grid settlement mode is unknown; review tariff values.".into());
            return;
        }
    }
    let demand = if mode == 2 { e.pointer("/NetMeteringTariffModel/Data") } else {
        let list = e.pointer("/DemandTariffList/DemandTariffModels").and_then(Value::as_array);
        if list.is_some_and(|v| v.len() > 1) { warnings.push("Multiple consumer tariffs cannot be represented by one SolarCraft tariff; review grid prices.".into()); None }
        else { list.and_then(|v| v.first()).and_then(|v| v.get("Data")) }
    };
    if let Some(d) = demand {
        let key = if mode == 2 { "NetMeteringTariffs" } else { "DemandTariffs" };
        let cref = reference(d, db, key, "tariff", sha);
        if cref.is_none() { warnings.push("The selected grid tariff's exact embedded catalog revision was not found; saved tariff values have no catalog reference.".into()); }
        p.tariff.catalog_ref = cref;
        if let Some(name) = d["Name"].as_str() { p.tariff.name = name.to_owned(); }
        if let Some(base) = number(&d["BaseTariff"]) { p.tariff.fixed_per_month = base; }
        if number(&d["DemandTariff"]).is_some_and(|x| x != 0.0) { warnings.push("The archived demand charge requires review; its billing period is not imported.".into()); }
        if let Some(htnt) = d.get("HTNT").filter(|v| v.is_object()) {
            let ranges = htnt["EnergyTariffRanges"].as_array();
            let valid = ranges.is_some_and(|groups| !groups.is_empty() && groups.iter().all(|g| {
                g["RangeValues"].as_array().is_some_and(|v| v.len() == 1 && number(&v[0]["Value"]).is_some() && number(&v[0]["Key"]) == Some(0.0))
            }));
            if valid && htnt["TimePlusXMinutes"].as_i64().unwrap_or(0) == 0 {
                let raw = encode_htnt(htnt);
                match zone_schedule(htnt, &raw) {
                    Ok(z) if z.rows.iter().flatten().all(|v| *v > 0) => { p.tariff.import_price = z.zone_prices[0]; p.tariff.zones = Some(z); }
                    _ => warnings.push("The archived grid tariff schedule is invalid and was not imported.".into()),
                }
            } else { warnings.push("The archived grid tariff has tiered prices or a minute offset that SolarCraft cannot reproduce; review grid prices.".into()); }
        } else { warnings.push("The archived grid tariff has no usable price schedule; review grid prices.".into()); }
        if number(&d["MinimumPrice"]).is_some_and(|v| v != 0.0) || number(&d["ThresholdValue_1"]).is_some_and(|v| v != 0.0) || d["SaldoRollover"].as_bool() == Some(true) {
            warnings.push("The archived minimum, threshold or rollover rule is not represented in SolarCraft billing.".into());
        }
    }
    // Net metering uses a project-selected balancing period and may roll credits forward;
    // SolarCraft's monthly model is not equivalent, so do not silently substitute it.
    p.tariff.export = ExportCompensation::None;
    if mode == 2 {
        warnings.push("The archived net-metering settlement is not imported: SolarCraft's monthly settlement cannot reproduce its saved balancing rules.".into());
    } else {
        let list = e.pointer("/ProductionTariffList/ProductionTariffModels").and_then(Value::as_array);
        if let Some([model]) = list.map(Vec::as_slice) {
            let d = &model["Data"];
            let cref = reference(d, db, "ProductionTariffs", "tariff", sha);
            if cref.is_none() { warnings.push("The selected production tariff's exact embedded catalog revision was not found; saved tariff values have no catalog reference.".into()); }
            let c = &d["FeedInTariff"];
            let tiers = c["RangeValues"].as_array().and_then(|a| a.iter().map(|v| Some([number(&v["Key"])?, number(&v["Range"])?, number(&v["Value"])?])).collect::<Option<Vec<_>>>());
            if c["TariffUnit"].as_str() == Some("kWh") && c["ZoningUnit"].as_str().is_none_or(|s| s == "kWp" || s.is_empty()) && c["IsExpense"].as_bool() != Some(true) {
                if let Some(tiers) = tiers.filter(|v| !v.is_empty()) {
                    let zoning = d["Zoning"].as_bool().unwrap_or(false);
                    if let Some(rate) = CatalogTierRate::rate(&tiers, kwp, zoning) {
                        p.tariff.export = ExportCompensation::Fixed { price: rate };
                        p.tariff.export_catalog_ref = cref;
                        p.tariff.catalog_export_rate = Some(CatalogTierRate { tiers, zoning, applied_rate: rate, unit_scale: 1.0 });
                    }
                }
            } else { warnings.push("The archived production tariff unit or expense rule is not supported.".into()); }
            if ["GenerationTariff", "ConsumptionTariff"].iter().any(|k| d[*k]["RangeValues"].as_array().is_some_and(|a| !a.is_empty())) { warnings.push("The archived production or self-consumption compensation is not represented.".into()); }
            if number(&d["PayoutDuration"]).is_some_and(|v| v > 0.0) { warnings.push("The archived feed-in payout duration is not applied to SolarCraft cash flow.".into()); }
        } else if list.is_some_and(|v| !v.is_empty()) { warnings.push("Multiple production tariffs cannot be represented by one SolarCraft tariff.".into()); }
    }
    warnings.push("Imported tariff amounts use the saved project values; currency, taxes and unsupported settlement rules require review.".into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn saved_values_and_exact_revision() {
        let mut p = Project::new("x", "x", 0.0, 0.0);
        let h = json!({"WochenTage": ["a","a","a","a","a","a","a"], "TarriffZoneDefList": [vec![1;288]], "TimePlusXMinutes":0,
            "EnergyTariffRanges":[{"Tarifname":"flat","RangeValues":[{"Key":0,"Value":0.24}]}]});
        let e = json!({"BilanzierungsModel":{"Data":{"Einspeisekonzept":1}}, "DemandTariffList":{"DemandTariffModels":[{"Data":{"ID":"d", "Version":2, "Name":"saved", "BaseTariff":7, "HTNT":h}}]},
            "ProductionTariffList":{"ProductionTariffModels":[{"Data":{"ID":"p", "Version":3, "Zoning":true, "FeedInTariff":{"TariffUnit":"kWh","ZoningUnit":"kWp","RangeValues":[{"Key":0,"Range":100,"Value":0.1},{"Key":10,"Range":100,"Value":0.05}]}}}]}});
        let db = json!({"DemandTariffs":[{"Id":"d","Version":2}],"ProductionTariffs":[{"Id":"p","Version":3}]});
        let mut w=vec![]; apply(&mut p, Some(&e), &db, "hash", 20.0, &mut w);
        assert_eq!(p.tariff.fixed_per_month, 7.0);
        assert_eq!(p.tariff.zones.as_ref().unwrap().price(0,0,0), Some(0.24));
        assert_eq!(p.tariff.catalog_ref.as_ref().unwrap().version, 2);
        assert_eq!(p.tariff.export, ExportCompensation::Fixed { price: 0.075 });
        assert_eq!(p.tariff.for_run(1000.0, 10.0).export, ExportCompensation::Fixed { price: 0.1 });
    }
    #[test]
    fn net_metering_is_not_silently_replaced_by_monthly_settlement() {
        let mut p = Project::new("x", "x", 0.0, 0.0);
        let e = json!({"BilanzierungsModel":{"Data":{"Einspeisekonzept":2}},"NetMeteringTariffModel":{"Data":{"ID":"missing","Version":1,"SaldoRollover":true}}});
        let mut w=vec![]; apply(&mut p, Some(&e), &json!({}), "hash", 1.0, &mut w);
        assert_eq!(p.tariff.export, ExportCompensation::None);
        assert!(p.tariff.catalog_ref.is_none());
        assert!(w.iter().any(|s| s.contains("net-metering settlement")));
    }
}
