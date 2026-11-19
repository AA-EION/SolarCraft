// SPDX-License-Identifier: GPL-3.0-or-later
//! Legacy island designs use parallel battery clusters on a common AC bus.
use super::{children, document, equipment_record, flag, number, path_node, read_load_values, value, HOURS};
use crate::{e, Result};
use roxmltree::Node;
use serde_json::{json, Value};
use solarcraft_model::{
    equipment::{BatteryCells, BatteryInverterSpec, BatterySpec, Chemistry, GeneratorSpec, Provenance},
    plant::{BatteryModel, Coupling, EnergyUse, GridService, Storage},
    workflow::SystemType,
    Project,
};

pub(super) fn apply(project: &mut Project, bytes: &[u8], db: &Value, sha: &str, backup: bool, warnings: &mut Vec<String>) -> Result<()> {
    let doc = document(bytes, "OffGrid.xml")?;
    let root = doc.root_element();
    let clusters = path_node(root, &["ClustersData"]).ok_or_else(|| e("PVSOL_IMPORT", "Missing island battery clusters."))?;
    let clusters: Vec<_> = children(clusters, "Cluster").collect();
    let first = *clusters.first().filter(|_| clusters.len() <= 24)
        .ok_or_else(|| e("PVSOL_IMPORT", "Expected one to 24 saved island battery clusters."))?;
    let counts = |node| {
        let converters = number::<u32>(node, &["AnzahlWR"])?;
        let blocks = number::<u32>(node, &["AnzahlBatt"])?;
        let series = number::<u32>(node, &["AnzahlBattReihe"])?;
        (converters > 0 && blocks > 0 && series > 0 && blocks % series == 0).then_some((converters, blocks, series))
    };
    let (converters, blocks, series) = counts(first).ok_or_else(|| e("PVSOL_IMPORT", "Invalid island battery or converter counts."))?;
    let same_settings = |a: Node<'_, '_>, b: Node<'_, '_>, tag| {
        let settings = |node| path_node(node, &[tag]).map(|n| n.children().filter(Node::is_element)
            .map(|c| (c.tag_name().name().to_owned(), c.text().unwrap_or_default().trim().to_owned()))
            .collect::<std::collections::BTreeMap<_, _>>());
        settings(a) == settings(b)
    };
    if clusters.iter().any(|&c| counts(c) != Some((converters, blocks, series))
        || !same_settings(first, c, "Batterie") || !same_settings(first, c, "Batteriewechselrichter")) {
        return Err(e("PVSOL_IMPORT", "Mixed battery clusters on a shared island bus require separate migration."));
    }
    let banks = clusters.len() as u32;
    let converters = converters.checked_mul(banks).filter(|v| *v <= 1024)
        .ok_or_else(|| e("PVSOL_IMPORT", "Island converter count is out of range."))?;
    let strings = (blocks / series).checked_mul(banks).filter(|v| *v <= 100_000)
        .ok_or_else(|| e("PVSOL_IMPORT", "Island battery string count is out of range."))?;
    let (battery, battery_ref) = equipment_record(db, "battery", &["Batteries"], value(first, &["Batterie", "BatterieId"]),
        number(first, &["Batterie", "BatterieVersion"]), sha)?;
    let (converter, converter_ref) = equipment_record(db, "batteryinverter", &["BatteryInvertersOffgrid"],
        value(first, &["Batteriewechselrichter", "BatterieInvId"]), number(first, &["Batteriewechselrichter", "BatterieInvVersion"]), sha)?;
    let chemistry_code = battery["BatteryType"].as_u64().and_then(|code| u32::try_from(code).ok());
    let chemistry = match chemistry_code {
        Some(101..=199) => Chemistry::LeadAcid, Some(201) => Chemistry::Lfp,
        Some(202..=206) => Chemistry::Nmc, _ => Chemistry::Other,
    };
    let series_cells = battery["NumberOfCellsSerie"].as_u64().and_then(|v| u32::try_from(v).ok())
        .and_then(|v| v.checked_mul(series)).filter(|v| (1..=10_000).contains(v))
        .ok_or_else(|| e("PVSOL_IMPORT", "Invalid series cell count on the saved battery bank."))?;
    let cell_voltage_v = rating(battery, "RatedVoltage", 0.1, 100.0)?;
    let cells = BatteryCells {
        chemistry_code, cell_voltage_v, series: series_cells,
        // Published Ah is per catalog block, including its internal parallels.
        parallel: 1, strings,
        internal_resistance_ohm: rating(battery, "InternalResistance", 0.0, 100.0)?,
        capacity_ah_by_hours: [(0.167, "CapacityAt0167"), (0.5, "CapacityAt05"), (1.0, "CapacityAt1"),
            (5.0, "CapacityAt5"), (10.0, "CapacityAt10"), (100.0, "CapacityAt100")].into_iter()
            .filter_map(|(h, key)| battery[key].as_f64().filter(|x| (0.001..=1e8).contains(x)).map(|x| [h, x]))
            .collect(),
        cycles_by_dod: [(20.0, "LifeCycles20"), (40.0, "LifeCycles40"), (60.0, "LifeCycles60"), (80.0, "LifeCycles80")]
            .into_iter().filter_map(|(depth, key)| battery[key].as_f64().filter(|x| (1.0..=1e8).contains(x)).map(|x| [depth, x])).collect(),
    };
    rating(battery, "CapacityAt10", 0.001, 1e8)?;
    let soc_min = number::<f64>(first, &["Batterie", "SocMin"]).filter(|v| (0.0..100.0).contains(v))
        .ok_or_else(|| e("PVSOL_IMPORT", "Invalid minimum state of charge on the saved battery bank."))?;
    let nominal_voltage = cells.pack_voltage();
    let max_dc_a = rating(converter, "Batt_MaxCurrent", 0.001, 1e6)? * f64::from(converters);
    let max_dc_kw = nominal_voltage * max_dc_a / 1000.0;
    let charge_share = number::<f64>(first, &["Batterie", "ILadeMaxProzent"])
        .filter(|v| (0.01..=100.0).contains(v)).map_or(1.0, |v| v / 100.0);
    let lead = chemistry == Chemistry::LeadAcid;
    let battery_id = format!("pvsol-battery-{}", battery_ref.id);
    let converter_id = format!("pvsol-battery-inverter-{}", converter_ref.id);
    project.library.batteries.push(BatterySpec {
        catalog_ref: Some(battery_ref), id: battery_id.clone(), provenance: Provenance::Imported, chemistry,
        manufacturer: label(battery.pointer("/Company/Name")),
        model: format!("{} · {banks} cluster(s)", label(battery.get("Name"))),
        capacity_kwh: cells.energy_kwh(), nominal_voltage_v: nominal_voltage, soc_min_pct: soc_min, soc_max_pct: 100.0,
        p_charge_max_kw: max_dc_kw * charge_share, p_discharge_max_kw: max_dc_kw,
        roundtrip_efficiency: if lead { 0.85 } else { 0.95 },
        self_discharge_pct_per_month: rating(battery, "Selfdischarge", 0.0, 1.0)? * 100.0,
        cycle_life: cells.cycles_at(80.0).unwrap_or(if lead { 1500.0 } else { 6000.0 }),
        cells: Some(cells), converter: None,
        unverified: json!({"shared_ac_clusters":banks, "blocks_per_cluster":blocks, "blocks_per_string":series,
            "charge_current_limit_fraction":charge_share}).as_object().unwrap().clone(),
        datasheet_ref: String::new(),
    });
    let efficiency_curve = [(5.0, "E5"), (10.0, "E10"), (20.0, "E20"), (30.0, "E30"),
        (50.0, "E50"), (75.0, "E75"), (100.0, "E100")].into_iter()
        .map(|(pct, key)| rating(converter, key, 0.01, 1.0).map(|eff| [pct / 100.0, eff]))
        .collect::<Result<Vec<_>>>()?;
    project.library.battery_inverters.push(BatteryInverterSpec {
        catalog_ref: Some(converter_ref), id: converter_id.clone(), provenance: Provenance::Imported,
        manufacturer: label(converter.pointer("/Company/Name")),
        model: format!("{} × {converters}", label(converter.get("Name"))),
        p_ac_max_w: rating(converter, "AC_RatedPower_25C", 0.001, 1e8)? * f64::from(converters),
        efficiency_curve, standby_w: rating(converter, "Consumption_Standby", 0.0, 1e6)? * f64::from(converters),
        battery_v_min: Some(rating(converter, "Batt_MinInputVoltage", 0.001, 1e5)?),
        battery_v_max: Some(rating(converter, "Batt_MaxInputVoltage", 0.001, 1e5)?), battery_i_max: Some(max_dc_a),
        datasheet_ref: String::new(),
    });
    let generator_id = if backup {
        let id = value(root, &["ZusatzgeneratorData", "GeneratorId"]);
        let revisions: Vec<_> = db["Generators"].as_array().into_iter().flatten()
            .filter(|item| item["Id"].as_str().zip(id).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))).collect();
        let [record] = revisions.as_slice() else {
            return Err(e("PVSOL_IMPORT", "The saved backup generator must have one identifiable embedded revision."));
        };
        let (_, catalog_ref) = equipment_record(db, "generator", &["Generators"], id, record["Version"].as_i64(), sha)?;
        let id = format!("pvsol-generator-{}", catalog_ref.id);
        let fuel_curve = [(0.0, "FuelConsumptionAt0"), (25.0, "FuelConsumptionAt25"), (50.0, "FuelConsumptionAt50"),
            (75.0, "FuelConsumptionAt75"), (100.0, "FuelConsumptionAt100")].into_iter()
            .map(|(pct, key)| rating(record, key, 0.0, 1e6).map(|lph| [pct / 100.0, lph]))
            .collect::<Result<Vec<_>>>()?;
        let phases = record["NumberOfPhases"].as_u64().filter(|v| matches!(*v, 1 | 3))
            .ok_or_else(|| e("PVSOL_IMPORT", "Invalid generator phase count."))? as u32;
        project.library.generators.push(GeneratorSpec {
            catalog_ref: Some(catalog_ref), id: id.clone(), provenance: Provenance::Imported,
            manufacturer: label(record.pointer("/Company/Name")), model: label(record.get("Name")),
            rated_va: rating(record, "AC_RatedPower", 0.001, 1e9)?, max_va: rating(record, "AC_MaxPower", 0.001, 1e9)?,
            voltage_v: rating(record, "AC_RatedVoltage", 0.001, 1e5)?, phases, fuel_curve, fuel_unit: "L".into(),
            datasheet_ref: String::new(),
        });
        warnings.push("The archived generator uses time- and load-dependent switching. SolarCraft uses its shortage-driven dispatch for this import.".into());
        Some(id)
    } else { None };
    project.workflow.system = if backup { SystemType::OffGridGenerator } else { SystemType::OffGrid };
    project.storage = Some(Storage {
        battery_id, units: 1, coupling: Coupling::AcCoupled { battery_inverter_id: converter_id },
        model: BatteryModel::Detailed, soc_min_pct: soc_min, soc_max_pct: 100.0, initial_soc_pct: soc_min.max(50.0),
        backup_reserve_pct: 0.0, grid_services_pct: 0.0,
        charge_priority: vec![EnergyUse::Backup, EnergyUse::SelfConsumption, EnergyUse::GridServices],
        grid_charge_backup: false, grid_service: GridService::None, outages: vec![], generator_id,
    });
    if banks > 1 {
        warnings.push(format!("The {banks} identical battery clusters share one AC bus and are represented as one equivalent bank. Cluster-specific balancing is not replayed."));
    }
    warnings.push("SolarCraft applies the saved charge-current percentage to the converter DC limit and uses its own charge taper, chemistry-based cell efficiency and initial state of charge of at least 50%. Staged charging and load-shedding controls require separate migration.".into());
    Ok(())
}

