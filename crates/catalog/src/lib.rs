// SPDX-License-Identifier: GPL-3.0-or-later
//! SolarCraft equipment and tariff catalog.
//!
//! Imports a catalog backup (SQLite with `datasets`, `records`, `api_responses`, …) into a
//! SolarCraft catalog database. Every revision of every entity is kept with its original JSON
//! (authoritative for field names, nesting and presence), the source provenance, the
//! normalised SolarCraft record with explicit units, and the issues found while normalising.
//! Identities are scoped by entity family; linked battery and optimizer revisions and embedded
//! company/country projections are checked against their targets.
pub mod coverage;
pub mod normalize;
pub mod tariff;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;
fn err<E: std::fmt::Display>(e: E) -> String { e.to_string() }

/// Entity family for a dataset name (the identity scope of `id`).
pub fn family(dataset: &str) -> &'static str {
    match dataset.trim_start_matches("Online ").trim_start_matches("Local ") {
        "companies" => "company", "pvmodules" => "module", "pvinverters" => "pvinverter", "batteries" => "battery",
        "batterysystems" => "batterysystem", "batteryinverters" => "batteryinverter", "poweroptimizers" => "optimizer",
        "generators" => "generator", "carsandstations" => "ev", "feedintariffs" => "feedintariff", "netmeteringtariffs" => "netmeteringtariff",
        "demandtariffs" => "demandtariff", "countries" => "country", "database versions" => "release", "publish metadata" => "publish",
        "changelog" => "changelog", _ => "other",
    }
}

/// Families whose rows are entity revisions (id + version).
pub fn versioned(f: &str) -> bool { !matches!(f, "country" | "release" | "publish" | "changelog" | "other") }

#[derive(Debug, Clone, Serialize, Default)]
pub struct Audit {
    pub tables: Vec<(String, usize, i64)>,
    pub storage_columns: usize,
    pub datasets: Vec<(String, String, i64, usize)>,
    pub records: i64,
    pub top_level_field_occurrences: usize,
    pub api_responses: i64,
    pub api_response_bytes: i64,
    pub local_schema_rows: i64,
    pub backup_info: Vec<(String, String)>,
    pub nonempty_htnt: usize,
    pub htnt_decoded: usize,
}

fn open_ro(p: &Path) -> Result<Connection> { Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err) }

/// Count what the backup contains.
pub fn audit(backup: &Path) -> Result<Audit> {
    let c = open_ro(backup)?;
    let mut a = Audit::default();
    let tables: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY rowid").map_err(err)?.query_map([], |r| r.get(0)).map_err(err)?.filter_map(|x| x.ok()).collect();
    for t in &tables {
        let cols = c.prepare(&format!("PRAGMA table_info('{t}')")).map_err(err)?.query_map([], |_| Ok(())).map_err(err)?.count();
        let n: i64 = c.query_row(&format!("SELECT COUNT(*) FROM '{t}'"), [], |r| r.get(0)).map_err(err)?;
        a.storage_columns += cols;
        a.tables.push((t.clone(), cols, n));
    }
    for (src, name, rows, cols) in c.prepare("SELECT source, name, row_count, columns_json FROM datasets ORDER BY id").map_err(err)?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))).map_err(err)?.filter_map(|x| x.ok()) {
        let n = serde_json::from_str::<Vec<String>>(&cols).map(|v| v.len()).unwrap_or(0);
        a.top_level_field_occurrences += n;
        a.records += rows;
        a.datasets.push((src, name, rows, n));
    }
    a.local_schema_rows = c.query_row("SELECT COUNT(*) FROM local_schema", [], |r| r.get(0)).unwrap_or(0);
    let (n, b): (i64, Option<i64>) = c.query_row("SELECT COUNT(*), SUM(body_bytes) FROM api_responses", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
    a.api_responses = n; a.api_response_bytes = b.unwrap_or(0);
    a.backup_info = c.prepare("SELECT key, value FROM backup_info").map_err(err)?.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?.filter_map(|x| x.ok()).collect();
    let mut st = c.prepare("SELECT r.record_json FROM records r JOIN datasets d ON d.id = r.dataset_id WHERE d.name LIKE '%tariffs'").map_err(err)?;
    for j in st.query_map([], |r| r.get::<_, String>(0)).map_err(err)?.filter_map(|x| x.ok()) {
        let v: Value = serde_json::from_str(&j).unwrap_or(Value::Null);
        if let Some(h) = v["htNtJson"].as_str().filter(|s| !s.trim().is_empty()) {
            a.nonempty_htnt += 1;
            if tariff::decode_htnt(h).and_then(|d| tariff::zone_schedule(&d, h)).is_ok() { a.htnt_decoded += 1; }
        }
    }
    Ok(a)
}

