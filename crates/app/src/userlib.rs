// SPDX-License-Identifier: GPL-3.0-or-later
//! The user's local equipment library: a SQLite file outside any project.
//! Records use exactly the schema of project-pinned and catalog equipment,
//! work offline, keep every revision, and are copied into a project when
//! selected (so projects stay portable and never depend on the library or a
//! subscription).
use crate::plant_ops::{canonical_equipment, KINDS};
use crate::{e, open, save, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// `SOLARCRAFT_LIBRARY` if set, else the platform's per-user data directory.
pub fn default_path() -> PathBuf {
    if let Ok(p) = std::env::var("SOLARCRAFT_LIBRARY") { return PathBuf::from(p); }
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    let base = if cfg!(target_os = "macos") { PathBuf::from(&home).join("Library/Application Support/SolarCraft") }
        else if cfg!(windows) { std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(&home)).join("SolarCraft") }
        else { std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(&home).join(".local/share")).join("solarcraft") };
    base.join("library.sqlite")
}

fn conn(lib: &Path) -> Result<Connection> {
    if let Some(d) = lib.parent() { std::fs::create_dir_all(d).map_err(|x| e("LIBRARY_OPEN", x))?; }
    let c = Connection::open(lib).map_err(|x| e("LIBRARY_OPEN", x))?;
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        CREATE TABLE IF NOT EXISTS item (kind TEXT NOT NULL, id TEXT NOT NULL, rev INTEGER NOT NULL, body TEXT NOT NULL, source TEXT NOT NULL, updated_unix INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (kind, id, rev));
        CREATE INDEX IF NOT EXISTS item_updated ON item(updated_unix);")
        .map_err(|x| e("LIBRARY_OPEN", x))?;
    Ok(c)
}

#[derive(Debug, Clone, Serialize)]
pub struct Item { pub kind: String, pub id: String, pub rev: i64, pub source: String, pub updated_unix: i64, pub record: Value }

fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

/// Save (create or new revision). `source`: user, imported or catalog.
pub fn put(lib: &Path, kind: &str, record: &Value, source: &str) -> Result<Item> {
    let rec = canonical_equipment(kind, record)?;
    let id = rec["id"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| e("EQUIPMENT_INVALID", "id is required"))?.to_string();
    let c = conn(lib)?;
    let rev: i64 = c.query_row("SELECT COALESCE(MAX(rev),0)+1 FROM item WHERE kind=?1 AND id=?2", params![kind, id], |r| r.get(0)).map_err(|x| e("LIBRARY", x))?;
    let t = now();
    c.execute("INSERT INTO item(kind,id,rev,body,source,updated_unix) VALUES(?1,?2,?3,?4,?5,?6)", params![kind, id, rev, rec.to_string(), source, t]).map_err(|x| e("LIBRARY", x))?;
    Ok(Item { kind: kind.into(), id, rev, source: source.into(), updated_unix: t, record: rec })
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    let body: String = r.get(3)?;
    Ok(Item { kind: r.get(0)?, id: r.get(1)?, rev: r.get(2)?, record: serde_json::from_str(&body).unwrap_or(Value::Null), source: r.get(4)?, updated_unix: r.get(5)? })
}

/// Latest revision of each live record, optionally filtered by kind and text (every word must
/// occur in the record), newest first, at most `limit` (default 200).
pub fn list(lib: &Path, kind: Option<&str>, query: Option<&str>) -> Result<Vec<Item>> { list_limited(lib, kind, query, 200) }

pub fn list_limited(lib: &Path, kind: Option<&str>, query: Option<&str>, limit: usize) -> Result<Vec<Item>> {
    let c = conn(lib)?;
    let words: Vec<String> = query.unwrap_or("").split_whitespace().map(|w| format!("%{}%", w.to_lowercase())).collect();
    let mut sql = "SELECT i.kind,i.id,i.rev,i.body,i.source,i.updated_unix FROM item i JOIN (SELECT kind,id,MAX(rev) r FROM item GROUP BY kind,id) m ON i.kind=m.kind AND i.id=m.id AND i.rev=m.r WHERE i.deleted=0".to_string();
    let mut args: Vec<String> = vec![];
    if let Some(k) = kind { sql.push_str(" AND i.kind=?"); args.push(k.to_string()); }
    for w in words { sql.push_str(" AND lower(i.body) LIKE ?"); args.push(w); }
    sql.push_str(&format!(" ORDER BY i.updated_unix DESC, i.kind, i.id LIMIT {}", limit.max(1)));
    let mut st = c.prepare(&sql).map_err(|x| e("LIBRARY", x))?;
    let items = st.query_map(rusqlite::params_from_iter(args.iter()), row).map_err(|x| e("LIBRARY", x))?.filter_map(|x| x.ok()).collect();
    Ok(items)
}

