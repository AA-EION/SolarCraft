// SPDX-License-Identifier: GPL-3.0-or-later
//! Field coverage: every entry of the field reference (source, dataset, exact path) is bound to
//! where SolarCraft consumes, validates, displays, identifies or preserves it. The report joins
//! the reference's own type, unit and evidence with these bindings and the presence counts
//! observed during import.
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Drives a calculation result.
    Consumed,
    /// Drives a validation check (design feasibility).
    Validated,
    /// Shown in catalog selection or equipment details, or used for search.
    Displayed,
    /// Identity, joins, revisions, provenance.
    Identity,
    /// Kept verbatim; meaning or consumer not yet verified, or feature not implemented.
    Preserved,
    /// Legacy schema reference (no rows in this backup).
    SchemaReference,
}

#[derive(Debug, Clone, Serialize)]
pub struct Binding { pub role: Role, pub consumer: String, pub open: String }

fn bnd(role: Role, consumer: &str, open: &str) -> Binding { Binding { role, consumer: consumer.into(), open: open.into() } }

/// Bind one field entry.
pub fn bind(source: &str, dataset: &str, path: &str) -> Binding {
    use Role::*;
    if source.starts_with("Legacy") { return bnd(SchemaReference, "schema reference for the legacy offline catalog (0 local rows in this backup)", "no local fixture to verify against"); }
    if source.starts_with("Backup") {
        return match path {
            "record_json" => bnd(Identity, "catalog::import parses it as the authoritative record (entity.raw_json)", ""),
            "dataset_id" | "id" if dataset == "records" || dataset == "datasets" => bnd(Identity, "catalog::import joins records.dataset_id = datasets.id", ""),
            "body" | "sha256" | "body_bytes" | "route" => bnd(Identity, "catalog::import archives api_responses verbatim and verifies SHA-256 and length (api_response)", "route kept private"),
            _ => bnd(Identity, "catalog::import / catalog::audit (provenance and counts)", ""),
        };
    }
    let leaf = path.rsplit('.').next().unwrap_or(path).trim_end_matches("[]");
    let top = path.split('.').next().unwrap_or(path);
    let generic = |l: &str| -> Option<Binding> {
        Some(match l {
            "id" | "version" | "companyId" | "countryId" | "guid" => bnd(Identity, "catalog::import identity (family, id, version); pinned as CatalogRef in projects", ""),
            "dbEntryType" | "productType" | "dbEntryTypeEnum" => bnd(Identity, "family discriminator kept with the record (checked by dataset family)", ""),
            "name" | "dbEntryName" => bnd(Displayed, "catalog::search (search text) and equipment details", ""),
            "shortName" | "website" | "companyName" => bnd(Displayed, "catalog::search / details (company)", ""),
            "description" => bnd(Displayed, "equipment details (normalised description)", ""),
            "publishedAt" | "publishDate" => bnd(Identity, "catalog::details history/release matching", ""),
            "isAvailable" => bnd(Displayed, "catalog::search available_only filter", ""),
            "isCorrupt" => bnd(Preserved, "raw record; surfaced in details", "no corrupt records observed"),
            "dbUserId" => bnd(Preserved, "raw record only (never displayed)", "access semantics"),
            "graphic" => bnd(Preserved, "raw record (not rendered)", "media validation"),
            "$id" => bnd(Preserved, "decoded schedule (serializer metadata)", ""),
            _ => return None,
        })
    };
    // Embedded projections.
    if path == "company" || path.ends_with(".company") { return bnd(Identity, "catalog::import join_check company projection vs companyId/companies", ""); }
    if top == "company" && path.contains('.') || path.contains(".company.") {
        return generic(leaf).map(|mut b| { if b.role == Identity { b.consumer = "catalog::import join_check company projection vs companyId/companies".into(); } b }).unwrap_or_else(|| bnd(Displayed, "company details", ""));
    }
    if top == "country" && path.contains('.') { return bnd(Displayed, "tariff country (search in all locales, details); id checked in join_check", ""); }
    if top == "powerOptimizer" {
        if path == "powerOptimizer" { return bnd(Identity, "join_check optimizer snapshot vs powerOptimizerId/Version", ""); }
        return generic(leaf).unwrap_or_else(|| bnd(Preserved, "embedded optimizer snapshot (optimizers not simulated)", "optimizer modelling not implemented"));
    }
    if dataset.ends_with("batterysystems") && top == "battery" && path.contains('.') {
        return generic(leaf).unwrap_or_else(|| bnd(Identity, "embedded battery snapshot: compared with the linked revision in join_check; values used only on explicit fallback", ""));
    }
    if let Some(b) = generic(leaf) { if !dataset.ends_with("countries") || leaf == "id" { return b; } }
    let ds = dataset.trim_start_matches("Online ");
    match (ds, leaf) {
        ("pvmodules", "cellCount") | ("pvmodules", "isHalfCellModule") => bnd(Consumed, "normalize::module series cells → diode fit (pv::diode::fit_cec) and string voltage", "topology exceptions beyond the half-cell rule"),
        ("pvmodules", "openCircuitVoltage" | "shortCircuitCurrent" | "mpP_Voltage" | "mpP_Current") => bnd(Consumed, "ModuleSpec → CEC fit → electrical::operate (I–V), validate string Voc/Vmp/Isc", ""),
        ("pvmodules", "voltageCoefficient" | "currentCoefficient" | "outputpowerCoefficient") => bnd(Consumed, "ModuleSpec beta_voc (mV/K÷1000), alpha_sc (mA/K÷1000), gamma (×100 %/K) → CEC fit", ""),
        ("pvmodules", "g2" | "umpp2" | "impp2" | "uoc2" | "isc2") => bnd(Validated, "ModuleSpec.low_light → fit check low_light_pmp_rel_err", "not used as a fit anchor (CEC fit is STC-based)"),
        ("pvmodules", "standardPartLoadOperation") => bnd(Preserved, "unverified map; tells whether the second point is explicit", "selection of the part-load model"),
        ("pvmodules", "width" | "height") => bnd(Consumed, "ModuleSpec length/width → roof layout (geometry::layout::fill) and module area", ""),
        ("pvmodules", "maxSysVoltage") => bnd(Validated, "validate MODULE_SYSTEM_VOLTAGE", ""),
        ("pvmodules", "increaseU0") => bnd(Validated, "validate: cold Voc × (1 + increaseU0) against module and inverter limits", ""),
        ("pvmodules", "onlyTransformerInverter") => bnd(Validated, "validate MODULE_NEEDS_TRANSFORMER", ""),
        ("pvmodules", "specifiedOutputPower") => bnd(Displayed, "nameplate_w in details", "array kWp uses Vmp×Imp"),
        ("pvmodules", "diodeCount" | "lossVoltagePerBypassDiode") => bnd(Preserved, "ModuleSpec bypass_diodes / bypass_diode_voltage_v", "bypass-diode partial shading not modelled (ELEC-013)"),
        ("pvmodules", "standardCellConnection") => bnd(Preserved, "unverified map", "true/false orientation for shading"),
        ("pvmodules", "cellType") => bnd(Displayed, "technology label (CELL_TYPES); unverified map", "model selection by cell type"),
        ("pvmodules", "iam") => bnd(Preserved, "unverified map", "angular equation of the reflection model"),
        ("pvmodules", "bifacialityFactor") => bnd(Preserved, "ModuleSpec.bifaciality", "bifacial rear irradiance not modelled (SIM-009)"),
        ("pvmodules", "depthOfPanel" | "weight") => bnd(Displayed, "ModuleSpec thickness/weight in details", ""),
        ("pvmodules", "widthOfFrame") => bnd(Preserved, "unverified map", "physical representation only"),
        ("pvmodules", "powerOptimizerId" | "powerOptimizerVersion") => bnd(Identity, "join_check optimizer revision", ""),
        ("pvinverters", l) if l.starts_with("mppT") => bnd(Consumed, "InverterSpec.inputs → electrical::operate_limited (window, current, power) and validate per input (Isc, current, power)", ""),
        ("pvinverters", "maxInputVoltageDC") => bnd(Validated, "validate STRING_VOC_MAX", ""),
        ("pvinverters", "maxAC_Power") => bnd(Consumed, "energy::inverter_ac clipping", ""),
        ("pvinverters", "dC_PowerRating") => bnd(Consumed, "efficiency-curve load reference (pdc_nominal_w)", "load reference of the curve (DC vs AC rating)"),
        ("pvinverters", "maxDC_Power") => bnd(Consumed, "energy::inverter_ac DC power limit", ""),
        ("pvinverters", "nominalVoltageDC" | "changeOfEtaDependentOnVoltage") => bnd(Consumed, "InverterSpec::efficiency_v: η − c·|V − Vnom|/100 at the operating voltage", "exact formula in the selected model"),
        ("pvinverters", "feedInFrom") => bnd(Consumed, "energy::inverter_ac start threshold (DC side)", "power side at each consumer"),
        ("pvinverters", "nightConsumption") => bnd(Consumed, "standby load when no DC input (dispatch) and ledger night step", ""),
        ("pvinverters", "standbyConsumption") => bnd(Displayed, "InverterSpec.standby_w", "which state uses it vs night consumption"),
        ("pvinverters", "matchingEtaMPP20" | "matchingEtaMPP100") => bnd(Consumed, "InverterSpec::efficiency_v MPP adaptation factor (below/above 20 % of nominal DC)", "exact 20 % boundary"),
        ("pvinverters", "parallelOperating") => bnd(Validated, "validate MPPT_INTERCONNECT; engine combines interconnected trackers", ""),
        ("pvinverters", "maxCurrentDC" | "maxShortCircuitCurrentDC") => bnd(Validated, "validate INVERTER_IDC_TOTAL / INVERTER_ISC_TOTAL", ""),
        ("pvinverters", "dC_InputCount") => bnd(Validated, "validate INVERTER_CONNECTORS", ""),
        ("pvinverters", "numberOfPhases") => bnd(Validated, "validate INVERTER_PHASES against grid phases", ""),
        ("pvinverters", "withTransformer") => bnd(Validated, "validate MODULE_NEEDS_TRANSFORMER", ""),
        ("pvinverters", "voltageAC") => bnd(Displayed, "InverterSpec.v_ac", "L-N/L-L convention"),
        ("pvinverters", "aC_PowerRating") => bnd(Preserved, "unverified map", "nominal AC reference vs maxAC_Power"),
        (_, "e0") => bnd(Preserved, "issue note when non-zero; curve starts at 5 %", "interpolator below 5 %"),
        (_, "e5" | "e10" | "e20" | "e30" | "e50" | "e75" | "e100") if ds.starts_with("poweroptimizers") => bnd(Preserved, "optimizer record", "optimizers not simulated"),
        (_, "e5" | "e10" | "e20" | "e30" | "e50" | "e75" | "e100") => bnd(Consumed, "efficiency curve (inverter / battery converter / battery inverter) → energy::dispatch", "load reference and direction for converters"),
        ("batterysystems", "nominalPower" | "maxCharge" | "maxDischarge") => bnd(Consumed, "BatterySpec power limits and converter rating → energy::dispatch", ""),
        ("batterysystems", "coupling") => bnd(Displayed, "suggested SolarCraft coupling (1 → integrated, 2/3 → dc_hybrid); the user selects the topology", "mapping of 2 vs 3 to a specific port"),
        ("batterysystems", "socMin") => bnd(Consumed, "BatterySpec.soc_min_pct = max(0.05, socMin) (traced DC controller floor)", ""),
        ("batterysystems", "batteryId" | "batteryVersion") => bnd(Identity, "join_check: linked battery revision (authoritative)", ""),
        ("batterysystems", "numberOfBatteriesPerString" | "numberOfBatteryStrings") => bnd(Consumed, "BatteryCells series × per-string, strings → capacity and pack voltage/resistance", ""),
        ("batterysystems", "battery") => bnd(Identity, "embedded battery snapshot compared in join_check", ""),
        ("batterysystems", _) => bnd(Preserved, "unverified map (maintenance charge strategy)", "boost/full/equalisation modes not implemented"),
        ("batteries", "ratedVoltage" | "numberOfCellsSerie" | "numberOfCellsParallel" | "capacityAt10") | (_, "ratedVoltage" | "numberOfCellsSerie" | "numberOfCellsParallel" | "capacityAt10") => bnd(Consumed, "BatteryCells → energy_kwh (V × Ah@10h × parallel × strings) and pack voltage", ""),
        (_, "capacityAt0167" | "capacityAt05" | "capacityAt1" | "capacityAt5" | "capacityAt100") => bnd(Preserved, "BatteryCells.capacity_ah_by_hours (interpolator available)", "rate-dependent capacity not used in dispatch"),
        (_, "internalResistance") => bnd(Consumed, "pack resistance → I²R loss in the detailed battery model", "cell/module scaling per chemistry"),
        (_, "selfdischarge") => bnd(Consumed, "self_discharge_pct_per_month → hourly self-discharge", ""),
        (_, l) if l.starts_with("lifeCycles") => bnd(Consumed, "cycles_by_dod → cycle_life at 80 % DoD → ageing estimate (−1 = missing point)", "full DoD-dependent ageing"),
        (_, "batteryType") => bnd(Consumed, "chemistry (efficiency default, labels)", "fitted voltage profile per chemistry"),
        ("batteries", "length" | "width" | "height" | "weight") => bnd(Displayed, "battery details (m, kg as stored)", ""),
        ("batteryinverters", "aC_RatedPower_25C") => bnd(Consumed, "BatteryInverterSpec.p_ac_max_w → AC-coupled dispatch", ""),
        ("batteryinverters", "consumption_Standby") => bnd(Consumed, "standby load in dispatch", ""),
        ("batteryinverters", "batt_MinInputVoltage" | "batt_MaxInputVoltage") => bnd(Validated, "validate STORAGE_VOLTAGE", ""),
        ("batteryinverters", "batt_MaxCurrent") => bnd(Consumed, "charge/discharge limit = I × pack voltage (AC coupling)", "discharge-side behaviour"),
        ("batteryinverters", _) => bnd(Preserved, "unverified map", "off-grid cluster ratings not modelled"),
        ("generators", "aC_RatedPower") => bnd(Consumed, "GeneratorSpec.rated_va → fuel curve load fraction", "power factor"),
        ("generators", "aC_MaxPower") => bnd(Consumed, "generator output cap during outages", ""),
        ("generators", l) if l.starts_with("fuelConsumption") => bnd(Consumed, "GeneratorSpec::fuel_rate → balance generator_fuel", "fuel volume unit"),
        ("generators", _) => bnd(Displayed, "generator details", ""),
        (d, _) if d.ends_with("tariffs") => tariff_binding(path, leaf),
        ("companies", _) => bnd(Displayed, "company search and details", ""),
        ("countries", "id") => bnd(Identity, "country join key for tariffs", ""),
        ("countries", _) => bnd(Displayed, "tariff search in all locales; country details", ""),
        ("database versions" | "publish metadata", _) => bnd(Displayed, "catalog::history releases; details release matching", ""),
        ("changelog", _) => bnd(Displayed, "catalog::details history (created / new version)", ""),
        ("poweroptimizers" | "carsandstations", _) => bnd(Preserved, "descriptive record in details", "optimizer/EV not simulated"),
        _ => bnd(Preserved, "raw record", "no binding"),
    }
}

