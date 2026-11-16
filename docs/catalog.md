# Equipment and tariff catalog

SolarCraft imports a catalog backup (SQLite, exporter schema) into its own catalog database. The imported records then drive layout, electrical design, storage, generator and billing calculations.

The repository contains **no catalog data**. Tests build small backups from fictional records. Your own backup is imported locally and stays on your machine.

## Use

```bash
solarcraft call catalog_audit '{"source": "PVSol catalog backup.sqlite"}'
```

```bash
solarcraft call catalog_import '{"source": "PVSol catalog backup.sqlite"}'
```

```bash
solarcraft call catalog_search '{"family": "pvinverter", "query": "5 kW 2 MPPT"}'
```

```bash
solarcraft call catalog_pin '{"file": "casa.solarcraft", "family": "module", "id": "<id>", "version": 3}'
```

```bash
solarcraft call catalog_apply_tariff '{"file": "casa.solarcraft", "import": {"family": "demandtariff", "id": "<id>"}, "export": {"family": "feedintariff", "id": "<id>"}}'
```

```bash
solarcraft call catalog_coverage '{"tsv": "SolarCraft-PVSol-Catalog-Fields.tsv", "output": "coverage.tsv"}'
```

The catalog database lives at `catalog.sqlite` next to the user library, or wherever `SOLARCRAFT_CATALOG` points. Both desktop apps (Electrical page → Catalog) and the MCP tools `catalog_search`, `catalog_details`, `catalog_pin` and `catalog_apply_tariff` use the same API.

## What the importer keeps

| Item | How it is kept |
|---|---|
| Records | `records.dataset_id` is joined to `datasets.id`. `record_json` is authoritative: case-sensitive names, nesting and presence. Every revision is kept as an entity under (family, id, version), together with the raw JSON, the source file SHA‑256, the dataset and its row number. |
| Normalised records | SolarCraft records are kept separately, with the unit of every value and any issues: missing value, sentinel −1, clamped, assumed, or unverified. |
| Presence | Counted per dataset and exact path as missing, null, zero, false, sentinel, empty string or value. Arrays are written `[]`; decoded schedules are written `{decoded}`. |
| Joins | Company projections are checked against `companyId` and the companies table. Countries are checked against `countryId`. Battery systems are checked against their linked battery revision, which is authoritative over the embedded snapshot. Modules are checked against their linked optimizer revision. Each join is recorded as `ok`, `mismatch`, `revision_missing`, `missing_target` or an explicit `fallback_embedded`. |
| History | The changelog (created / new version) and database releases and publications. Equipment details show the entity's events and the release published around it. |
| API responses | Archived verbatim; SHA‑256 and length are verified. |
| Projects | A project pins the resolved revision: its library copy carries a `catalog_ref` with family, id, version, source SHA‑256 and dataset. Ids are scoped as `cat-<id>-v<version>`, so several revisions can coexist. |

## Owner backup (2026-11-04 export): audit and import

| Check | Expected | Observed |
|---|---:|---:|
| Online datasets | 16 | 16 |
| Records | 165,166 | 165,166 |
| Top-level field occurrences (union of dataset columns) | 426 | 426 |
| Nested paths | 337 | 337 (763 paths observed − 426) |
| SQLite storage columns | 28 | 28 |
| Archived API responses (SHA-256 verified) | 217 | 217 (0 mismatches) |
| Local catalog rows (`local_schema`) | 0 | 0 |
| Non-empty `htNtJson` schedules decoded | 60 | 60 |

Revisions and distinct ids imported:

| Family | Revisions | Distinct ids |
|---|---:|---:|
| Modules | 43,327 | 31,355 |
| PV inverters | 15,936 | 9,432 |
| Batteries | 2,992 | 2,239 |
| Battery systems | 27,969 | 18,749 |
| Battery inverters | 173 | 144 |
| Optimizers | 150 | 79 |
| Generators | 31 | 16 |
| EVs and stations | 2,353 | 1,820 |
| Feed-in tariffs | 1,225 | 857 |
| Net-metering tariffs | 21 | 21 |
| Demand tariffs | 25 | 24 |
| Companies | 1,721 | 1,667 |
| Countries | 250 | 250 |

