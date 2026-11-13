// SPDX-License-Identifier: GPL-3.0-or-later
//! `.solarcraft` project files: one SQLite database in WAL mode with
//! `synchronous=FULL` (see docs/spikes/F-project-store.md). Every save is a
//! transaction that also appends to the change log, so autosave is implicit
//! and history/undo survive restarts.
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use solarcraft_model::Project;
use std::path::Path;

pub const STORE_SCHEMA: &str = "solarcraft-store/1";

#[derive(Debug)]
pub enum StoreError { Sqlite(rusqlite::Error), Json(serde_json::Error), NotAProject(String), Missing(&'static str) }
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "database error: {e}"),
            Self::Json(e) => write!(f, "project data error: {e}"),
            Self::NotAProject(s) => write!(f, "not a SolarCraft project: {s}"),
            Self::Missing(s) => write!(f, "missing {s}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<rusqlite::Error> for StoreError { fn from(e: rusqlite::Error) -> Self { Self::Sqlite(e) } }
impl From<serde_json::Error> for StoreError { fn from(e: serde_json::Error) -> Self { Self::Json(e) } }
pub type Result<T> = std::result::Result<T, StoreError>;

pub struct Store { conn: Connection }

#[derive(Debug, Clone)]
pub struct Change { pub seq: i64, pub op: String, pub at_unix: i64 }

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

impl Store {
    fn configure(conn: &Connection) -> Result<()> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    }

    /// Create a new project file. Fails if the file exists.
    pub fn create(path: &Path, project: &Project) -> Result<Self> {
        if path.exists() { return Err(StoreError::NotAProject(format!("{} already exists", path.display()))); }
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        conn.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE project(id INTEGER PRIMARY KEY CHECK (id = 1), body TEXT NOT NULL, rev INTEGER NOT NULL);
             CREATE TABLE changes(seq INTEGER PRIMARY KEY AUTOINCREMENT, at_unix INTEGER NOT NULL, op TEXT NOT NULL, before TEXT);
             CREATE TABLE blobs(hash TEXT PRIMARY KEY, kind TEXT NOT NULL, media_type TEXT NOT NULL, data BLOB NOT NULL, created_unix INTEGER NOT NULL);",
        )?;
        conn.execute("INSERT INTO meta VALUES('schema', ?1)", [STORE_SCHEMA])?;
        conn.execute("INSERT INTO meta VALUES('application', 'SolarCraft')", [])?;
        conn.execute("INSERT INTO project(id, body, rev) VALUES(1, ?1, 1)", [serde_json::to_string(project)?])?;
        conn.execute("INSERT INTO changes(at_unix, op, before) VALUES(?1, 'create', NULL)", [now()])?;
        Ok(Store { conn })
    }

    pub fn open(path: &Path) -> Result<Self> {
        if !path.exists() { return Err(StoreError::NotAProject(format!("{} does not exist", path.display()))); }
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        let schema: Option<String> = conn.query_row("SELECT value FROM meta WHERE key='schema'", [], |r| r.get(0)).optional().map_err(|_| StoreError::NotAProject(path.display().to_string()))?;
        match schema.as_deref() {
            Some(STORE_SCHEMA) => Ok(Store { conn }),
            Some(other) => Err(StoreError::NotAProject(format!("unsupported store schema {other}"))),
            None => Err(StoreError::NotAProject(path.display().to_string())),
        }
    }

    pub fn load(&self) -> Result<Project> {
        let body: String = self.conn.query_row("SELECT body FROM project WHERE id=1", [], |r| r.get(0))?;
        Ok(serde_json::from_str(&body)?)
    }

