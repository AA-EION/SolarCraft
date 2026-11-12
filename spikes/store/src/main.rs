// SPDX-License-Identifier: GPL-3.0-or-later
//! Spike F: single-file SQLite project store under process-kill stress.
//!   spike-store writer <file>        transactional edits forever; prints committed seq
//!   spike-store killtest <file> <n>  spawn writer, SIGKILL at random time, verify, repeat
//!   spike-store bigopen <file> <mb>  build a project with <mb> MB of results, time reopen
use rusqlite::{params, Connection};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn open(path: &str) -> Connection {
    let c = Connection::open(path).unwrap();
    c.pragma_update(None, "journal_mode", "WAL").unwrap();
    c.pragma_update(None, "synchronous", "FULL").unwrap();
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS objects(id TEXT PRIMARY KEY, kind TEXT NOT NULL, body TEXT NOT NULL, rev INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS changes(seq INTEGER PRIMARY KEY, object TEXT NOT NULL, op TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS blobs(hash TEXT PRIMARY KEY, kind TEXT NOT NULL, data BLOB NOT NULL);
         INSERT OR IGNORE INTO meta VALUES('schema','solarcraft-store-spike/1');",
    ).unwrap();
    c
}

/// xorshift for reproducible pseudo-random content without dependencies.
struct Rng(u64);
impl Rng { fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 } }

fn writer(path: &str) {
    let mut c = open(path);
    let mut seq: i64 = c.query_row("SELECT COALESCE(MAX(seq),0) FROM changes", [], |r| r.get(0)).unwrap();
    let mut rng = Rng(0x9E3779B97F4A7C15 ^ seq as u64);
    let out = std::io::stdout();
    loop {
        seq += 1;
        let tx = c.transaction().unwrap();
        // One logical edit = several rows, as a module-field move would be.
        for k in 0..8 {
            let id = format!("module-{}", rng.next() % 5000);
            let body = format!("{{\"x\":{},\"y\":{},\"edit\":{seq},\"k\":{k}}}", rng.next() % 10000, rng.next() % 10000);
            tx.execute("INSERT INTO objects(id,kind,body,rev) VALUES(?1,'module',?2,1)
                        ON CONFLICT(id) DO UPDATE SET body=excluded.body, rev=rev+1", params![id, body]).unwrap();
        }
        tx.execute("INSERT INTO changes(seq,object,op) VALUES(?1,'batch','move')", params![seq]).unwrap();
        if seq % 25 == 0 {
            let data: Vec<u8> = (0..262_144).map(|_| rng.next() as u8).collect();
            tx.execute("INSERT OR REPLACE INTO blobs(hash,kind,data) VALUES(?1,'results',?2)", params![format!("r{}", seq % 4), data]).unwrap();
        }
        tx.commit().unwrap();
        let mut o = out.lock();
        writeln!(o, "{seq}").unwrap();
        o.flush().unwrap();
    }
}

fn killtest(path: &str, n: u32) {
    let _ = std::fs::remove_file(path);
    let exe = std::env::current_exe().unwrap();
    let mut rng = Rng(42);
    let (mut max_seq, mut failures) = (0i64, 0u32);
    let t0 = Instant::now();
    for i in 0..n {
        let mut child = Command::new(&exe).args(["writer", path]).stdout(Stdio::piped()).spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut last = 0i64;
            for l in BufReader::new(stdout).lines().map_while(Result::ok) { last = l.parse().unwrap_or(last); }
            last
        });
        std::thread::sleep(Duration::from_millis(20 + rng.next() % 130));
        child.kill().unwrap(); // SIGKILL on Unix
        child.wait().unwrap();
        let acked = reader.join().unwrap();
        let c = open(path);
        let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        let db_seq: i64 = c.query_row("SELECT COALESCE(MAX(seq),0) FROM changes", [], |r| r.get(0)).unwrap();
        let gaps: i64 = c.query_row("SELECT COUNT(*) FROM changes", [], |r| r.get(0)).unwrap();
        // Every acknowledged commit must be durable; sequence must be contiguous.
        if ok != "ok" || db_seq < acked || gaps != db_seq || db_seq < max_seq {
            failures += 1;
            eprintln!("iteration {i}: integrity={ok} acked={acked} db={db_seq} rows={gaps}");
        }
        max_seq = db_seq;
    }
    println!("killtest iterations={n} failures={failures} final_seq={max_seq} elapsed={:.1}s", t0.elapsed().as_secs_f64());
    if failures > 0 { std::process::exit(1); }
}

fn bigopen(path: &str, mb: usize) {
    let _ = std::fs::remove_file(path);
    {
        let c = open(path);
        let mut rng = Rng(7);
        for i in 0..mb {
            let data: Vec<u8> = (0..1_048_576).map(|_| rng.next() as u8).collect();
            c.execute("INSERT INTO blobs(hash,kind,data) VALUES(?1,'results',?2)", params![format!("b{i}"), data]).unwrap();
        }
        for i in 0..20_000 {
            c.execute("INSERT INTO objects(id,kind,body,rev) VALUES(?1,'module','{}',1)", params![format!("m{i}")]).unwrap();
        }
        c.pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
    }
    let t0 = Instant::now();
    let c = open(path);
    let n: i64 = c.query_row("SELECT COUNT(*) FROM objects", [], |r| r.get(0)).unwrap();
    let mut st = c.prepare("SELECT id, body FROM objects").unwrap();
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().count();
    let open_ms = t0.elapsed().as_secs_f64() * 1e3;
    let size = std::fs::metadata(path).unwrap().len();
    println!("bigopen size={:.1}MB objects={n} loaded={rows} open_and_load_model_ms={open_ms:.1}", size as f64 / 1e6);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("writer") => writer(&a[2]),
        Some("killtest") => killtest(&a[2], a[3].parse().unwrap()),
        Some("bigopen") => bigopen(&a[2], a[3].parse().unwrap()),
        _ => eprintln!("usage: spike-store writer|killtest|bigopen ..."),
    }
}