pub fn count(lib: &Path) -> Result<Vec<(String, i64)>> {
    let c = conn(lib)?;
    let mut st = c.prepare("SELECT kind, COUNT(DISTINCT id) FROM item WHERE deleted=0 GROUP BY kind ORDER BY kind").map_err(|x| e("LIBRARY", x))?;
    let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|x| e("LIBRARY", x))?.filter_map(|x| x.ok()).collect();
    Ok(v)
}

/// Save many already-canonical records in one transaction (bulk import). Returns how many were written.
pub fn put_many(lib: &Path, items: &[(String, Value)], source: &str) -> Result<usize> {
    let mut c = conn(lib)?;
    let tx = c.transaction().map_err(|x| e("LIBRARY", x))?;
    let t = now();
    let mut n = 0;
    {
        let mut next = tx.prepare("SELECT COALESCE(MAX(rev),0)+1 FROM item WHERE kind=?1 AND id=?2").map_err(|x| e("LIBRARY", x))?;
        let mut ins = tx.prepare("INSERT INTO item(kind,id,rev,body,source,updated_unix) VALUES(?1,?2,?3,?4,?5,?6)").map_err(|x| e("LIBRARY", x))?;
        for (kind, rec) in items {
            let Some(id) = rec["id"].as_str() else { continue };
            let rev: i64 = next.query_row(params![kind, id], |r| r.get(0)).map_err(|x| e("LIBRARY", x))?;
            ins.execute(params![kind, id, rev, rec.to_string(), source, t]).map_err(|x| e("LIBRARY", x))?;
            n += 1;
        }
    }
    tx.commit().map_err(|x| e("LIBRARY", x))?;
    Ok(n)
}

pub fn get(lib: &Path, kind: &str, id: &str) -> Result<Item> {
    let c = conn(lib)?;
    c.query_row("SELECT kind,id,rev,body,source,updated_unix FROM item WHERE kind=?1 AND id=?2 AND deleted=0 ORDER BY rev DESC LIMIT 1", params![kind, id], row)
        .optional().map_err(|x| e("LIBRARY", x))?.ok_or_else(|| e("NOT_FOUND", format!("{kind} '{id}' is not in the library")))
}

pub fn history(lib: &Path, kind: &str, id: &str) -> Result<Vec<Item>> {
    let c = conn(lib)?;
    let mut st = c.prepare("SELECT kind,id,rev,body,source,updated_unix FROM item WHERE kind=?1 AND id=?2 ORDER BY rev").map_err(|x| e("LIBRARY", x))?;
    let v = st.query_map(params![kind, id], row).map_err(|x| e("LIBRARY", x))?.filter_map(|x| x.ok()).collect();
    Ok(v)
}

/// Mark deleted (a tombstone revision; history is kept).
pub fn delete(lib: &Path, kind: &str, id: &str) -> Result<()> {
    let it = get(lib, kind, id)?;
    let c = conn(lib)?;
    c.execute("INSERT INTO item(kind,id,rev,body,source,updated_unix,deleted) VALUES(?1,?2,?3,?4,?5,?6,1)", params![kind, id, it.rev + 1, it.record.to_string(), it.source, now()]).map_err(|x| e("LIBRARY", x))?;
    Ok(())
}

/// Copy a library record into a project (the project keeps its own copy).
pub fn pin(lib: &Path, project: &Path, kind: &str, id: &str) -> Result<Value> {
    let it = get(lib, kind, id)?;
    // Imported modules are fitted when first used.
    let rec = canonical_equipment(kind, &it.record)?;
    let (mut s, mut p) = open(project)?;
    crate::plant_ops::insert(&mut p, kind, rec.clone())?;
    solarcraft_model::derive::derive(&mut p);
    save(&mut s, &p, &format!("pin {kind} {id} rev {}", it.rev))?;
    Ok(rec)
}

/// Import records from a JSON file: `[{"kind": "...", "record": {...}}]` or a catalog bundle `{"items": [...]}`.
pub fn import_file(lib: &Path, file: &Path, source: &str) -> Result<Vec<Item>> {
    let text = std::fs::read_to_string(file).map_err(|x| e("FILE_READ", x))?;
    let v: Value = serde_json::from_str(&text).map_err(|x| e("PARSE", x))?;
    let arr = v.get("items").cloned().unwrap_or(v);
    let arr = arr.as_array().ok_or_else(|| e("PARSE", "expected an array of {kind, record}"))?;
    let mut out = vec![];
    for x in arr {
        let kind = x.get("kind").and_then(Value::as_str).filter(|k| KINDS.contains(k)).ok_or_else(|| e("PARSE", "item without a valid kind"))?;
        out.push(put(lib, kind, x.get("record").unwrap_or(&Value::Null), source)?);
    }
    Ok(out)
}