    /// Save the project as one transaction, recording `op` and the prior state for undo.
    pub fn save(&mut self, project: &Project, op: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let before: String = tx.query_row("SELECT body FROM project WHERE id=1", [], |r| r.get(0))?;
        let after = serde_json::to_string(project)?;
        if before != after {
            tx.execute("UPDATE project SET body=?1, rev=rev+1 WHERE id=1", [&after])?;
            tx.execute("INSERT INTO changes(at_unix, op, before) VALUES(?1, ?2, ?3)", params![now(), op, before])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Undo the most recent change; returns the restored project.
    pub fn undo(&mut self) -> Result<Option<Project>> {
        let tx = self.conn.transaction()?;
        let last: Option<(i64, Option<String>)> = tx.query_row("SELECT seq, before FROM changes ORDER BY seq DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let Some((seq, Some(before))) = last else { return Ok(None) };
        tx.execute("UPDATE project SET body=?1, rev=rev+1 WHERE id=1", [&before])?;
        tx.execute("DELETE FROM changes WHERE seq=?1", [seq])?;
        tx.commit()?;
        Ok(Some(serde_json::from_str(&before)?))
    }

    pub fn history(&self, limit: usize) -> Result<Vec<Change>> {
        let mut st = self.conn.prepare("SELECT seq, op, at_unix FROM changes ORDER BY seq DESC LIMIT ?1")?;
        let rows = st.query_map([limit as i64], |r| Ok(Change { seq: r.get(0)?, op: r.get(1)?, at_unix: r.get(2)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Store a content-addressed blob; returns its SHA-256.
    pub fn put_blob(&mut self, kind: &str, media_type: &str, data: &[u8]) -> Result<String> {
        let hash = sha256_hex(data);
        self.conn.execute("INSERT OR IGNORE INTO blobs(hash, kind, media_type, data, created_unix) VALUES(?1, ?2, ?3, ?4, ?5)", params![hash, kind, media_type, data, now()])?;
        // Identical content is stored once; the pointer records which blob of a kind was written last.
        self.conn.execute("INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("latest:{kind}"), hash])?;
        Ok(hash)
    }

    pub fn get_blob(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT data FROM blobs WHERE hash=?1", [hash], |r| r.get(0)).optional()?)
    }

    /// Latest blob of a kind (e.g. "results").
    pub fn latest_blob(&self, kind: &str) -> Result<Option<(String, Vec<u8>)>> {
        let ptr: Option<String> = self.conn.query_row("SELECT value FROM meta WHERE key=?1", [format!("latest:{kind}")], |r| r.get(0)).optional()?;
        if let Some(h) = ptr { if let Some(d) = self.get_blob(&h)? { return Ok(Some((h, d))); } }
        Ok(self.conn.query_row("SELECT hash, data FROM blobs WHERE kind=?1 ORDER BY created_unix DESC, rowid DESC LIMIT 1", [kind], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
    }

    /// Checkpoint the WAL into the main file (call before copying/sharing).
    pub fn checkpoint(&self) -> Result<()> { self.conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?; Ok(()) }

    /// Write a compact, self-contained copy (portable export).
    pub fn export_copy(&self, dest: &Path) -> Result<()> {
        self.conn.execute("VACUUM INTO ?1", [dest.to_string_lossy()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_save_undo_blob_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sc-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.solarcraft");
        let _ = std::fs::remove_file(&path);
        let mut p = Project::new("p", "Test", 4.711, -74.072);
        let mut s = Store::create(&path, &p).unwrap();
        assert!(Store::create(&path, &p).is_err());
        p.set_by_pointer("/arrays/0/tilt", serde_json::json!(12.0)).unwrap();
        s.save(&p, "set tilt").unwrap();
        drop(s);
        let mut s = Store::open(&path).unwrap();
        assert_eq!(s.load().unwrap().arrays[0].tilt.value, 12.0);
        let h = s.put_blob("weather", "application/json", b"{}").unwrap();
        assert_eq!(s.get_blob(&h).unwrap().unwrap(), b"{}");
        let restored = s.undo().unwrap().unwrap();
        assert_eq!(restored.arrays[0].tilt.value, 5.0);
        assert_eq!(s.load().unwrap().arrays[0].tilt.value, 5.0);
        let copy = dir.join("copy.solarcraft");
        let _ = std::fs::remove_file(&copy);
        s.export_copy(&copy).unwrap();
        assert_eq!(Store::open(&copy).unwrap().load().unwrap().arrays[0].tilt.value, 5.0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