/// Value class of a field occurrence, distinguishing missing, null, zero, false and sentinels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Presence { Missing, Null, Zero, False, Sentinel, EmptyString, Value }

/// Sentinel values documented for a field (−1 = point missing in the cycle-life characteristic).
pub fn is_sentinel(path: &str, v: &Value) -> bool { path.rsplit('.').next().is_some_and(|leaf| leaf.starts_with("lifeCycles")) && v.as_f64() == Some(-1.0) }

fn classify(path: &str, v: &Value) -> Presence {
    match v {
        Value::Null => Presence::Null,
        Value::Bool(false) => Presence::False,
        Value::Number(n) if is_sentinel(path, v) => { let _ = n; Presence::Sentinel }
        Value::Number(n) if n.as_f64() == Some(0.0) => Presence::Zero,
        Value::String(s) if s.is_empty() => Presence::EmptyString,
        _ => Presence::Value,
    }
}

/// Walk a record and count every path (`a.b`, arrays as `[]`, decoded schedules as `{decoded}`).
pub fn walk(path: &str, v: &Value, out: &mut BTreeMap<String, BTreeMap<Presence, u64>>) {
    *out.entry(path.to_string()).or_default().entry(classify(path, v)).or_default() += 1;
    match v {
        Value::Object(m) => for (k, x) in m { walk(&if path.is_empty() { k.clone() } else { format!("{path}.{k}") }, x, out); },
        Value::Array(a) => for x in a { walk(&format!("{path}[]"), x, out); },
        Value::String(s) if path.ends_with("htNtJson") && !s.trim().is_empty() => if let Ok(d) = tariff::decode_htnt(s) { walk(&format!("{path}{{decoded}}"), &d, out); },
        _ => {}
    }
}

const SCHEMA: &str = "PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS source(id INTEGER PRIMARY KEY, kind TEXT NOT NULL, file_name TEXT NOT NULL, sha256 TEXT NOT NULL, info_json TEXT NOT NULL, imported_unix INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS entity(family TEXT NOT NULL, id TEXT NOT NULL, version INTEGER NOT NULL, name TEXT, company_id TEXT, country_id TEXT, is_available INTEGER, published_at TEXT,
  dataset TEXT NOT NULL, row_number INTEGER NOT NULL, source_id INTEGER NOT NULL, raw_json TEXT NOT NULL, normalized_json TEXT, issues_json TEXT, search TEXT,
  PRIMARY KEY(family, id, version));
CREATE INDEX IF NOT EXISTS entity_search ON entity(family, search);
CREATE TABLE IF NOT EXISTS changelog(event_id INTEGER, family_code INTEGER, guid TEXT, version INTEGER, entry_tag INTEGER, publish_date TEXT, name TEXT, company_name TEXT, raw_json TEXT, PRIMARY KEY(event_id));
CREATE INDEX IF NOT EXISTS changelog_guid ON changelog(guid);
CREATE TABLE IF NOT EXISTS release(kind TEXT, version TEXT, major INTEGER, minor INTEGER, at TEXT, comment TEXT, raw_json TEXT);
CREATE TABLE IF NOT EXISTS api_response(id INTEGER, source_id INTEGER, route TEXT, body_bytes INTEGER, sha256 TEXT, body BLOB, sha_ok INTEGER, PRIMARY KEY(source_id, id));
CREATE TABLE IF NOT EXISTS backup_record(source_id INTEGER NOT NULL, dataset TEXT NOT NULL, row_number INTEGER NOT NULL, raw_json TEXT NOT NULL, PRIMARY KEY(source_id, dataset, row_number));
CREATE TABLE IF NOT EXISTS source_presence(source_id INTEGER NOT NULL, dataset TEXT NOT NULL, path TEXT NOT NULL, class TEXT NOT NULL, n INTEGER NOT NULL, PRIMARY KEY(source_id, dataset, path, class));
CREATE TABLE IF NOT EXISTS export_log(source_id INTEGER, src TEXT, route_or_table TEXT, dataset_name TEXT, exported INTEGER, catalog_count INTEGER, status TEXT);
CREATE TABLE IF NOT EXISTS presence(dataset TEXT, path TEXT, class TEXT, n INTEGER, PRIMARY KEY(dataset, path, class));
CREATE TABLE IF NOT EXISTS join_check(family TEXT, id TEXT, version INTEGER, link TEXT, status TEXT, detail TEXT);";

