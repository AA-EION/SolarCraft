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

## Where SolarCraft uses the code

`crates/scene` builds the 3D scene on these kernels; `docs/scene.md` has the details.

| Upstream | In SolarCraft | Used by |
|---|---|---|
| `facegeom.rs` (`face_geometry`) | vendored | triangulating every scene face with holes |
| `weld.rs` | vendored | welding solid vertices before closure checks |
| `parity.rs` (`Tris::crossings`) | vendored | point-in-solid and orientation votes |
| `hlr.rs` (`Occluders::visible_spans_all`) | vendored | hidden-line roof plan in the DXF export |
| `core/orient.py` | ported to `src/core/orient.rs` | closure, coherent winding, signed volume, `ray_parity_outside` |
| `core/triangulate.py` | ported to `src/core/triangulate.rs` | roof-plane keep-outs (`crates/geometry`) |
| `formats/igz.py` (format 1 JSON) | read and written by `crates/scene/src/io/igz.rs` | the `.igz` bridge; checked against IngeTrazo's own reader with `tools/validate_exchange.py --ingetrazo` |

`gltape.rs`, `instances.rs`, `boxes.rs` and `occlusion.rs` are vendored and compiled but not called yet.