fn rating(record: &Value, key: &str, low: f64, high: f64) -> Result<f64> {
    record[key].as_f64().filter(|x| (low..=high).contains(x))
        .ok_or_else(|| e("PVSOL_IMPORT", format!("Missing or invalid island equipment rating {key}.")))
}

fn label(field: Option<&Value>) -> String {
    field.and_then(Value::as_str).unwrap_or_default().to_owned()
}

pub(super) fn load(root: Node<'_, '_>, warnings: &mut Vec<String>) -> Result<Option<Vec<f64>>> {
    let Some(consumption) = path_node(root, &["Verbrauch"]) else { return Ok(None); };
    let mut total = vec![0.0; HOURS];
    let mut skipped = 0;
    for definition in children(consumption, "VerbrauchGruppen").flat_map(|g| children(g, "Verbrauchsdefinition")) {
        let Some(load) = definition.first_element_child() else { continue; };
        let rows = match load.tag_name().name() {
            "GeneratedConsumption" => generated(load),
            "EinzelverbrauchNeu" => match value(load, &["Values"]) {
                Some(encoded) => Some(read_load_values(encoded, 60, HOURS)?),
                None => appliance(load),
            },
            "Ueberschussverbrauch" if !flag(load, &["IsOn"]) => continue,
            _ => None,
        };
        if let Some(rows) = rows {
            for (site, individual) in total.iter_mut().zip(rows) { *site += individual; }
        } else { skipped += 1; }
    }
    if skipped > 0 {
        warnings.push(format!("{skipped} island load definitions had an unsupported schedule and are missing from the reconstructed demand."));
    }
    if total.iter().any(|&v| v > 0.0) {
        warnings.push("Island demand combines saved appliance intervals, reconstructed schedules and generated load shapes. It uses the archive's individual annual demand settings and the SolarCraft reference calendar.".into());
        Ok(Some(total))
    } else { Ok(None) }
}

