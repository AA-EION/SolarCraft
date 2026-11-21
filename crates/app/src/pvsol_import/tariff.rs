// SPDX-License-Identifier: GPL-3.0-or-later
//! Recover the explicitly saved tariff model, with provenance only for an exact embedded revision.
use serde_json::Value;
use roxmltree::Node;
use solarcraft_catalog::tariff::{encode_htnt, zone_schedule};
use solarcraft_model::{plant::{CatalogRef, CatalogTierRate, Consumer, ConsumerTariff, ExportCompensation, FeedIn, Tariff}, Project};

fn reference(data: &Value, db: &Value, key: &str, family: &str, sha: &str) -> Option<CatalogRef> {
    let id = data["ID"].as_str().filter(|s| !s.is_empty())?;
    let version = data["Version"].as_i64().filter(|v| *v >= 0)?;
    db[key].as_array()?.iter().find(|r| r["Id"].as_str().or_else(|| r["ID"].as_str()).is_some_and(|s| s.eq_ignore_ascii_case(id)) && r["Version"].as_i64() == Some(version))?;
    Some(CatalogRef { family: family.into(), id: id.into(), version, source_sha256: sha.into(), dataset: key.into() })
}
fn number(v: &Value) -> Option<f64> { v.as_f64().filter(|n| n.is_finite() && *n >= 0.0) }

fn read_demand(tariff: &mut Tariff, d: &Value, db: &Value, sha: &str, warnings: &mut Vec<String>) -> bool {
    tariff.catalog_ref = reference(d, db, "DemandTariffs", "tariff", sha);
    if tariff.catalog_ref.is_none() {
        warnings.push("The selected grid tariff's exact embedded catalog revision was not found; saved tariff values have no catalog reference.".into());
    }
    if let Some(name) = d["Name"].as_str() { tariff.name = name.to_owned(); }
    if let Some(base) = number(&d["BaseTariff"]) { tariff.fixed_per_month = base; }
    if number(&d["DemandTariff"]).is_some_and(|x| x != 0.0) {
        warnings.push("The archived demand charge requires review; its billing period is not imported.".into());
    }
    if number(&d["MinimumPrice"]).is_some_and(|v| v != 0.0) || number(&d["ThresholdValue_1"]).is_some_and(|v| v != 0.0) {
        warnings.push("The archived minimum or threshold rule is not represented in SolarCraft billing.".into());
    }
    let Some(htnt) = d.get("HTNT").filter(|v| v.is_object()) else {
        warnings.push("The archived grid tariff has no usable price schedule; review grid prices.".into());
        return false;
    };
    let valid = htnt["EnergyTariffRanges"].as_array().is_some_and(|groups| !groups.is_empty() && groups.iter().all(|g|
        g["RangeValues"].as_array().is_some_and(|v| v.len() == 1
            && number(&v[0]["Value"]).is_some() && number(&v[0]["Key"]) == Some(0.0))));
    if !valid || htnt["TimePlusXMinutes"].as_i64().unwrap_or(0) != 0 {
        warnings.push("The archived grid tariff has tiered prices or a minute offset that SolarCraft cannot reproduce; review grid prices.".into());
        return false;
    }
    match zone_schedule(htnt, &encode_htnt(htnt)) {
        Ok(z) if z.rows.iter().flatten().all(|v| *v > 0) => {
            tariff.import_price = z.zone_prices[0];
            tariff.zones = Some(z);
            true
        }
        _ => {
            warnings.push("The archived grid tariff schedule is invalid and was not imported.".into());
            false
        }
    }
}

const HEATING: &str = "8b3dffdd-25df-4199-aed9-384265a1ae20";
const EV: &str = "7dc793d0-5061-4bdf-bca9-f2998e08c99e";
const SITE: &str = "00000000-0000-0000-0000-000000000000";

