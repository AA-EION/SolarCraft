// SPDX-License-Identifier: GPL-3.0-or-later
//! Catalog operations used by the shells, CLI and MCP: import a catalog backup, search with
//! companies and countries, details with history, pin a resolved revision into a project, and
//! apply catalog tariffs to the project's billing.
use crate::{e, open, save, Result};
use serde_json::{json, Value};
use solarcraft_catalog as cat;
use std::path::{Path, PathBuf};

/// `SOLARCRAFT_CATALOG` or `catalog.sqlite` next to the user library.
pub fn default_catalog() -> PathBuf {
    std::env::var("SOLARCRAFT_CATALOG").map(PathBuf::from).unwrap_or_else(|_| crate::userlib::default_path().with_file_name("catalog.sqlite"))
}

fn ce(m: String) -> crate::AppError { e("CATALOG", m) }

/// Equipment kind in the project library for a catalog family.
pub fn kind_of(family: &str) -> Option<&'static str> {
    match family { "module" => Some("module"), "pvinverter" => Some("inverter"), "batterysystem" => Some("battery"), "batteryinverter" => Some("battery_inverter"), "generator" => Some("generator"), _ => None }
}

pub fn audit(source: &Path) -> Result<Value> { Ok(serde_json::to_value(cat::audit(source).map_err(ce)?).unwrap_or_default()) }
pub fn import(source: &Path, catalog: &Path) -> Result<Value> { Ok(serde_json::to_value(cat::import(source, catalog).map_err(ce)?).unwrap_or_default()) }
pub fn search(catalog: &Path, family: Option<&str>, query: &str, available_only: bool, limit: usize) -> Result<Value> {
    Ok(json!({"hits": cat::search(catalog, family, query, available_only, limit).map_err(ce)?}))
}
pub fn details(catalog: &Path, family: &str, id: &str, version: Option<i64>) -> Result<Value> { cat::details(catalog, family, id, version).map_err(ce) }
pub fn history(catalog: &Path) -> Result<Value> { cat::history(catalog, 50).map_err(ce) }

/// Pin a resolved revision (latest when `version` is None) into the project library.
pub fn pin(catalog: &Path, project: &Path, family: &str, id: &str, version: Option<i64>) -> Result<Value> {
    let kind = kind_of(family).ok_or_else(|| e("INVALID_ARGUMENT", format!("family '{family}' is not equipment; tariffs use catalog_apply_tariff")))?;
    let (mut rec, cref, _) = cat::resolve(catalog, family, id, version).map_err(ce)?;
    rec["catalog_ref"] = serde_json::to_value(&cref).unwrap_or_default();
    let rec = crate::plant_ops::canonical_equipment(kind, &rec)?;
    let (mut s, mut p) = open(project)?;
    crate::plant_ops::insert(&mut p, kind, rec.clone())?;
    solarcraft_model::derive::derive(&mut p);
    save(&mut s, &p, &format!("pin catalog {family} {id} v{}", cref.version))?;
    Ok(json!({"kind": kind, "record": rec, "catalog_ref": cref}))
}

fn tariff_family(f: &str) -> Option<cat::tariff::TariffFamily> {
    match f { "feedintariff" => Some(cat::tariff::TariffFamily::FeedIn), "netmeteringtariff" => Some(cat::tariff::TariffFamily::NetMetering), "demandtariff" => Some(cat::tariff::TariffFamily::Demand), _ => None }
}

/// Apply catalog tariffs to the project: `import` (demand or net-metering family) sets the
/// from-grid side, `export` (feed-in family) the compensation. Tiers use the project's annual
/// consumption and installed kWp.
pub fn apply_tariff(catalog: &Path, project: &Path, import: Option<(&str, &str, Option<i64>)>, export: Option<(&str, &str, Option<i64>)>) -> Result<Value> {
    let (mut s, mut p) = open(project)?;
    let kwp: f64 = p.arrays.iter().map(|a| a.target_kwp.value).sum();
    let ctx = cat::tariff::TariffContext { annual_consumption_kwh: p.loads.annual_kwh, installed_kwp: kwp };
    let mut t = p.tariff.clone();
    let mut out = vec![];
    for (side, sel) in [("import", import), ("export", export)] {
        let Some((family, id, ver)) = sel else { continue };
        let fam = tariff_family(family).ok_or_else(|| e("INVALID_ARGUMENT", format!("'{family}' is not a tariff family")))?;
        if (side == "export") != (fam == cat::tariff::TariffFamily::FeedIn) { return Err(e("INVALID_ARGUMENT", format!("{side} side needs a {} tariff", if side == "export" { "feed-in" } else { "demand or net-metering" }))); }
        let d = cat::details(catalog, family, id, ver).map_err(ce)?;
        let cref = solarcraft_model::plant::CatalogRef { family: family.into(), id: id.into(), version: d["version"].as_i64().unwrap_or(0), source_sha256: d["source_sha256"].as_str().unwrap_or("").into(), dataset: d["dataset"].as_str().unwrap_or("").into() };
        let cv = cat::tariff::apply(&mut t, fam, &d["raw"], ctx, cref.clone());
        out.push(json!({"side": side, "catalog_ref": cref, "notes": cv.notes, "not_applied": cv.unsupported}));
    }
    if t.currency.is_empty() || t.currency == "EUR" { t.currency = "source currency (unverified)".into(); }
    p.tariff = t.clone();
    save(&mut s, &p, "apply catalog tariff")?;
    Ok(json!({"tariff": t, "applied": out}))
}

/// Coverage report of the field reference against the importer and the application.
pub fn coverage(tsv: &Path, catalog: Option<&Path>, out: Option<&Path>) -> Result<Value> {
    let rows = cat::coverage::report(tsv, catalog).map_err(ce)?;
    if let Some(o) = out { std::fs::write(o, cat::coverage::to_tsv(&rows)).map_err(|x| e("FILE_WRITE", x))?; }
    let mut by: std::collections::BTreeMap<String, std::collections::BTreeMap<String, usize>> = Default::default();
    for r in &rows { *by.entry(r.source.clone()).or_default().entry(format!("{:?}", r.role)).or_default() += 1; }
    let open: Vec<Value> = rows.iter().filter(|r| !r.open.is_empty() && r.source.starts_with("Online")).map(|r| json!({"dataset": r.dataset, "path": r.path, "role": r.role, "open": r.open})).collect();
    Ok(json!({"entries": rows.len(), "by_source_and_role": by, "open_items": open.len(), "open": open, "written": out.map(|o| o.display().to_string())}))
}
