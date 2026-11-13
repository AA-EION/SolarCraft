# Slice 1: project → weather → layout → simulate → explain → report

Status on 2026-11-13 (CI run 23 green on Linux, Windows x64/ARM64, macOS arm64/x86_64-under-Rosetta). The row IDs refer to [feature-matrix.tsv](../discovery/feature-matrix.tsv).

| Step | Works today | Evidence | Rows |
|---|---|---|---|
| Project | Create/open/save single-file `.solarcraft`, undo, history, portable export; Easy/Detailed modes with value provenance | `crates/store`, `crates/model/tests/modes.rs`, 1,000-kill test (spike F) | PRJ-001 (partial), PRJ-007, PRJ-008, PRJ-013 (partial), PRJ-014 |
| Weather | PVGIS TMY fetch (consented), EPW and PVGIS JSON import, provenance, QC | `crates/weather` tests; live Bogotá fetch | SITE-001 (manual), SITE-004, SITE-007 (EPW), SITE-008 (basic) |
| Layout | Easy-mode sizing (tilt, mounting, inverter, strings within inverter capacity) and Detailed-mode manual counts. **Roof faces:** rectangle / hip-face trapezoid / custom outline (typed or clicked on a 1 m grid), rectangular obstacles, edge setback and obstacle clearance; modules placed with per-row offsets and the best orientation; Easy mode sizes from (and never exceeds) what fits; roof plan drawn in both shells and in PDF/DOCX reports. Triangulation ported from IngeTrazo | sweep test 1–400 kWp; layout matches hand counts with an independent sampling check; shell self-tests place modules through the UI API | LAY-001, LAY-002, LAY-008, LAY-012, LAY-018 (partial), ELEC-001, ELEC-002 |
| Simulate | Hourly SPA, five sky models, physical IAM, soiling, Faiman by mounting, CEC 6-parameter module (fitted from datasheet), MPP window, PVWatts inverter, clipping, night tare, AC/availability losses plus **near shading** from roof obstacles (faces placed in 3D, obstacles as prisms with heights, hourly ray-traced beam on 3×6 samples per module, sky-view factor for diffuse; own irradiation ledger step) | pvlib parity; chimney shading grows with height (0.15 % at 0.5 m → 4.1 % at 2.5 m on a 10 × 6 m array) (machine precision); PVGIS cross-check within 3.2% | SIM-001..003, SIM-005..007, SIM-012, SIM-015, SIM-017, SIM-018, ELEC-007 |
| Explain | Irradiation and energy waterfalls that close exactly; monthly table; coded notes | `crates/engine/tests/ledger.rs` | SIM-012, RPT-007 (partial) |
| Report | Customer report as PDF (Typst, tagged) and editable DOCX, in en/es/de/pt-BR | `crates/report/tests/render.rs`; LibreOffice opens the DOCX | RPT-001, RPT-011 (JSON via `--json`) |
| Automation | CLI and MCP server (stdio) over the same app layer | `crates/mcp/tests/stdio.rs` | MCP-001, MCP-002 (partial), MCP-003 (dry run, undo), MCP-005 |
| Native shells | WinUI 3 (x64, ARM64) and SwiftUI (universal): project → site & weather (consented PVGIS, EPW/PVGIS import, offline demo) → design (easy/detailed, calculated-value hints, checks) → results (KPIs, monthly chart, loss table) → report export; UI in en/es/de/pt-BR from one string table in the core | Self-tests run the full offline workflow through the same JSON API (`sc_invoke`) | PRJ-002 (partial), PRJ-006 (partial), PRJ-009 |

## Remaining for slice 1
1. A run on real desktops (owner): CI proves the shells build and that the API calls they make work; the look, feel and pointer editing need a person.
2. Electrical effect of partial shading: shaded cells drive bypass diodes, so string power drops more than the shaded area suggests (feature row ELEC-013, cell-string I–V with bypass diodes, pvmismatch as oracle).
3. Several arrays per project in the shells (the core already supports them).