There were also 68,585 changelog events and 408 release rows.

Every join resolved, including the linked battery revision for all 27,969 battery systems and the linked optimizer for all 46 modules that have one. No embedded snapshot differs from its target.

## Tariff schedules

`htNtJson` is decoded in three steps: Base64, then UTF‑16LE (optional BOM), then JSON. The encoded original is kept in `ZoneSchedule.raw_encoded`.

What the 60 decoded schedules contain:

- **Weekday tokens.** `WochenTage` holds seven tokens in .NET weekday order, Sunday first. A token is a row letter: `a` is row 0, `b` is row 1. 57 schedules use `a` for all seven days. 3 use `b` on Sunday and Saturday.
- **Zone grid.** `TarriffZoneDefList` holds rows of 288 zone codes, one per month and hour.
- **Zone prices.** The zone code is a 1-based index into `EnergyTariffRanges`; in every schedule the highest code equals the number of rate groups.
- **Minute offset.** `TimePlusXMinutes` is 0, 30 or missing.

Billing follows the traced hourly demand adapter:

- the weekday picks the row;
- the slot is the zero-based month × 24 + hour;
- the zone price is the first rate row of the matching `EnergyTariffRanges` entry;
- the minute offset is kept but not applied (the adapter does not read it).

`crates/catalog/src/tariff.rs` and `crates/app/tests/catalog_import.rs::zone_billing_boundaries` test this at the hour-16/17 and 19/20 boundaries, on weekend rows and at the year end.

Tariff concepts are applied by tariff family:

| Family | Concept | Becomes |
|---|---|---|
| Production (feed-in) | 0 FeedIn | Export price |
| Demand and net metering | 0 Base | Fixed charge per month |
| Demand and net metering | 1 Demand | Charge per kW·month |
| Demand and net metering | 2 Energy | Import price |

Tiers are selected by installed kWp (feed-in) or annual consumption (energy), with `zoning` choosing between a weighted price and the price of the band reached. Rules that are not applied are reported by `catalog_apply_tariff`: production Consumption/Generation concepts, minimum price, thresholds, rollover, payout duration and dynamic prices.

The backup records carry no currency. Billing uses the source numbers as they are, and the tariff currency is labelled "source currency (unverified)".

## Field coverage

`catalog_coverage` joins all 1,181 entries of the field reference with their SolarCraft binding and with the presence counts observed in the import. It writes one row per source, dataset and exact path.

| Source | Consumed | Validated | Displayed | Identity / provenance | Preserved (open item stated) | Legacy schema reference |
|---|---:|---:|---:|---:|---:|---:|
| Online JSON (763) | 152 | 17 | 192 | 199 | 203 | |
| Backup SQLite (28) | | | | 28 | | |
| Legacy SDF (390) | | | | | | 390 |

Every one of the 763 online paths was observed in the backup, and every entry has a binding. Each *preserved* field keeps its original value in the raw record, and in the normalised record's `unverified` map where it belongs to equipment.

### Open items

Grouped by what remains to verify. Paths are `dataset:field_path`.

