# 4. Open-source candidates and algorithm comparison

Checked 2026-11-12. Licences were read from the upstream licence file at the stated ref. "Latest" is the newest tag returned by `git ls-remote` that day. **Nothing here is adopted yet.** Each entry is labelled *reuse*, *oracle* (validation only), *candidate* (needs a spike) or *omit*.

## 4.1 Library candidates

| Domain | Candidate | Licence (verified) | Latest ref | Language / binding | Proposed role | Notes / risks |
|---|---|---|---|---|---|---|
| PV modelling | [pvlib-python](https://github.com/pvlib/pvlib-python) | BSD-3-Clause | v0.16.1 (`ba161f99`) | Python | **Oracle**. Selected functions ported to Rust with attribution (BSD permits). | Reference implementation for solar position, transposition, temperature, IAM, single-diode, inverter, trackers, bifacial. |
| PV/storage/finance | [SAM SSC](https://github.com/NatLabRockies/ssc) (moved from NREL org) | BSD-3-Clause | `2026.7.3.r0.ssc.308` | C++ with C API | **Oracle** + **candidate reuse** for the battery lifetime and CEC 6-param fit (`6par_solve`) | A large C++ build. Must be checked on Windows ARM64 and macOS x86_64. |
| PV/SAM desktop | [SAM](https://github.com/NatLabRockies/SAM) | BSD-3-Clause | develop | C++ (wxWidgets) | **Omit** as code. UX and benchmark reference only. | |
| Mismatch | pvmismatch (SunPower) | BSD-3 (**verify**) | — | Python | **Oracle** for bypass-diode I–V | |
| Booleans | [manifold](https://github.com/elalish/manifold) | Apache-2.0 | v3.5.4 | C++ / C API | **Candidate reuse** for solid booleans (roof cut-outs, terrain) | Already used by IngeTrazo, OpenSCAD and Blender. |
| 2D polygons | `geo`, `i_overlay` (Rust) | MIT/Apache-2.0 (geo verified; i_overlay **verify**) | — | Rust | **Candidate reuse** for roof layout, setbacks, offsets | |
| Triangulation | earcut | ISC | — | C++/Rust ports | **Reuse** | |
| Ray tracing | Intel Embree | Apache-2.0 (**verify** current) | — | C++ | **Candidate** for shading. ARM64 support exists via the NEON path. Must be tested on Windows ARM64. | Alternative: an own BVH in Rust (simpler packaging), or wgpu compute shadow maps. |
| GPU / viewport | [wgpu](https://github.com/gfx-rs/wgpu) | MIT/Apache-2.0 | — | Rust | **Candidate** shared renderer (D3D12/Metal) | Spike B. |
| IFC | [IfcOpenShell](https://github.com/IfcOpenShell/IfcOpenShell) | LGPL-3.0 | v0.8.x (**verify** exact) | C++/Python | **Candidate**: dynamic link or helper process | LGPL obligations (relinkable). Large dependency (OCCT for geometry). |
| SKP | [openskp](https://github.com/iamahsanmehmood/openskp) | MIT | `main` (IngeTrazo pins fork `291700bc`) | C++, Rust, Python, .NET… | **Candidate reuse** for read; write after verification in SketchUp | Built by reverse engineering (README). Counsel item L4. |
| DXF | `dxf` crate (Rust) / ezdxf (Python, MIT) | MIT (**verify** crate) | — | Rust/Python | **Candidate** | |
| DWG | [LibreDWG](https://github.com/LibreDWG/libredwg) | GPL-3.0 | — | C | **Optional separate helper** only | Same arm's-length pattern as IngeTrazo. ODA SDK is the paid alternative (§5 of 05). |
| CityGML / geo | `proj` (MIT), `geo`, `gdal` (MIT/X) | MIT | — | Rust/C | Candidate | GDAL is heavy. Prefer pure-Rust readers for GeoTIFF/CityGML. |
| Map rendering | [MapLibre Native](https://github.com/maplibre/maplibre-native) | BSD-2-Clause | — | C++ | **Candidate** for a site map. Windows support must be checked. | Alternative: draw tiles in our wgpu viewport. |
| FFI | [UniFFI](https://github.com/mozilla/uniffi-rs) | MPL-2.0 | — | Rust → Swift, (C# via community uniffi-bindgen-cs) | **Candidate** | C# binding is community-maintained. Fallback: a C ABI with csbindgen (MIT). |
| MCP | [rmcp (rust-sdk)](https://github.com/modelcontextprotocol/rust-sdk) | MIT (and Apache text present; **verify**) | rmcp-v3.9.0 | Rust | **Reuse** | Track spec [2026-07-28](https://modelcontextprotocol.io/specification/latest) (stateless). |
| PDF reports | [Typst](https://github.com/typst/typst) | Apache-2.0 | — | Rust library | **Candidate** for PDF typesetting | Fonts must be redistributable (OFL). |
| DOCX | docx-rs | MIT (**verify**) | — | Rust | Candidate | Editable export. |
| LP/MILP | HiGHS | MIT | — | C++ | Candidate for STO-007 | |
| Embedded DB | SQLite | Public domain | — | C | **Reuse** for project store and local catalog | |
| Columnar results | Apache Arrow / Parquet (arrow-rs) | Apache-2.0 | — | Rust | Candidate | |
| IngeTrazo | [ingetrazo](https://github.com/ingelibre/ingetrazo) | GPL-3.0-or-later | v0.6.1 `ffc7187b` | Python/Qt + Rust | **Omit as dependency**; companion bridge (see 02 §4) | |

"Verify" marks licences I did not read directly today. Each gets checked from upstream before adoption, together with transitive dependencies, through `cargo deny` and an SBOM.

## 4.2 Algorithm comparison per subsystem

Columns: what PV\*SOL does (from the bundle), the public basis, the available implementation, the SolarCraft plan, and the shared fixture used to score them. Scoring dimensions for spike reports: correctness on fixtures, error and uncertainty, edge cases, reproducibility, speed, memory, binding/packaging, maintenance and redistribution.

| Subsystem | PV\*SOL 2026 R4 (`STATIC`) | Public basis (`PUB`) | Open implementation | SolarCraft plan | Shared fixture |
|---|---|---|---|---|---|
| Solar position | Two modes in legacy solver; managed engine separate | NREL SPA (Reda & Andreas 2004); NOAA | pvlib `spa_python`, SSC | Rust SPA + fast mode; select per run | SPA paper test vectors; 100k random times vs pvlib |
| Decomposition | Six split equations (legacy); Perez/DIRINT-style separation in Meteonorm | Erbs 1982, Reindl 1990, DIRINT (Perez 1992), Engerer2 (2015) | pvlib | Implement Erbs, DIRINT, Engerer2 | BSRN station-years (public) |
| Transposition | Hay–Davies, Liu–Jordan, Klucher, Perez (1990 coeffs), Reindl, with gates sin α<0.035, q<0.01 | Same publications | pvlib `get_total_irradiance` | Implement all five; gates as documented options | pvlib parity on PVGIS TMY for 10 sites |
| Module temperature | Linear and dynamic; wind correction | Faiman 2008; SAPM; PVsyst; Prilliman 2020 | pvlib | Implement four | pvlib parity |
| Module electrical | Low-light anchor fit + FF(G) rational curve; one/two-diode fits; MPP sampling | De Soto 2006; Dobos 2012 (CEC 6-par); PVsyst 1-diode (Mermoud); two-diode (Ishaque 2011) | SSC `6par_solve`, pvlib `singlediode` | CEC/De Soto as default. "PV\*SOL-like low-light" mode only if spec'd from public description + black-box tests. | CEC module list (public) fits; low-light efficiency at 200 W/m² vs datasheet |
| Mismatch / bypass | Managed electrical handoff of shading per cell row | Bishop 1988 reverse-bias; Deline 2013 | pvmismatch | Cell-string I–V with diodes, cached per shade pattern | pvmismatch reference cases |
| Inverter | Efficiency curve at 8 load knots, clipping, start threshold, MPP window | Sandia (King 2007), Driesse 2008 (ADR) | pvlib, SSC | Sandia + ADR + PV\*SOL-like knot curve | CEC inverter list |
| Direct shading | Silhouette projection + clipping onto module plane; 3-byte per slot shade grid; resampling | Ray casting / shadow volumes (standard CG) | Embree, own BVH | Spike C compares BVH ray cast vs GPU shadow map vs analytic polygon clipping | Analytic cases: wall, pole, row-to-row, tree (transmittance) |
| Diffuse shading | Sky-view reduction per module + horizontal | Sky view factor integration (isotropic/Perez-weighted) | own | Stratified hemisphere sampling | Urban canyon analytic SVF |
| Minute irradiance | 201×201 Markov transition matrices, seeded draws | Hofmann et al. 2014; Bright et al. 2015 | (various, check licences) | Own published-method generator with own fitted matrices from public BSRN | Hourly mean exactness; ramp-rate distribution vs BSRN |
| Battery | DC/AC controllers, charge modes, Li-ion pack with aging | Shepherd/Tremblay; NREL BLAST-Lite; SSC battery | SSC (BSD), BLAST-Lite (BSD-3, verify) | Simple + detailed, same dispatch contract | Energy conservation; SSC parity scenarios |
| Dispatch | Rule-based state machine | Rule-based; LP (HiGHS) | — | Rule-based default; LP option (owner Q) | Brief's contradiction/outage/negative-price set |
| Economics | Annuity, NPV, IRR, LCOE, tax, depreciation, 4 loan calendars | Standard finance; IEA/NREL LCOE definitions | numpy-financial (BSD) oracle | Own | Spreadsheet fixtures |
| Photo rectification | Homography + IPL warp | Hartley & Zisserman DLT | own | Own | Synthetic rendered photos |

## 4.3 Validation oracles and datasets (public CI-safe)

PVGIS TMY (download per terms, stored as fixtures if permitted), NREL NSRDB samples, BSRN (citation required, redistribution **verify**), the CEC module and inverter lists (California Energy Commission, public), the Sandia inverter database, NIST ground-truth PV datasets (public domain, US Gov), and the PVPMC blind-modelling comparison. Proprietary or uncertain data stays out of the public CI. A private validation lane runs it.