#[derive(Debug, Clone, Serialize, Default)]
pub struct ImportReport {
    pub source_sha256: String,
    pub audit: Audit,
    pub entities_by_family: BTreeMap<String, i64>,
    pub distinct_ids_by_family: BTreeMap<String, i64>,
    pub normalized_by_family: BTreeMap<String, i64>,
    pub normalize_failures_by_family: BTreeMap<String, i64>,
    pub changelog_events: i64,
    pub releases: i64,
    pub api_responses: i64,
    pub api_sha_mismatches: i64,
    pub joins: BTreeMap<String, BTreeMap<String, i64>>,
    pub paths_observed: usize,
}

fn migrate_archive(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let id_primary_key: i64 = tx.query_row(
        "SELECT pk FROM pragma_table_info('api_response') WHERE name='id'", [], |r| r.get(0),
    ).map_err(err)?;
    if id_primary_key == 1 {
        tx.execute_batch(
            "CREATE TABLE api_response_v2(id INTEGER, source_id INTEGER, route TEXT, body_bytes INTEGER,
                 sha256 TEXT, body BLOB, sha_ok INTEGER, PRIMARY KEY(source_id, id));
             INSERT INTO api_response_v2 SELECT * FROM api_response;
             DROP TABLE api_response;
             ALTER TABLE api_response_v2 RENAME TO api_response;
             INSERT INTO backup_record SELECT source_id, dataset, row_number, raw_json FROM entity;
             INSERT INTO source_presence
                 SELECT recent.id, p.dataset, p.path, p.class, p.n
                 FROM presence p CROSS JOIN (SELECT MAX(id) AS id FROM source) recent
                 WHERE recent.id IS NOT NULL;",
        ).map_err(err)?;
    }
    Ok(())
}