| Role | Open item | Fields |
|---|---|---|
| Consumed | Consumption/Generation production concepts not applied | demandtariffs:tariffConcepts, demandtariffs:tariffConcepts[], demandtariffs:tariffConcepts[].concept, demandtariffs:tariffConcepts[].tariffRangeValues, demandtariffs:tariffConcepts[].tariffRangeValues[], demandtariffs:tariffConcepts[].tariffRangeValues[].value (+15) |
| Consumed | binding in non-energy branches | demandtariffs:zoning, feedintariffs:zoning, netmeteringtariffs:zoning |
| Consumed | cell/module scaling per chemistry | batteries:internalResistance |
| Consumed | discharge-side behaviour | batteryinverters:batt_MaxCurrent |
| Consumed | exact 20 % boundary | pvinverters:matchingEtaMPP100, pvinverters:matchingEtaMPP20 |
| Consumed | exact formula in the selected model | pvinverters:changeOfEtaDependentOnVoltage, pvinverters:nominalVoltageDC |
| Consumed | fitted voltage profile per chemistry | batteries:batteryType |
| Consumed | fuel volume unit | generators:fuelConsumptionAt0, generators:fuelConsumptionAt100, generators:fuelConsumptionAt25, generators:fuelConsumptionAt50, generators:fuelConsumptionAt75 |
| Consumed | full DoD-dependent ageing | batteries:lifeCycles20, batteries:lifeCycles40, batteries:lifeCycles60, batteries:lifeCycles80 |
| Consumed | inclusive boundary | demandtariffs:tariffConcepts[].tariffRangeValues[].key, feedintariffs:tariffConcepts[].tariffRangeValues[].key, netmeteringtariffs:tariffConcepts[].tariffRangeValues[].key |
| Consumed | load reference and direction for converters | batteryinverters:e10, batteryinverters:e100, batteryinverters:e20, batteryinverters:e30, batteryinverters:e5, batteryinverters:e50 (+15) |
| Consumed | load reference of the curve (DC vs AC rating) | pvinverters:dC_PowerRating |
| Consumed | other consumers of the grid | demandtariffs:htNtJson{decoded}.TarriffZoneDefList, demandtariffs:htNtJson{decoded}.TarriffZoneDefList[], demandtariffs:htNtJson{decoded}.TarriffZoneDefList[][], feedintariffs:htNtJson{decoded}.TarriffZoneDefList, feedintariffs:htNtJson{decoded}.TarriffZoneDefList[], feedintariffs:htNtJson{decoded}.TarriffZoneDefList[][] (+3) |
| Consumed | power factor | generators:aC_RatedPower |
| Consumed | power side at each consumer | pvinverters:feedInFrom |
| Consumed | tiers beyond the first row | demandtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues, demandtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[], demandtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Value, feedintariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues, feedintariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[], feedintariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Value (+3) |
| Consumed | token-to-row rule inferred from data (a, b) | demandtariffs:htNtJson{decoded}.WochenTage, demandtariffs:htNtJson{decoded}.WochenTage[], feedintariffs:htNtJson{decoded}.WochenTage, feedintariffs:htNtJson{decoded}.WochenTage[], netmeteringtariffs:htNtJson{decoded}.WochenTage, netmeteringtariffs:htNtJson{decoded}.WochenTage[] |
| Consumed | topology exceptions beyond the half-cell rule | pvmodules:cellCount, pvmodules:isHalfCellModule |
| Displayed | L-N/L-L convention | pvinverters:voltageAC |
| Displayed | array kWp uses Vmp×Imp | pvmodules:specifiedOutputPower |
| Displayed | mapping of 2 vs 3 to a specific port | batterysystems:coupling |
| Displayed | model selection by cell type | pvmodules:cellType |
| Displayed | which state uses it vs night consumption | pvinverters:standbyConsumption |
| Preserved | access semantics | batteries:dbUserId, batteryinverters:dbUserId, batterysystems:battery.dbUserId, batterysystems:dbUserId, carsandstations:dbUserId, companies:dbUserId (+8) |
| Preserved | angular equation of the reflection model | pvmodules:iam |
| Preserved | application of non-zero offsets | demandtariffs:htNtJson{decoded}.TimePlusXMinutes, feedintariffs:htNtJson{decoded}.TimePlusXMinutes, netmeteringtariffs:htNtJson{decoded}.TimePlusXMinutes |
| Preserved | bifacial rear irradiance not modelled (SIM-009) | pvmodules:bifacialityFactor |
| Preserved | boost/full/equalisation modes not implemented | batterysystems:boostDuration, batterysystems:cycleEqualisation, batterysystems:cycleFull, batterysystems:durationEqualisation, batterysystems:durationFull, batterysystems:socBoostEnd (+4) |
| Preserved | bypass-diode partial shading not modelled (ELEC-013) | pvmodules:diodeCount, pvmodules:lossVoltagePerBypassDiode |
| Preserved | cash-flow sign rule | demandtariffs:tariffConcepts[].isExpense, feedintariffs:tariffConcepts[].isExpense, netmeteringtariffs:tariffConcepts[].isExpense |
| Preserved | dynamic price source | demandtariffs:isDynamic, feedintariffs:isDynamic, netmeteringtariffs:isDynamic |
| Preserved | interpolator below 5 % | batteryinverters:e0, batterysystems:e0, poweroptimizers:e0, pvinverters:e0 |
| Preserved | link to catalog range rows unverified | demandtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Id, feedintariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Id, netmeteringtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Id |
| Preserved | media validation | batteries:graphic, batteryinverters:graphic, batterysystems:battery.graphic, batterysystems:graphic, carsandstations:graphic, companies:graphic (+8) |
| Preserved | no corrupt records observed | batteries:isCorrupt, batteryinverters:isCorrupt, batterysystems:battery.isCorrupt, batterysystems:isCorrupt, carsandstations:isCorrupt, generators:isCorrupt (+4) |
| Preserved | nominal AC reference vs maxAC_Power | pvinverters:aC_PowerRating |
| Preserved | off-grid cluster ratings not modelled | batteryinverters:aC_30MinPower, batteryinverters:aC_60MinPower, batteryinverters:aC_MaxGenCurrent, batteryinverters:aC_RatedVoltage, batteryinverters:batt_RatedInputVoltage, batteryinverters:consumption_WithoutLoad (+3) |
| Preserved | optimizer modelling not implemented | pvmodules:powerOptimizer.e0, pvmodules:powerOptimizer.e10, pvmodules:powerOptimizer.e100, pvmodules:powerOptimizer.e20, pvmodules:powerOptimizer.e30, pvmodules:powerOptimizer.e5 (+15) |
| Preserved | optimizer/EV not simulated | carsandstations:akkuKapazitaet, carsandstations:anzahlSitzplaetze, carsandstations:canDischarge, carsandstations:entLadeLeistung, carsandstations:hoechstgeschwindigkeit, carsandstations:ladeLeistung (+19) |
| Preserved | optimizers not simulated | poweroptimizers:e10, poweroptimizers:e100, poweroptimizers:e20, poweroptimizers:e30, poweroptimizers:e5, poweroptimizers:e50 (+1) |
| Preserved | physical representation only | pvmodules:widthOfFrame |
| Preserved | rate-dependent capacity not used in dispatch | batteries:capacityAt0167, batteries:capacityAt05, batteries:capacityAt1, batteries:capacityAt100, batteries:capacityAt5 |
| Preserved | rule semantics | demandtariffs:minimumPrice, demandtariffs:minimumPriceUnitId, demandtariffs:payoutDuration, demandtariffs:periodicDuration, demandtariffs:saldoRollover, demandtariffs:thresholdType_1 (+15) |
| Preserved | secondary range parameter | demandtariffs:tariffConcepts[].tariffRangeValues[].range, feedintariffs:tariffConcepts[].tariffRangeValues[].range, netmeteringtariffs:tariffConcepts[].tariffRangeValues[].range |
| Preserved | selection of the part-load model | pvmodules:standardPartLoadOperation |
| Preserved | tier breakpoints within a zone | demandtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Key, demandtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Range, feedintariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Key, feedintariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Range, netmeteringtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Key, netmeteringtariffs:htNtJson{decoded}.EnergyTariffRanges[].RangeValues[].Range |
| Preserved | true/false orientation for shading | pvmodules:standardCellConnection |
| Validated | not used as a fit anchor (CEC fit is STC-based) | pvmodules:g2, pvmodules:impp2, pvmodules:isc2, pvmodules:umpp2, pvmodules:uoc2 |
