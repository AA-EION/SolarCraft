# 1. Reference inventory, feasibility and discovery plan

## 1.1 What I could access

The ZIP `SolarCraft-PVSol-Reference.zip` opened cleanly: 16,390,619 bytes, SHA-256 `f4f27000…d8eb800`, 729 entries, 137 MB extracted. I extracted it to a session scratch directory. Nothing from it is committed.

| Item | Read? | Notes |
|---|---|---|
| Orientation note and brief | Yes, in full | Primary reference build: **PV\*SOL premium 2026 R4, version `2026.8.26109.0`** (Windows). |
| Coverage audit and report index | Yes, in full | |
| Subsystem reports (UI, data formats, persistence, maths, storage, economics, 3D, reports) | Key sections | Used for the feature matrix. |
| Supporting tables and listings | Indexed only | Read only to check a specific claim. No bundled script was run. |

Per-file citations, hashes and the list of omitted items live in a **private evidence index** kept outside this repository (owner decision D-08). Feature rows cite it by key (`EV-<row id>`). The bundle contains no example projects, weather files, catalogs or templates. So every `SAVED` claim is second-hand until owner-supplied files confirm it.

## 1.2 Version reconciliation

| Version | Date | Source | Relevance |
|---|---|---|---|
| 2026 R1 | 2025-11-24 | [Product page release history](https://valentin-software.com/en/products/pvsol-premium/) | Thermal+battery type, multiple grid tariffs, compact report, JSON export, Meteonorm 9 |
| 2026 R2 / R3 | 2026-01-26 / 27 | same | AC-battery and EV circuit symbols, grouped load profiles |
| **2026 R4** | **2026-03-18** | same; bundle build `2026.8.26109.0` | **Analyzed build.** Google Solar API in Latin America, PAN import fixes |
| 2026 R5 / R6 | 2026-06-09 / 10 | same | Rexplorer API (Germany), SolarAnywhere TMY update, new load profiles |
| 2026 R7 | 2026-08-04 | same | Rexplorer example |
| 2026 R8 | 2026-09-22 | same | New map providers (PL, CZ, German states) |
| 2027 R1 | **2026-11-11** | same; [shop](https://shop.valentin-software.com/en/collections/kategorie-photovoltaik) | Module limits 15,000 roof / 10,000 ground, capacity-price tariffs, dynamic tariff import, follow-up investments, battery peak shaving and delayed charging |

The parity baseline is "PV\*SOL premium 2026", and the last 2026 release is R8. Rows that cover R5–R8 are tagged by version with `DOC` evidence only, because the bundle predates them. 2027 R1 shipped yesterday. I have added its features as separate `2027R1` rows so they stay visible. Whether they count toward parity is owner question Q9.

## 1.3 Feasibility assessment (evidence-based)

**Overall: feasible as a multi-year programme.** Full parity is a 3–4 year effort for a small senior team. A credible product for a defined segment can arrive much sooner. The reasons are below.

1. **The physics is not the hard part.** Every simulation model the bundle identifies has a public description and at least one BSD-3 open implementation (pvlib, SAM/SSC). Examples are the five transposition models, the Perez coefficients, the one- and two-diode fits, NOCT/Faiman temperature, the inverter efficiency curve, the Li-ion and lead-acid storage models, annuity loans and NPV/IRR/LCOE. The bundle adds value as a source of defaults, thresholds, gates and sign conventions. Example: the beam-projection gates on low sun and grazing incidence (`STATIC`, private evidence index).
2. **3D layout plus shading is the critical path.** PV\*SOL's 3D is a mature Delphi/Direct3D application (Visual3D, 45k functions). IngeTrazo is a promising modeller, but its geometry core is Python objects coupled to Qt (51 of 80 `core/*.py` import PySide6). Its own roadmap reports 0.6–1.3 s push/pull edits and "too heavy for millions of faces". That suits rooftop editing. It does not meet utility-scale or native-shell requirements. Details are in [02 §4](02-commercial-licensing-memo.md#4-ingetrazo-options) and [05](05-platform-interop-feasibility.md).
3. **Data is the commercial moat and the main cost.** The vendor ships 30k+ modules, ~9k inverters and 12.5–16k battery systems, plus Meteonorm 9 weather. SolarCraft cannot reuse any of it. Open alternatives cover a meaningful core: CEC module and inverter lists, PVGIS, NASA POWER, and Overture/OSM buildings. Europe-wide battery and hybrid-inverter data does not exist in open form and must be curated.
4. **Native file fidelity varies widely.** Neutral formats (IFC, glTF, OBJ, DXF, CityGML) are fully achievable with open libraries. `.skp` read/write is plausible via openskp (MIT, reverse-engineered). `.dwg` needs LibreDWG (GPL-3) or ODA (paid). `.rvt`/`.rfa` is realistically only possible through an Autodesk-licensed add-in or APS. PV\*SOL `.pvprj` import is feasible for project versions 15–24 using structure documented in the bundle. That depends on the provenance question in [02 §1](02-commercial-licensing-memo.md#1-evidence-provenance-needs-counsel-before-use).
5. **Platforms:** a Rust core with WinUI 3 (C#) and SwiftUI/AppKit shells is a well-trodden pattern. Windows ARM64, macOS universal and Intel are all buildable. Intel macOS is limited to macOS 26 (macOS 27 is Apple-silicon-only, [AppleInsider, 2026-06-08](https://appleinsider.com/articles/26/06/08/macos-27-compatibility-list-focuses-entirely-on-apple-silicon)). So the Intel slice has a fixed horizon: OS security updates usually run about 2 more years.
6. **Reference-material policy.** The owner decided (D-04) that the reference findings may inform behaviour and mathematics as fully as possible. Equipment and catalog data come only from the owner's own database and user input, never from vendor files. See [02 §1](02-commercial-licensing-memo.md#1-reference-material-policy).

### Rough effort and cost ranges (for planning, not a quote)

| Stage | Scope | Team (FTE) | Calendar | Cost range* |
|---|---|---|---|---|
| S0 Discovery + spikes | This document set, 6 technical spikes, counsel review | 1–2 | 6–10 weeks | €40k–90k + counsel €10k–30k |
| S1 Vertical slice | Project → weather → 2D layout → simulate → explain → PDF, both shells, CLI/MCP | 3–4 | 4–6 months | €250k–450k |
| S2 Engineering depth | Validated PV/inverter/shading, 3D roof editor, strings, SLD | 5–7 | 9–12 months | €0.6M–1.1M |
| S3 Storage/EV/thermal/economics | Dispatch engine, tariffs, finance, reports | 5–7 | 6–9 months | €0.5M–0.9M |
| S4 Catalog server + interop | Hosted catalog, curation tools, IFC/SKP/DXF, PV\*SOL import | 4–6 | 6–9 months (parallel with S3) | €0.4M–0.8M + data |
| S5 Parity completion + release | Remaining matrix rows, localisation, packaging, docs | 5–8 | 6–12 months | €0.5M–1.2M |

\*Loaded cost assumption: €100k–140k per FTE-year in Europe. Data acquisition is separate. See [02 §3](02-commercial-licensing-memo.md#3-catalog-business-model).

## 1.4 Discovery plan (what happens next, in order)

| Step | Output | Needs approval? |
|---|---|---|
| D1 | Owner answers Q1–Q9 | — |
| D2 | Counsel review of bundle provenance and use rules; IngeTrazo boundary; code licence | Owner selects counsel |
| D3 | Spike A: Rust core + UniFFI → WinUI 3 (x64, ARM64) and SwiftUI (arm64, x86_64), one call each way, signed builds | No (throwaway, private branch) |
| D4 | Spike B: wgpu viewport inside SwapChainPanel / CAMetalLayer; 10k and 200k module instance scenes; frame time and memory | No |
| D5 | Spike C: shading engine shoot-out on shared synthetic fixtures (ray-cast BVH vs GPU shadow-map vs polygon clipping) | No |
| D6 | Spike D: PV chain against pvlib/SSC oracles (transposition, temperature, CEC 6-param, Sandia inverter) on public TMY | No |
| D7 | Spike E: IngeTrazo exchange: `.igz`/glTF/IFC round-trip with a roof with holes; test openskp C++/Rust read of `.skp` 2013–2026 samples we are licensed to use | No code adoption |
| D8 | Spike F: project store (SQLite single-file vs ZIP+JSON/Parquet) with autosave and crash-kill tests | No |
| D9 | Foundation recommendation v1 with spike measurements | **Owner approval** |

Spikes are disposable. They run in a private branch, ship to no one and are deleted or rewritten under the approved licence.