/// Accumulate backups by source hash. Preserve every raw row and retain the first
/// canonical record when two archives contain the same family, ID and revision.
pub fn import(backup: &Path, catalog: &Path) -> Result<ImportReport> {
    let src = open_ro(backup)?;
    let bytes = std::fs::read(backup).map_err(err)?;
    let sha = solarcraft_store::sha256_hex(&bytes);
    drop(bytes);
    let mut rep = ImportReport { source_sha256: sha.clone(), audit: audit(backup)?, ..Default::default() };
    let mut db = Connection::open(catalog).map_err(err)?;
    db.execute_batch(SCHEMA).map_err(err)?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(err)?;
    migrate_archive(&tx)?;
    let existing_source = tx.query_row(
        "SELECT id FROM source WHERE sha256=?1", [&sha], |r| r.get(0),
    ).optional().map_err(err)?;
    let source_id = match existing_source {
        Some(id) => id,
        None => {
            let file = backup.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
            let info = json!({"backup_info": rep.audit.backup_info, "file": file});
            tx.execute(
                "INSERT INTO source(kind, file_name, sha256, info_json, imported_unix)
                 VALUES('backup-sqlite', ?1, ?2, ?3, strftime('%s','now'))",
                params![file.as_ref(), sha, info.to_string()],
            ).map_err(err)?;
            tx.last_insert_rowid()
        }
    };
    for table in ["backup_record", "source_presence", "api_response", "export_log"] {
        tx.execute(&format!("DELETE FROM {table} WHERE source_id=?1"), [source_id]).map_err(err)?;
    }

    // 1. Raw records: join records.dataset_id to datasets.id; record_json is authoritative.
    let mut rows: Vec<(String, i64, String)> = vec![];
    {
        let mut st = src.prepare("SELECT d.name, r.row_number, r.record_json FROM records r JOIN datasets d ON d.id = r.dataset_id ORDER BY d.id, r.row_number").map_err(err)?;
        for x in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))).map_err(err)? { rows.push(x.map_err(err)?); }
    }
    let mut presence: HashMap<String, BTreeMap<String, BTreeMap<Presence, u64>>> = HashMap::new();
    let mut parsed: Vec<(String, i64, Value, String)> = Vec::with_capacity(rows.len());
    for (ds, rn, j) in rows {
        let v: Value = serde_json::from_str(&j).map_err(|e| format!("{ds} row {rn}: {e}"))?;
        let mut m = BTreeMap::new();
        walk("", &v, &mut m);
        let agg = presence.entry(ds.clone()).or_default();
        for (p, c) in m { if p.is_empty() { continue; } for (k, n) in c { *agg.entry(p.clone()).or_default().entry(k).or_default() += n; } }
        parsed.push((ds, rn, v, j));
    }
    // Lookup tables for joins: companies and countries, batteries and optimizers by (id, version).
    let mut by_key: HashMap<(&'static str, String, i64), usize> = HashMap::new();
    let mut latest: HashMap<(&'static str, String), i64> = HashMap::new();
    for (i, (ds, _, v, _)) in parsed.iter().enumerate() {
        let f = family(ds);
        if let (Some(id), true) = (v["id"].as_str(), versioned(f) || f == "country") {
            let ver = v["version"].as_i64().unwrap_or(0);
            by_key.insert((f, id.to_string(), ver), i);
            let e = latest.entry((f, id.to_string())).or_insert(ver);
            *e = (*e).max(ver);
        }
    }
    let lookup = |f: &'static str, id: &str, ver: Option<i64>| -> Option<&Value> {
        let ver = ver.or_else(|| latest.get(&(f, id.to_string())).copied())?;
        by_key.get(&(f, id.to_string(), ver)).map(|&i| &parsed[i].2)
    };
    let mut joins: Vec<(String, String, i64, String, String, String)> = vec![];
    {
        let mut ins_raw = tx.prepare("INSERT INTO backup_record VALUES(?1,?2,?3,?4)").map_err(err)?;
        let mut ins = tx.prepare("INSERT OR IGNORE INTO entity(family,id,version,name,company_id,country_id,is_available,published_at,dataset,row_number,source_id,raw_json,normalized_json,issues_json,search) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)").map_err(err)?;
        let mut ins_cl = tx.prepare("INSERT OR IGNORE INTO changelog VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)").map_err(err)?;
        let mut ins_rel = tx.prepare("INSERT INTO release SELECT ?1,?2,?3,?4,?5,?6,?7 WHERE NOT EXISTS (SELECT 1 FROM release WHERE kind=?1 AND raw_json=?7)").map_err(err)?;
        for (ds, rn, v, raw) in &parsed {
            ins_raw.execute(params![source_id, ds, rn, raw]).map_err(err)?;
            let f = family(ds);
            match f {
                "changelog" => { ins_cl.execute(params![v["id"].as_i64(), v["dbEntryTypeEnum"].as_i64(), v["guid"].as_str(), v["version"].as_i64(), v["entryTag"].as_i64(), v["publishDate"].as_str(), v["dbEntryName"].as_str(), v["companyName"].as_str(), raw]).map_err(err)?; rep.changelog_events += 1; continue; }
                "release" => { ins_rel.execute(params!["database_version", v["version"].as_str(), v["majorVersion"].as_i64(), v["minorVersion"].as_i64(), v["versionChangedAt"].as_str(), v["comment"].as_str(), raw]).map_err(err)?; rep.releases += 1; continue; }
                "publish" => { ins_rel.execute(params!["publish", v["databaseVersion"].as_str(), v["majorVersion"].as_i64(), v["minorVersion"].as_i64(), v["publishDate"].as_str(), Option::<String>::None, raw]).map_err(err)?; rep.releases += 1; continue; }
                _ => {}
            }
            let id = v["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("row-{rn}"));
            let ver = v["version"].as_i64().unwrap_or(0);
            // Joins: company projection, country projection, linked battery and optimizer revisions.
            let mut check = |link: &str, status: &str, detail: String| joins.push((f.to_string(), id.clone(), ver, link.to_string(), status.to_string(), detail));
            if let Some(cid) = v["companyId"].as_str() {
                match (&v["company"], lookup("company", cid, None)) {
                    (Value::Object(p), Some(_)) if p.get("id").and_then(Value::as_str) != Some(cid) => check("company", "mismatch", format!("embedded company.id {:?} ≠ companyId {cid}", p.get("id"))),
                    (Value::Object(p), Some(_)) => { let pv = p.get("version").and_then(Value::as_i64); if pv.is_some_and(|pv| lookup("company", cid, Some(pv)).is_none()) { check("company", "revision_missing", format!("company {cid} v{pv:?} not in companies")); } else { check("company", "ok", String::new()); } }
                    (_, None) => check("company", "missing_target", format!("company {cid} not in companies")),
                    (_, Some(_)) => check("company", "ok_no_projection", String::new()),
                }
            }
            if let Some(cid) = v["countryId"].as_str() {
                match lookup("country", cid, Some(0)).or_else(|| lookup("country", cid, None)) {
                    None => check("country", "missing_target", format!("country {cid} not in countries")),
                    Some(_) => if v["country"].is_object() && v["country"]["id"].as_str() != Some(cid) { check("country", "mismatch", "embedded country.id differs".into()) } else { check("country", "ok", String::new()) },
                }
            }
            let mut linked_battery: Option<Value> = None;
            if f == "batterysystem" {
                let bid = v["batteryId"].as_str().unwrap_or("");
                let bver = v["batteryVersion"].as_i64();
                match lookup("battery", bid, bver) {
                    Some(b) => {
                        linked_battery = Some(b.clone());
                        let emb = &v["battery"];
                        if emb.is_object() {
                            let diffs: Vec<String> = ["id", "version", "ratedVoltage", "numberOfCellsSerie", "numberOfCellsParallel", "capacityAt10", "internalResistance"].iter()
                                .filter(|k| emb[**k] != b[**k]).map(|k| format!("{k}: embedded {} vs linked {}", emb[*k], b[*k])).collect();
                            if diffs.is_empty() { check("battery", "ok", String::new()) } else { check("battery", "mismatch", diffs.join("; ")) }
                        } else { check("battery", "ok_no_projection", String::new()) }
                    }
                    None => {
                        if v["battery"].is_object() { linked_battery = Some(v["battery"].clone()); check("battery", "fallback_embedded", format!("battery {bid} v{bver:?} not in batteries; using the embedded snapshot")); }
                        else { check("battery", "missing_target", format!("battery {bid} v{bver:?} not found")); }
                    }
                }
            }
            if let Some(oid) = v["powerOptimizerId"].as_str() {
                let over = v["powerOptimizerVersion"].as_i64();
                match lookup("optimizer", oid, over) {
                    Some(o) => if v["powerOptimizer"].is_object() && (v["powerOptimizer"]["id"] != o["id"] || v["powerOptimizer"]["version"] != o["version"]) { check("optimizer", "mismatch", "embedded optimizer id/version differ".into()) } else { check("optimizer", "ok", String::new()) },
                    None => if v["powerOptimizer"].is_object() { check("optimizer", "fallback_embedded", format!("optimizer {oid} v{over:?} not in poweroptimizers; using the embedded snapshot")) } else { check("optimizer", "missing_target", format!("optimizer {oid} v{over:?} not found")) },
                }
            }
            let company = v["companyId"].as_str().and_then(|c| lookup("company", c, None)).cloned().or_else(|| v["company"].as_object().map(|o| Value::Object(o.clone())));
            let country = v["countryId"].as_str().and_then(|c| lookup("country", c, Some(0)).or_else(|| lookup("country", c, None))).cloned();
            let n = normalize::normalize(f, v, linked_battery.as_ref(), company.as_ref(), country.as_ref(), &sha, ds);
            let ok_norm = n.record.is_some();
            *rep.entities_by_family.entry(f.to_string()).or_default() += 1;
            if ok_norm { *rep.normalized_by_family.entry(f.to_string()).or_default() += 1; } else if normalize::technical(f) { *rep.normalize_failures_by_family.entry(f.to_string()).or_default() += 1; }
            let cname = company.as_ref().and_then(|c| c["name"].as_str()).unwrap_or("");
            let search = format!("{} {} {} {}", v["name"].as_str().unwrap_or(""), cname, company.as_ref().and_then(|c| c["shortName"].as_str()).unwrap_or(""), country.as_ref().map(|c| ["name", "isoCode", "isoCode2", "deu", "fra", "eng", "esp", "pol", "ita", "prt", "ces", "tur"].iter().map(|k| c[*k].as_str().unwrap_or("")).collect::<Vec<_>>().join(" ")).unwrap_or_default()).to_lowercase();
            ins.execute(params![f, id, ver, v["name"].as_str(), v["companyId"].as_str(), v["countryId"].as_str(), v["isAvailable"].as_bool(), v["publishedAt"].as_str(), ds, rn, source_id, raw,
                n.record.as_ref().map(|r| json!({"record": r, "units": n.units}).to_string()), serde_json::to_string(&n.issues).ok(), search]).map_err(err)?;
        }
        tx.execute(
            "DELETE FROM join_check WHERE EXISTS (
                SELECT 1 FROM entity e WHERE e.source_id=?1 AND e.family=join_check.family
                AND e.id=join_check.id AND e.version=join_check.version
            )", [source_id],
        ).map_err(err)?;
        let mut ins_j = tx.prepare(
            "INSERT INTO join_check SELECT ?1,?2,?3,?4,?5,?6 WHERE EXISTS (
                SELECT 1 FROM entity WHERE family=?1 AND id=?2 AND version=?3 AND source_id=?7
            )",
        ).map_err(err)?;
        for (f, id, ver, link, status, detail) in &joins {
            ins_j.execute(params![f, id, ver, link, status, detail, source_id]).map_err(err)?;
            *rep.joins.entry(format!("{f}.{link}")).or_default().entry(status.clone()).or_default() += 1;
        }
        let mut ins_p = tx.prepare("INSERT INTO source_presence VALUES(?1,?2,?3,?4,?5)").map_err(err)?;
        for (ds, paths) in &presence {
            for (p, cls) in paths { rep.paths_observed += 1; for (k, n) in cls { ins_p.execute(params![source_id, ds, p, serde_json::to_value(k).ok().and_then(|x| x.as_str().map(String::from)), *n as i64]).map_err(err)?; } }
        }
    }
    // 2. Archived API responses (verbatim, checksum verified) and the export log.
    {
        let mut st = src.prepare("SELECT id, route, body_bytes, sha256, body FROM api_responses ORDER BY id").map_err(err)?;
        let mut ins = tx.prepare("INSERT INTO api_response VALUES(?1,?2,?3,?4,?5,?6,?7)").map_err(err)?;
        for x in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, Vec<u8>>(4)?))).map_err(err)? {
            let (id, route, n, h, body) = x.map_err(err)?;
            let ok = solarcraft_store::sha256_hex(&body).eq_ignore_ascii_case(&h) && body.len() as i64 == n;
            if !ok { rep.api_sha_mismatches += 1; }
            ins.execute(params![id, source_id, route, n, h, body, ok]).map_err(err)?;
            rep.api_responses += 1;
        }
        let mut st = src.prepare("SELECT source, route_or_table, dataset_name, exported, catalog_count, status FROM export_log").map_err(err)?;
        let mut ins = tx.prepare("INSERT INTO export_log VALUES(?1,?2,?3,?4,?5,?6,?7)").map_err(err)?;
        for x in st.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<i64>>(3)?, r.get::<_, Option<i64>>(4)?, r.get::<_, String>(5)?))).map_err(err)? {
            let (a, b, c2, d, e2, f2) = x.map_err(err)?;
            ins.execute(params![source_id, a, b, c2, d, e2, f2]).map_err(err)?;
        }
    }
    tx.execute_batch(
        "DELETE FROM presence;
         INSERT INTO presence SELECT dataset, path, class, SUM(n) FROM source_presence
             GROUP BY dataset, path, class;",
    ).map_err(err)?;
    tx.commit().map_err(err)?;
    let mut unique_ids: BTreeMap<String, std::collections::HashSet<String>> = BTreeMap::new();
    for (dataset, row, value, _) in &parsed {
        let family = family(dataset);
        if matches!(family, "changelog" | "release" | "publish") { continue; }
        let id = value["id"].as_str().map_or_else(|| format!("row-{row}"), str::to_owned);
        unique_ids.entry(family.to_owned()).or_default().insert(id);
    }
    rep.distinct_ids_by_family = unique_ids.into_iter()
        .map(|(family, ids)| (family, ids.len() as i64)).collect();
    Ok(rep)
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit { pub family: String, pub id: String, pub version: i64, pub versions: i64, pub name: String, pub company: String, pub country: String, pub is_available: Option<bool>, pub summary: Value }