fn tariff_binding(path: &str, leaf: &str) -> Binding {
    use Role::*;
    if path.starts_with("htNtJson") {
        return match leaf {
            "htNtJson" => bnd(Consumed, "tariff::decode_htnt (Base64 → UTF-16LE → JSON); raw kept in ZoneSchedule.raw_encoded", ""),
            "WochenTage" => bnd(Consumed, "ZoneSchedule.weekday_rows (token letter → row, .NET weekday order)", "token-to-row rule inferred from data (a, b)"),
            "TarriffZoneDefList" => bnd(Consumed, "ZoneSchedule rows: slot = month0 × 24 + hour (hourly demand adapter)", "other consumers of the grid"),
            "EnergyTariffRanges" | "Tarifname" => bnd(Consumed, "zone_prices / zone_names (zone code = 1-based index)", ""),
            "RangeValues" | "Value" => bnd(Consumed, "zone price = first rate row", "tiers beyond the first row"),
            "Key" | "Range" => bnd(Preserved, "decoded schedule", "tier breakpoints within a zone"),
            "Id" | "$id" => bnd(Preserved, "decoded schedule", "link to catalog range rows unverified"),
            "TimePlusXMinutes" => bnd(Preserved, "ZoneSchedule.time_plus_minutes (not applied, as in the traced hourly adapter)", "application of non-zero offsets"),
            _ => bnd(Consumed, "zone schedule", ""),
        };
    }
    if path.starts_with("tariffConcepts") {
        return match leaf {
            "concept" | "tariffUnit" | "value" | "tariffConcepts" | "tariffRangeValues" => bnd(Consumed, "tariff::apply: FeedIn → export price; Base → fixed/month; Demand → per kW·month; Energy → import price", "Consumption/Generation production concepts not applied"),
            "key" => bnd(Consumed, "tariff::tier_rate breakpoint (inclusive lower bound) by consumption or installed kWp", "inclusive boundary"),
            "range" => bnd(Preserved, "concept tiers", "secondary range parameter"),
            "isExpense" => bnd(Preserved, "concept record", "cash-flow sign rule"),
            _ => bnd(Identity, "concept identity (tariffId/version, concept id)", ""),
        };
    }
    match leaf {
        "zoning" => bnd(Consumed, "tariff::tier_rate weighting (zoning) vs reached band", "binding in non-energy branches"),
        "isNetMetering" | "isDemandTariff" => bnd(Displayed, "tariff details; family decides billing (net metering → monthly offset)", ""),
        "city" | "installationType" | "typeOfCustomer" | "validFrom" => bnd(Displayed, "tariff search/details (selection context)", ""),
        "country" => bnd(Identity, "country join_check", ""),
        "minimumPrice" | "minimumPriceUnitId" | "thresholdType_1" | "thresholdValue_1" | "saldoRollover" | "payoutDuration" | "periodicDuration" => bnd(Preserved, "reported by tariff::apply as not applied", "rule semantics"),
        "isDynamic" => bnd(Preserved, "reported by tariff::apply (no price series)", "dynamic price source"),
        _ => bnd(Preserved, "raw record", ""),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub source: String, pub dataset: String, pub path: String, pub declared: String, pub unit: String, pub evidence: String,
    pub role: Role, pub consumer: String, pub open: String, pub observed: String,
}

/// Read the field reference TSV (header + rows) and join bindings and observed presence.
pub fn report(tsv: &Path, catalog: Option<&Path>) -> Result<Vec<Row>, String> {
    let text = std::fs::read_to_string(tsv).map_err(|e| e.to_string())?;
    let mut presence: BTreeMap<(String, String), String> = BTreeMap::new();
    if let Some(c) = catalog {
        let db = Connection::open_with_flags(c, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        let mut st = db.prepare("SELECT dataset, path, group_concat(class || ':' || n, ' ') FROM presence GROUP BY dataset, path").map_err(|e| e.to_string())?;
        for r in st.query_map(params![], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).map_err(|e| e.to_string())? { let (d, p, s) = r.map_err(|e| e.to_string())?; presence.insert((d, p), s); }
    }
    let mut out = vec![];
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 8 { continue; }
        let b = bind(f[0], f[1], f[2]);
        let observed = presence.get(&(f[1].to_string(), f[2].to_string())).cloned().unwrap_or_else(|| if f[0].starts_with("Online") { "not observed".into() } else { String::new() });
        out.push(Row { source: f[0].trim_start_matches('\u{feff}').into(), dataset: f[1].into(), path: f[2].into(), declared: f[3].into(), unit: f[5].into(), evidence: f[7].into(), role: b.role, consumer: b.consumer, open: b.open, observed });
    }
    Ok(out)
}

pub fn to_tsv(rows: &[Row]) -> String {
    let mut s = String::from("source\tdataset\tfield_path\tdeclared_type\tstored_unit\tevidence\trole\tconsumer\topen_item\tobserved_presence\n");
    for r in rows { s.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{:?}\t{}\t{}\t{}\n", r.source, r.dataset, r.path, r.declared, r.unit, r.evidence, r.role, r.consumer, r.open, r.observed)); }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bindings() {
        assert_eq!(bind("Online JSON", "Online pvinverters", "mppT2_MaxCurrent").role, Role::Consumed);
        assert_eq!(bind("Online JSON", "Online pvmodules", "company.id").role, Role::Identity);
        assert_eq!(bind("Online JSON", "Online demandtariffs", "htNtJson{decoded}.TimePlusXMinutes").role, Role::Preserved);
        assert_eq!(bind("Online JSON", "Online demandtariffs", "htNtJson{decoded}.TarriffZoneDefList").role, Role::Consumed);
        assert_eq!(bind("Legacy SDF", "Products_PVModules", "Voc").role, Role::SchemaReference);
        assert_eq!(bind("Online JSON", "Online batterysystems", "battery.ratedVoltage").role, Role::Identity);
    }
}