fn repeated<'a, T: std::str::FromStr>(node: Node<'a, '_>, tag: &'static str, count: usize) -> Option<Vec<T>> {
    let rows = children(node, tag).map(|n| n.text()?.trim().parse().ok()).collect::<Option<Vec<_>>>()?;
    (rows.len() == count).then_some(rows)
}

fn positive(node: Node<'_, '_>, key: &str) -> Option<f64> {
    number::<f64>(node, &[key]).filter(|x| (0.0..=1e9).contains(x))
}

fn generated(node: Node<'_, '_>) -> Option<Vec<f64>> {
    const MONTHS: [usize; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let annual = positive(node, "Strombedarf")?;
    let weights = if flag(node, &["HasDifferentJahresverlauf"]) { repeated::<f64>(node, "Jahresverlauf", 12)? }
        else { MONTHS.iter().map(|&days| days as f64).collect() };
    let use_day = flag(node, &["HasDifferentTagesverlauf"]);
    let common = if use_day { repeated::<f64>(node, "Tagesverlauf", 24)? } else { vec![1.0; 24] };
    let by_week = flag(node, &["HasDifferentWochenverlauf"]);
    let mut rows = Vec::with_capacity(HOURS);
    for (month, days) in MONTHS.into_iter().enumerate() {
        let season = match month { 11 | 0 | 1 => "Winter", 5..=7 => "Sommer", _ => "Uebergang" };
        let single = match season { "Winter" => "Wintertagverlauf", "Sommer" => "Sommertagverlauf", _ => "Uebergangstagverlauf" };
        let seasonal = repeated::<f64>(node, single, 24);
        let first_day = rows.len() / 24;
        let first_row = rows.len();
        for day in first_day..first_day + days {
            let weekday = (day + 2) % 7; // 1 January 2025, Wednesday.
            let pattern = match (season, weekday) {
                ("Winter", 5) => "WinterSamstag", ("Winter", 6) => "WinterSonntag", ("Winter", _) => "WinterWerktag",
                ("Sommer", 5) => "SommerSamstag", ("Sommer", 6) => "SommerSonntag", ("Sommer", _) => "SommerWerktag",
                (_, 5) => "UebergangSamstag", (_, 6) => "UebergangSonntag", _ => "UebergangWerktag",
            };
            let weekly = match weekday { 5 => "Samstagsverlauf", 6 => "Sonntagsverlauf", _ => "Werktagsverlauf" };
            let profile = if by_week { repeated::<f64>(node, if use_day { pattern } else { weekly }, 24) } else { None };
            rows.extend(profile.as_ref().or(seasonal.as_ref().filter(|_| use_day)).unwrap_or(&common));
        }
        let monthly = &mut rows[first_row..];
        let sum: f64 = monthly.iter().sum();
        if monthly.iter().any(|v| !v.is_finite() || *v < 0.0) || sum <= 0.0 { return None; }
        for v in monthly { *v *= weights[month] / sum; }
    }
    let sum: f64 = rows.iter().sum();
    if sum <= 0.0 || rows.iter().any(|v| !v.is_finite() || *v < 0.0) { return None; }
    for v in &mut rows { *v *= annual / sum; }
    Some(rows)
}

fn appliance(node: Node<'_, '_>) -> Option<Vec<f64>> {
    match number::<u32>(node, &["Typ"])? {
        0 => Some(vec![positive(node, "Jahresstrombedarf")? / HOURS as f64; HOURS]),
        kind @ (1 | 2) => {
            if number::<u32>(node, &["WeitereEinschraenkungTyp1"])? != 0 { return None; }
            let active_days = repeated::<bool>(node, "Jahresschaltuhr", 365)?;
            let fractions: Vec<f64> = if kind == 1 {
                repeated::<bool>(node, "ZeitschaltuhrTyp1", 168)?.into_iter().map(|v| if v { 1.0 } else { 0.0 }).collect()
            } else {
                let duration = positive(node, "NutzungsdauerOrStrombedarf")?;
                let hours = match number::<u32>(node, &["StundenOrMinuten"])? { 0 => duration / 60.0, 1 => duration, _ => return None };
                repeated::<u32>(node, "BenutzungenProStunde", 168)?.into_iter().map(|v| (f64::from(v) * hours).min(1.0)).collect()
            };
            let power = positive(node, "Leistung")? / 1000.0;
            let standby = positive(node, "StandbyLeistung").unwrap_or(0.0) / 1000.0;
            Some((0..HOURS).map(|hour| {
                let day = hour / 24;
                if !active_days[day] { return 0.0; }
                // The saved appliance series begin at the first weekly slot.
                let active = fractions[hour % 168];
                power * active + standby * (1.0 - active)
            }).collect())
        }
        _ => None,
    }
}
