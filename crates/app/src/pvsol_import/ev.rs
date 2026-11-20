// SPDX-License-Identifier: GPL-3.0-or-later
//! Preserve EV source configuration without guessing the vendor dispatch algorithm.
use super::{children, number, value};
use roxmltree::Node;
use serde_json::Value;
use solarcraft_model::{demand::ImportedEvGroup, Project};

fn boolean(s: &str) -> Option<bool> {
    if s.eq_ignore_ascii_case("true") { Some(true) }
    else if s.eq_ignore_ascii_case("false") { Some(false) }
    else { None }
}

pub(super) fn apply(project: &mut Project, root: Node<'_, '_>, db: &Value, warnings: &mut Vec<String>) {
    let Some(section) = root.descendants().find(|n| n.has_tag_name("ElektroAuto")) else { return; };
    let mut unmatched = false;
    let groups = children(section, "Car").map(|car| {
        let id = value(car, &["ECarAndStationDBId"]).map(str::to_owned);
        let revision: Option<u32> = number(car, &["ECarAndStationDBVersion", "Value"]);
        let mut matches = db.get("CarsAndStations").and_then(Value::as_array).into_iter().flatten().filter(|item| {
            id.as_deref().is_some_and(|id| item.get("Id").and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(id)))
                && revision.is_some_and(|v| item.get("Version").and_then(Value::as_u64) == Some(u64::from(v)))
        });
        let first = matches.next();
        let equipment = if matches.next().is_none() { first } else { None };
        if equipment.is_none() { unmatched = true; }
        let numeric = |name| equipment.and_then(|v| v.get(name)).and_then(Value::as_f64).filter(|v| v.is_finite());
        ImportedEvGroup {
            name: value(car, &["Name"]).map(str::to_owned),
            vehicle_count: number(car, &["AnzahlAutos", "Value"]),
            equipment_id: id, equipment_revision: revision,
            manufacturer: value(car, &["ECarAndStationDBHersteller"]).map(str::to_owned),
            equipment_name: value(car, &["ECarAndStationDBName"]).map(str::to_owned),
            battery_kwh: numeric("AkkuKapazitaet"),
            charging_kw: numeric("LadeLeistung").map(|w| w / 1000.0),
            consumption_kwh_per_100km: numeric("VerbrauchInKwhPro100km"),
            desired_range_km: number(car, &["GewuenschteReichweite", "Value"]),
            charging_mode_code: number(car, &["ChargingMode", "Value"]),
            automatic_phase_switching: value(car, &["HasAutomatischePhasenumschaltung"]).and_then(boolean),
            surplus_start_threshold: number(car, &["StartschwellePVUeberschuss", "Value"]),
            availability_slots: children(car, "AvailabilityStundenwerte")
                .map(|slot| value(slot, &["Availability"]).and_then(boolean)).collect(),
        }
    }).collect::<Vec<_>>();
    if !groups.is_empty() {
        warnings.push("EV source groups are retained as editable imported_ev_groups; availability order, charging mode, phase switching and surplus threshold semantics are unresolved, and they are not included in simulation. Configure a SolarCraft EV or fleet separately before using its results.".into());
        if unmatched { warnings.push("At least one EV equipment ID and revision did not uniquely match an embedded CarsAndStations record; equipment values were left empty.".into()); }
        project.imported_ev_groups = groups;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn keeps_multiple_absences_per_day_in_original_order() {
        let slots: String = (0..168).map(|i| {
            let present = !matches!(i % 24, 8..=11 | 15..=16);
            format!("<AvailabilityStundenwerte><Availability>{present}</Availability></AvailabilityStundenwerte>")
        }).collect();
        let xml = format!("<Project><ElektroAuto><Car>{slots}</Car></ElektroAuto></Project>");
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let mut p = Project::new("id", "name", 0.0, 0.0);
        apply(&mut p, doc.root_element(), &json!({}), &mut vec![]);
        let saved = &p.imported_ev_groups[0].availability_slots;
        assert_eq!(saved.len(), 168);
        assert_eq!(&saved[7..18], &[Some(true), Some(false), Some(false), Some(false), Some(false), Some(true), Some(true), Some(true), Some(false), Some(false), Some(true)]);
    }
    #[test]
    fn preserves_slots_and_resolves_only_matching_revision() {
        let xml = r#"<Project><ElektroAuto><Car><Name>Group</Name><ECarAndStationDBId>abc</ECarAndStationDBId><ECarAndStationDBVersion><Value>2</Value></ECarAndStationDBVersion><AnzahlAutos><Value>2</Value></AnzahlAutos><AvailabilityStundenwerte><Availability>True</Availability></AvailabilityStundenwerte><AvailabilityStundenwerte><Availability>False</Availability></AvailabilityStundenwerte><GewuenschteReichweite><Value>210</Value></GewuenschteReichweite><ChargingMode><Value>1</Value></ChargingMode><HasAutomatischePhasenumschaltung>false</HasAutomatischePhasenumschaltung><StartschwellePVUeberschuss><Value>0.5</Value></StartschwellePVUeberschuss></Car></ElektroAuto></Project>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let mut p = Project::new("id", "name", 0.0, 0.0);
        let mut warnings = vec![];
        apply(&mut p, doc.root_element(), &json!({"CarsAndStations":[{"Id":"ABC","Version":2,"AkkuKapazitaet":52,"LadeLeistung":11000,"VerbrauchInKwhPro100km":17.7}]}), &mut warnings);
        let g = &p.imported_ev_groups[0];
        assert_eq!(g.availability_slots, vec![Some(true), Some(false)]);
        assert_eq!((g.battery_kwh, g.charging_kw, g.desired_range_km), (Some(52.0), Some(11.0), Some(210.0)));
        assert_eq!((g.charging_mode_code, g.automatic_phase_switching, g.surplus_start_threshold), (Some(1), Some(false), Some(0.5)));
        assert!(p.fleet.is_none());
        p.set_by_pointer("/imported_ev_groups/0/availability_slots/0", json!(false)).unwrap();
        assert_eq!(p.imported_ev_groups[0].availability_slots[0], Some(false));
        assert!(solarcraft_model::validate::validate(&p).iter().any(|issue| issue.code == "IMPORTED_EV_NOT_SIMULATED"));
        assert_eq!(warnings.len(), 1);
        let roundtrip: Project = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(roundtrip.imported_ev_groups, p.imported_ev_groups);
        apply(&mut p, doc.root_element(), &json!({"CarsAndStations":[{"Id":"abc","Version":1,"AkkuKapazitaet":50}]}), &mut warnings);
        assert_eq!(p.imported_ev_groups[0].battery_kwh, None);
    }
}