/// Search the latest revision per entity. Every word of `query` must match name, company or
/// country (all locales); `company` and `country` filter by name or ISO code.
pub fn search(catalog: &Path, family: Option<&str>, query: &str, available_only: bool, limit: usize) -> Result<Vec<Hit>> {
    let c = open_ro(catalog)?;
    let mut sql = String::from("SELECT e.family, e.id, e.version, e.name, e.is_available, e.normalized_json, e.search, (SELECT COUNT(*) FROM entity x WHERE x.family=e.family AND x.id=e.id) FROM entity e WHERE e.version = (SELECT MAX(version) FROM entity y WHERE y.family=e.family AND y.id=e.id)");
    let mut args: Vec<String> = vec![];
    if let Some(f) = family { sql.push_str(" AND e.family = ?"); args.push(f.to_string()); }
    for w in query.split_whitespace() { sql.push_str(" AND e.search LIKE ?"); args.push(format!("%{}%", w.to_lowercase())); }
    if available_only { sql.push_str(" AND e.is_available = 1"); }
    sql.push_str(&format!(" ORDER BY e.name LIMIT {}", limit.max(1)));
    let mut st = c.prepare(&sql).map_err(err)?;
    let rows = st.query_map(rusqlite::params_from_iter(args.iter()), |r| {
        let n: Option<String> = r.get(5)?;
        let norm: Value = n.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        let rec = &norm["record"];
        Ok(Hit { family: r.get(0)?, id: r.get(1)?, version: r.get(2)?, name: r.get::<_, Option<String>>(3)?.unwrap_or_default(), is_available: r.get(4)?, versions: r.get(7)?,
            company: rec["manufacturer"].as_str().or(rec["utility"].as_str()).unwrap_or("").to_string(), country: rec["country"]["name"].as_str().unwrap_or("").to_string(),
            summary: normalize::summary(rec) })
    }).map_err(err)?.filter_map(|x| x.ok()).collect();
    Ok(rows)
}

