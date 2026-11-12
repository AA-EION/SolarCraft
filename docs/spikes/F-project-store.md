# Spike F: single-file SQLite project store

**Result: PASS on Linux.** Windows and macOS runs are pending CI. Run on 2026-11-12.

## Design under test
One `.solarcraft` file in SQLite with WAL journal and `synchronous=FULL`. The tables are `meta`, `objects` (id, kind, JSON body, revision), `changes` (append-only log for undo, history and MCP transactions) and `blobs` (content-addressed results and weather). Each user edit is one transaction.

## Results
| Test | Criterion | Result |
|---|---|---|
| Process-kill stress: writer SIGKILLed at a random 20–150 ms, 1,000 times | No corruption; every acknowledged commit durable; contiguous log | **0 failures**. 147,794 commits, all `integrity_check = ok`. 106 s. |
| Open a 53 MB project (50 MB results, 20,000 objects) and load the model | < 1 s | **3.7 ms** |

## Findings and rules for the product
- SQLite in WAL mode **must not run on network shares** (SMB/NFS locking). When the project path is remote, the app copies it to a local working file and saves back atomically. Same approach on cloud-sync folders (OneDrive/iCloud/Dropbox), which can upload `-wal`/`-shm` side files mid-transaction.
- "Save" becomes a checkpoint plus an optional compact copy. Autosave is implicit: every edit is already committed, which closes PRJ-007.
- Power-loss durability relies on `synchronous=FULL` and the OS honouring fsync. The pull-the-plug test needs real hardware and is a manual protocol.
- Portable export (PRJ-008/015) is `VACUUM INTO` a temporary file, then a hash manifest.

## Reproduce
```bash
cargo build --release -p spike-store
target/release/spike-store killtest /tmp/k.solarcraft 1000
target/release/spike-store bigopen /tmp/b.solarcraft 50
```