fn consumer_demand(p: &mut Project, root: Node<'_, '_>, list: &[Value], db: &Value, sha: &str, warnings: &mut Vec<String>) {
    // A single base load can be bound to a saved site consumer ID. Multiple archived
    // base consumers may share one aggregate load ledger in SolarCraft.
    let base_ids: Vec<_> = super::path_node(root, &["Verbrauch"]).into_iter()
        .flat_map(|node| super::children(node, "VerbrauchGruppen"))
        .filter(|&group| super::children(group, "Verbrauchsdefinition").any(|definition|
            definition.children().any(|n| n.is_element() && !n.has_tag_name("Ueberschussverbrauch"))))
        .filter_map(|group| super::value(group, &["ID"]))
        .collect();
    let mut sites = Vec::new();
    let mut secondary = Vec::new();
    for model in list {
        let id = model["ConsumptionId"].as_str().unwrap_or_default().to_ascii_lowercase();
        match id.as_str() {
            HEATING => secondary.push((Consumer::Heating, &model["Data"])),
            EV => secondary.push((Consumer::ElectricVehicles, &model["Data"])),
            SITE => sites.push(&model["Data"]),
            _ if matches!(base_ids.as_slice(), [base] if base.eq_ignore_ascii_case(&id)) => sites.push(&model["Data"]),
            _ => warnings.push("An archived demand tariff belongs to a consumer without a separate native load profile.".into()),
        }
    }
    if let [site] = sites.as_slice() {
        read_demand(&mut p.tariff, site, db, sha, warnings);
    } else {
        warnings.push("The archive has no unique rate for shared site imports; review the default site tariff.".into());
    }
    for consumer in [Consumer::ElectricVehicles, Consumer::Heating] {
        let entries: Vec<_> = secondary.iter().filter_map(|&(kind, data)| (kind == consumer).then_some(data)).collect();
        if let [data] = entries.as_slice() {
            let mut meter = Tariff::default();
            if read_demand(&mut meter, data, db, sha, warnings) {
                p.tariff.consumer_rates.push(ConsumerTariff {
                    consumer, name: meter.name, import_price: meter.import_price,
                    fixed_per_month: meter.fixed_per_month, demand_charge_per_kw_month: meter.demand_charge_per_kw_month,
                    zones: meter.zones, catalog_ref: meter.catalog_ref,
                });
            }
        } else if !entries.is_empty() {
            warnings.push("Multiple archived rates target the same consumer; that meter uses the shared site tariff.".into());
        }
    }
}

pub(super) fn apply(p: &mut Project, root: Node<'_, '_>, economy: Option<&Value>, db: &Value, sha: &str, kwp: f64, warnings: &mut Vec<String>) {
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
    p.tariff.export = ExportCompensation::None;
    if mode == 2 {
        if let Some(d) = e.pointer("/NetMeteringTariffModel/Data") {
            p.tariff.catalog_ref = reference(d, db, "NetMeteringTariffs", "tariff", sha);
            super::net_metering::apply(p, e, d, root, warnings);
        } else {
            warnings.push("The archived net-metering settlement has no saved contract.".into());
        }
    } else {
        if let Some(demand) = e.pointer("/DemandTariffList/DemandTariffModels").and_then(Value::as_array) {
            match demand.as_slice() {
                [single] => { read_demand(&mut p.tariff, &single["Data"], db, sha, warnings); }
                [] => {}
                many => consumer_demand(p, root, many, db, sha, warnings),
            }
        }
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
        let xml = roxmltree::Document::parse("<Project/>").unwrap();
        let mut w=vec![]; apply(&mut p, xml.root_element(), Some(&e), &db, "hash", 20.0, &mut w);
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
        let xml = roxmltree::Document::parse("<Project/>").unwrap();
        let mut w=vec![]; apply(&mut p, xml.root_element(), Some(&e), &json!({}), "hash", 1.0, &mut w);
        assert_eq!(p.tariff.export, ExportCompensation::None);
        assert!(p.tariff.catalog_ref.is_none());
        assert!(w.iter().any(|s| s.contains("net-metering settlement")));
    }

    #[test]
    fn separate_demand_rates_use_archived_consumer_ids() {
        let xml = roxmltree::Document::parse("<Project><Verbrauch><VerbrauchGruppen><ID>home</ID><Verbrauchsdefinition><ImportierteProfile/></Verbrauchsdefinition></VerbrauchGruppen></Verbrauch></Project>").unwrap();
        let saved = |id, price| json!({
            "ConsumptionId":id, "Data":{"Name":id, "HTNT":{
                "WochenTage":["a","a","a","a","a","a","a"], "TarriffZoneDefList":[vec![1;288]],
                "EnergyTariffRanges":[{"RangeValues":[{"Key":0,"Value":price}]}]
            }}
        });
        let economy = json!({
            "BilanzierungsModel":{"Data":{"Einspeisekonzept":1}},
            "DemandTariffList":{"DemandTariffModels":[saved(HEATING,0.15),saved("home",0.27),saved(EV,0.12),saved("other",1.0)]}
        });
        let mut p = Project::new("test", "Meters", 0.0, 0.0);
        let mut warnings = vec![];
        apply(&mut p, xml.root_element(), Some(&economy), &json!({}), "hash", 1.0, &mut warnings);
        assert_eq!(p.tariff.import_price, 0.27);
        assert_eq!(p.tariff.consumer_rates.iter().map(|r| (r.consumer, r.import_price)).collect::<Vec<_>>(),
            vec![(Consumer::ElectricVehicles, 0.12), (Consumer::Heating, 0.15)]);
        assert!(warnings.iter().any(|s| s.contains("without a separate native load profile")));
    }
}
