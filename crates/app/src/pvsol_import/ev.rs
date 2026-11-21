// SPDX-License-Identifier: GPL-3.0-or-later
//! Bind archived charging settings to the hourly fleet dispatch.
use super::{children, flag, number, value};
use roxmltree::Node;
use serde_json::Value;
use solarcraft_model::{
    demand::{ChargeStrategy, Fleet, FleetGroup, ImportedEvGroup, SurplusChargeStart, WeeklyFleetSchedule, WeeklyScheduleOrigin},
    plant::Grid,
    Project,
};

fn boolean(s: &str) -> Option<bool> {
    if s.eq_ignore_ascii_case("true") || s == "1" { Some(true) }
    else if s.eq_ignore_ascii_case("false") || s == "0" { Some(false) }
    else { None }
}

fn dispatch(source: &ImportedEvGroup, grid: &Grid, name: String, discharge_kw: f64) -> Option<FleetGroup> {
    let weekly_schedule = WeeklyFleetSchedule {
        slots: source.availability_slots.iter().copied().collect::<Option<_>>()?,
        origin: WeeklyScheduleOrigin::YearStart,
    };
    let departures = weekly_schedule.departures();
    let weekly_km = source.desired_range_km.filter(|km| *km >= 0.0 && km.is_finite())?;
    if departures == 0 && weekly_km > 0.0 { return None; }
    let consumption = source.consumption_kwh_per_100km.filter(|kwh| *kwh > 0.0)?;
    let trip_kwh = weekly_km * consumption / 100.0 / departures.max(1) as f64;
    let battery_kwh = source.battery_kwh.filter(|kwh| *kwh > 0.0)?;
    let charge_kw = source.charging_kw?;
    let departure_soc_pct = 20.0 + trip_kwh / battery_kwh * 100.0;
    let strategy = match source.charging_mode_code? {
        0 => ChargeStrategy::Immediate,
        1 => ChargeStrategy::SolarPreferred,
        _ => return None,
    };
    let surplus_start = if strategy == ChargeStrategy::SolarPreferred {
        let phases = if source.automatic_phase_switching? { 1 } else { grid.phases };
        Some(SurplusChargeStart {
            minimum_kw: (6.0 * grid.voltage_v * f64::from(phases) / 1000.0).min(charge_kw),
            fraction: source.surplus_start_threshold?,
        })
    } else { None };
    let group = FleetGroup {
        name, vehicles: source.vehicle_count?, battery_kwh,
        initial_soc_pct: departure_soc_pct, departure_soc_pct, trip_kwh,
        weekdays: vec![], arrival_hour: 0, departure_hour: 0,
        charge_kw, charge_efficiency: 0.92, discharge_kw, discharge_efficiency: 0.92,
        strategy, weekly_schedule: Some(weekly_schedule), surplus_start,
    };
    group.valid().then_some(group)
}

pub(super) fn apply(project: &mut Project, root: Node<'_, '_>, db: &Value, warnings: &mut Vec<String>) {
    let Some(section) = root.descendants().find(|n| n.has_tag_name("ElektroAuto")) else { return; };
    let mut archived = Vec::new();
    let mut groups: Vec<FleetGroup> = Vec::new();
    for (index, car) in children(section, "Car").enumerate() {
        let id = value(car, &["ECarAndStationDBId"]).map(str::to_owned);
        let revision: Option<u32> = number(car, &["ECarAndStationDBVersion", "Value"]);
        let mut matches = db.get("CarsAndStations").and_then(Value::as_array).into_iter().flatten().filter(|item| {
            id.as_deref().is_some_and(|id| item.get("Id").and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(id)))
                && revision.is_some_and(|v| item.get("Version").and_then(Value::as_u64) == Some(u64::from(v)))
        });
        let first = matches.next();
        let equipment = if matches.next().is_none() { first } else { None };
        let numeric = |name| equipment.and_then(|v| v.get(name)).and_then(Value::as_f64).filter(|v| v.is_finite());
        let source = ImportedEvGroup {
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
        };
        let mut name = source.name.as_deref().or(source.equipment_name.as_deref())
            .filter(|name| !name.trim().is_empty() && name.len() <= 90)
            .map_or_else(|| format!("EV {}", index + 1), str::to_owned);
        if groups.iter().any(|group| group.name == name) {
            let base = name;
            name = format!("{base} ({})", index + 1);
            while groups.iter().any(|group| group.name == name) { name.push('+'); }
        }
        let discharge_kw = if flag(car, &["CanDischargeUser"])
            && equipment.and_then(|v| v["CanDischarge"].as_bool()) == Some(true)
        {
            numeric("EntLadeLeistung").map_or(0.0, |w| w / 1000.0)
        } else { 0.0 };
        if equipment.is_none() {
            warnings.push(format!("EV group {} has no unique embedded equipment revision and was excluded from dispatch.", index + 1));
        } else if let Some(group) = dispatch(&source, &project.grid, name, discharge_kw) {
            groups.push(group);
        } else {
            warnings.push(format!("EV group {} has incomplete or unsupported schedule, travel or charger settings and was excluded from dispatch.", index + 1));
        }
        archived.push(source);
    }
    project.imported_ev_groups = archived;
    if !groups.is_empty() {
        let fleet = Fleet {
            site_charge_kw: groups.iter().map(|g| g.charge_kw * f64::from(g.vehicles)).sum(),
            site_discharge_kw: groups.iter().map(|g| g.discharge_kw * f64::from(g.vehicles)).sum(),
            groups, v2g: None,
        };
        if fleet.valid() {
            project.fleet = Some(fleet);
            warnings.push("EV dispatch uses the archived year-start weekly availability, trip energy and charger settings. Initial SOC is the departure target, conversion uses constant efficiency, and charging decisions are hourly.".into());
        } else {
            warnings.push("The archived EV fleet exceeds SolarCraft's group or aggregate charger limits and was excluded from dispatch.".into());
        }
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