/// Full details of one revision (latest when `version` is None): normalised record with units,
/// original JSON, issues, joins, revisions, changelog events and the release published around it.
pub fn details(catalog: &Path, family: &str, id: &str, version: Option<i64>) -> Result<Value> {
    let c = open_ro(catalog)?;
    let ver: i64 = match version { Some(v) => v, None => c.query_row("SELECT MAX(version) FROM entity WHERE family=?1 AND id=?2", params![family, id], |r| r.get::<_, Option<i64>>(0)).map_err(err)?.ok_or(format!("{family} {id} not found"))? };
    type DetailRow = (String, Option<String>, Option<String>, String, i64, Option<String>);
    let row: Option<DetailRow> = c.query_row("SELECT raw_json, normalized_json, issues_json, dataset, row_number, published_at FROM entity WHERE family=?1 AND id=?2 AND version=?3", params![family, id, ver],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).optional().map_err(err)?;
    let (raw, norm, issues, ds, rn, published) = row.ok_or(format!("{family} {id} v{ver} not found"))?;
    let versions: Vec<i64> = c.prepare("SELECT version FROM entity WHERE family=?1 AND id=?2 ORDER BY version").map_err(err)?.query_map(params![family, id], |r| r.get(0)).map_err(err)?.filter_map(|x| x.ok()).collect();
    let history: Vec<Value> = c.prepare("SELECT event_id, version, entry_tag, publish_date, name, company_name FROM changelog WHERE guid=?1 ORDER BY publish_date").map_err(err)?
        .query_map(params![id], |r| Ok(json!({"event": r.get::<_, i64>(0)?, "version": r.get::<_, Option<i64>>(1)?, "change": match r.get::<_, Option<i64>>(2)? { Some(0) => "created".to_string(), Some(1) => "new_version".to_string(), x => format!("tag {x:?}") }, "published": r.get::<_, Option<String>>(3)?, "name": r.get::<_, Option<String>>(4)?, "company": r.get::<_, Option<String>>(5)?}))).map_err(err)?.filter_map(|x| x.ok()).collect();
    let release: Option<Value> = published.as_ref().and_then(|p| c.query_row("SELECT version, at FROM release WHERE kind='publish' AND at >= ?1 ORDER BY at LIMIT 1", params![p], |r| Ok(json!({"database_version": r.get::<_, Option<String>>(0)?, "published": r.get::<_, Option<String>>(1)?}))).optional().ok().flatten());
    let joins: Vec<Value> = c.prepare("SELECT link, status, detail FROM join_check WHERE family=?1 AND id=?2 AND version=?3").map_err(err)?.query_map(params![family, id, ver], |r| Ok(json!({"link": r.get::<_, String>(0)?, "status": r.get::<_, String>(1)?, "detail": r.get::<_, String>(2)?}))).map_err(err)?.filter_map(|x| x.ok()).collect();
    let sha: String = c.query_row("SELECT s.sha256 FROM source s JOIN entity e ON e.source_id = s.id WHERE e.family=?1 AND e.id=?2 AND e.version=?3", params![family, id, ver], |r| r.get(0)).map_err(err)?;
    Ok(json!({"family": family, "id": id, "version": ver, "versions": versions, "dataset": ds, "row_number": rn, "source_sha256": sha,
        "normalized": norm.and_then(|s| serde_json::from_str::<Value>(&s).ok()), "raw": serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null),
        "issues": issues.and_then(|s| serde_json::from_str::<Value>(&s).ok()), "joins": joins, "history": history, "release": release}))
}

