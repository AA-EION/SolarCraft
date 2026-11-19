// SPDX-License-Identifier: GPL-3.0-or-later
//! Grid-connected battery systems coupled on a hybrid inverter's DC side.
//!
//! PV*SOL saves the system under `Batteriesystem/BatSys`: the battery-system
//! record (`BatteryDBId` in `DB.json` `BatterySystems`, with its cells
//! embedded), the inverter it is wired to (`ConnectedWrOrMppIndex`) and the
//! coupling flags. The DC-generator coupling becomes SolarCraft's DC-hybrid
//! coupling: the system's DC/DC stage (nominal, charge and discharge power and
//! its E0..E100 part-load curve) becomes the inverter's battery port, and the
//! embedded cells become a detailed battery.
use super::{children, flag, number, path_node, value};
use crate::{e, Result};
use roxmltree::Node;
use serde_json::{json, Value};
use solarcraft_model::{
    equipment::{BatteryCells, BatteryPort, BatterySpec, Chemistry, Provenance},
    plant::{BatteryModel, CatalogRef, Coupling, EnergyUse, GridService, Storage},
    Project,
};

fn rating(record: &Value, key: &str, low: f64, high: f64) -> Result<f64> {
    record[key].as_f64().filter(|v| v.is_finite() && (low..=high).contains(v))
        .ok_or_else(|| e("PVSOL_IMPORT", format!("Invalid saved battery value {key}.")))
}

