# Porting IngeTrazo code into SolarCraft

SolarCraft takes geometry code from [IngeTrazo](https://github.com/ingelibre/ingetrazo). Both projects are GPL-3.0-or-later (owner decision D-03).

The arrangement is designed so that new upstream code ports mechanically:

| Upstream | Here | How it stays in sync |
|---|---|---|
| `rust/ingetrazo_core/src/<name>.rs` (native kernels, pure Rust) | `src/upstream/rust/<name>.rs`, byte for byte | `tools/ingetrazo_sync.py` copies the files at a commit and updates the SHA-256 pins. A test fails if a vendored file is edited locally. |
| `rust/ingetrazo_core/src/lib.rs` (pyo3 wrappers for Python) | not taken | SolarCraft calls the kernels directly from Rust. |
| `core/<name>.py` (Python modules) | `src/core/<name>.rs`: a Rust port with the same module name | The sync tool compares the pinned upstream hash. When the Python source changes, it writes the upstream diff for review, and `--accept` moves the pin after the port is updated. |

## Update to a new upstream commit

```bash
git -C ../ingetrazo fetch
```

```bash
python3 tools/ingetrazo_sync.py ../ingetrazo --commit origin/main
```

```bash
cargo test -p solarcraft-ingetrazo -p solarcraft-geometry
```

## Rules

- **Never edit `src/upstream/`.** Fix bugs upstream and re-sync.
- **Port Python modules function by function, keeping upstream names.** Record the upstream path in the file header.
- **Keep attribution.** Each file keeps its upstream copyright line, and `NOTICE` names IngeTrazo.
- **Add new kernels in two places.** Declare each new kernel module in `src/lib.rs` under its upstream name, so its `crate::` paths resolve unchanged. Add it to `upstream.toml` as a `[[vendored]]` entry; the sync tool lists unmapped kernels.