/// Revision summary for catalog history (changelog plus releases).
pub fn history(catalog: &Path, limit: usize) -> Result<Value> {
    let c = open_ro(catalog)?;
    let rel: Vec<Value> = c.prepare(&format!("SELECT kind, version, major, minor, at, comment FROM release ORDER BY at DESC LIMIT {limit}")).map_err(err)?
        .query_map([], |r| Ok(json!({"kind": r.get::<_, Option<String>>(0)?, "version": r.get::<_, Option<String>>(1)?, "major": r.get::<_, Option<i64>>(2)?, "minor": r.get::<_, Option<i64>>(3)?, "at": r.get::<_, Option<String>>(4)?, "comment": r.get::<_, Option<String>>(5)?}))).map_err(err)?.filter_map(|x| x.ok()).collect();
    let by_tag: Vec<Value> = c.prepare("SELECT family_code, entry_tag, COUNT(*) FROM changelog GROUP BY family_code, entry_tag ORDER BY family_code").map_err(err)?
        .query_map([], |r| Ok(json!({"family_code": r.get::<_, Option<i64>>(0)?, "entry_tag": r.get::<_, Option<i64>>(1)?, "events": r.get::<_, i64>(2)?}))).map_err(err)?.filter_map(|x| x.ok()).collect();
    Ok(json!({"releases": rel, "changelog_by_family_and_tag": by_tag}))
}

/// The normalised SolarCraft record of a revision with its catalog reference set (for pinning in projects).
pub fn resolve(catalog: &Path, family: &str, id: &str, version: Option<i64>) -> Result<(Value, solarcraft_model::plant::CatalogRef, Value)> {
    let d = details(catalog, family, id, version)?;
    let rec = d["normalized"]["record"].clone();
    if rec.is_null() { return Err(format!("{family} {id} v{} has no usable normalised record: {}", d["version"], d["issues"])); }
    let cref = solarcraft_model::plant::CatalogRef { family: family.to_string(), id: id.to_string(), version: d["version"].as_i64().unwrap_or(0), source_sha256: d["source_sha256"].as_str().unwrap_or("").to_string(), dataset: d["dataset"].as_str().unwrap_or("").to_string() };
    Ok((rec, cref, d["raw"].clone()))
}