/// Migrate the saved battery system. Returns whether storage was imported;
/// unsupported layouts leave the project without storage and add a warning.
pub(super) fn apply(project: &mut Project, root: Node<'_, '_>, db: &Value, sha: &str, warnings: &mut Vec<String>) -> Result<bool> {
    let Some(group) = path_node(root, &["Batteriesystem"]) else { return Ok(false) };
    let systems: Vec<_> = children(group, "BatSys").collect();
    let [node] = systems.as_slice() else {
        if !systems.is_empty() { warnings.push("Several saved battery systems require separate migration.".into()); }
        return Ok(false);
    };
    let node = *node;
    let count = number::<u32>(node, &["AnzahlBatterieSysteme"]).unwrap_or(0);
    if count != 1 {
        warnings.push(format!("{count} parallel battery systems on one inverter require separate migration."));
        return Ok(false);
    }
    if flag(node, &["IsAcKopplung"]) || flag(node, &["IsDcKopplungZwischenkreis"]) || !flag(node, &["IsDcKopplungGenerator"]) {
        warnings.push("Only the DC generator-side battery coupling is migrated; this battery system's coupling requires separate migration.".into());
        return Ok(false);
    }
    let unit_index = number::<usize>(node, &["ConnectedWrOrMppIndex"]).unwrap_or(usize::MAX);
    let Some(unit) = project.electrical.inverters.get(unit_index).cloned() else {
        warnings.push("The battery system points to an inverter that was not imported.".into());
        return Ok(false);
    };
    let id = value(node, &["BatteryDBId"]).filter(|v| !v.is_empty() && v.len() <= 80)
        .ok_or_else(|| e("PVSOL_IMPORT", "The saved battery system has no database ID."))?;
    let revisions: Vec<_> = db["BatterySystems"].as_array().into_iter().flatten()
        .filter(|r| r["Id"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(id))).collect();
    let [system] = revisions.as_slice() else {
        return Err(e("PVSOL_IMPORT", "The saved battery system must have one identifiable embedded revision."));
    };
    let version = system["Version"].as_i64().unwrap_or(0);
    let battery = &system["Battery"];
    let chemistry_code = battery["BatteryType"].as_u64().and_then(|code| u32::try_from(code).ok());
    let chemistry = match chemistry_code {
        Some(101..=199) => Chemistry::LeadAcid, Some(201) => Chemistry::Lfp,
        Some(202..=206) => Chemistry::Nmc, _ => Chemistry::Other,
    };
    let per_string = system["NumberOfBatteriesPerString"].as_u64().filter(|v| (1..=1000).contains(v)).unwrap_or(1) as u32;
    let strings = system["NumberOfBatteryStrings"].as_u64().filter(|v| (1..=1000).contains(v)).unwrap_or(1) as u32;
    let series_cells = battery["NumberOfCellsSerie"].as_u64().and_then(|v| u32::try_from(v).ok())
        .and_then(|v| v.checked_mul(per_string)).filter(|v| (1..=10_000).contains(v))
        .ok_or_else(|| e("PVSOL_IMPORT", "Invalid series cell count in the saved battery system."))?;
    let parallel = battery["NumberOfCellsParallel"].as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| (1..=1000).contains(v)).unwrap_or(1);
    let cells = BatteryCells {
        chemistry_code, cell_voltage_v: rating(battery, "RatedVoltage", 0.1, 100.0)?, series: series_cells,
        // Published Ah is per catalog cell; parallel cells multiply it.
        parallel, strings,
        // InternalResistance is per catalog battery (block); convert it to one cell.
        internal_resistance_ohm: rating(battery, "InternalResistance", 0.0, 100.0)? * f64::from(parallel)
            / battery["NumberOfCellsSerie"].as_f64().filter(|v| *v >= 1.0).unwrap_or(1.0),
        capacity_ah_by_hours: [(0.167, "CapacityAt0167"), (0.5, "CapacityAt05"), (1.0, "CapacityAt1"),
            (5.0, "CapacityAt5"), (10.0, "CapacityAt10"), (100.0, "CapacityAt100")].into_iter()
            .filter_map(|(h, key)| battery[key].as_f64().filter(|x| (0.001..=1e8).contains(x)).map(|x| [h, x]))
            .collect(),
        cycles_by_dod: [(20.0, "LifeCycles20"), (40.0, "LifeCycles40"), (60.0, "LifeCycles60"), (80.0, "LifeCycles80")]
            .into_iter().filter_map(|(depth, key)| battery[key].as_f64().filter(|x| (1.0..=1e8).contains(x)).map(|x| [depth, x])).collect(),
    };
    rating(battery, "CapacityAt10", 0.001, 1e8)?;
    let nominal_w = rating(system, "NominalPower", 1.0, 1e8)?;
    let charge_w = rating(system, "MaxCharge", 1.0, 1e8)?;
    let discharge_w = rating(system, "MaxDischarge", 1.0, 1e8)?;
    let soc_min = rating(system, "SocMin", 0.0, 0.99)? * 100.0;
    let curve = [(0.0, "E0"), (0.05, "E5"), (0.10, "E10"), (0.20, "E20"), (0.30, "E30"), (0.50, "E50"), (0.75, "E75"), (1.0, "E100")]
        .into_iter().map(|(x, key)| rating(system, key, 0.01, 1.0).map(|eta| [x, eta])).collect::<Result<Vec<_>>>()?;
    let pack_v = cells.pack_voltage();
    let lead = chemistry == Chemistry::LeadAcid;
    let battery_id = format!("pvsol-battery-system-{id}");
    let catalog_ref = CatalogRef { family: "batterysystem".into(), id: id.to_owned(), version, source_sha256: sha.to_owned(), dataset: "BatterySystems".into() };
    project.library.batteries.retain(|b| b.id != battery_id);
    project.library.batteries.push(BatterySpec {
        catalog_ref: Some(catalog_ref), id: battery_id.clone(), provenance: Provenance::Imported, chemistry,
        manufacturer: system.pointer("/Company/Name").and_then(Value::as_str).unwrap_or_default().to_owned(),
        model: system["Name"].as_str().unwrap_or("Battery system").to_owned(),
        capacity_kwh: cells.energy_kwh(), nominal_voltage_v: pack_v, soc_min_pct: soc_min, soc_max_pct: 100.0,
        p_charge_max_kw: charge_w / 1000.0, p_discharge_max_kw: discharge_w / 1000.0,
        roundtrip_efficiency: if lead { 0.85 } else { 0.95 },
        self_discharge_pct_per_month: rating(battery, "Selfdischarge", 0.0, 1.0)? * 100.0,
        cycle_life: cells.cycles_at(80.0).unwrap_or(if lead { 1500.0 } else { 6000.0 }),
        cells: Some(cells), converter: None,
        unverified: json!({"boost_soc": [system["SocBoostStart"], system["SocBoostEnd"]], "float_soc_start": system["SocFloatStart"],
            "pack_voltage_window": "±20 % of nominal (the archive stores no converter voltage window)"}).as_object().unwrap().clone(),
        datasheet_ref: String::new(),
    });
    // The battery system's DC/DC stage sits on the inverter's DC side.
    let inverter = project.library.inverters.iter_mut().find(|i| i.id == unit.inverter_id)
        .ok_or_else(|| e("PVSOL_IMPORT", "The battery inverter record is missing from the project library."))?;
    inverter.battery_port = Some(BatteryPort {
        v_min: pack_v * 0.8, v_max: pack_v * 1.2, p_charge_max_w: charge_w, p_discharge_max_w: discharge_w,
        dc_dc_efficiency: curve.iter().find(|k| k[0] == 0.5).map_or(0.95, |k| k[1]),
        efficiency_curve: curve, p_rated_w: Some(nominal_w),
    });
    project.storage = Some(Storage {
        battery_id, units: 1, coupling: Coupling::DcHybrid { unit_id: unit.id.clone() },
        model: BatteryModel::Detailed, soc_min_pct: soc_min, soc_max_pct: 100.0, initial_soc_pct: 100.0,
        backup_reserve_pct: 0.0, grid_services_pct: 0.0,
        charge_priority: vec![EnergyUse::Backup, EnergyUse::SelfConsumption, EnergyUse::GridServices],
        grid_charge_backup: false, grid_service: GridService::None, outages: vec![], generator_id: None,
    });
    if flag(node, &["TimedChargeEnabled"]) || flag(node, &["TimedDischargeEnabled"]) {
        warnings.push("Timed battery charge and discharge windows require separate migration; SolarCraft uses self-consumption dispatch.".into());
    }
    warnings.push("The battery system is migrated as a DC-coupled detailed battery: its DC/DC part-load curve on the inverter's battery port, embedded cells with internal resistance, a full initial charge and self-consumption dispatch. Boost, float and equalisation charging stages are not replayed.".into());
    Ok(true)
}
