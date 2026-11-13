# Slice 1: project → weather → layout → simulate → explain → report

Status on 2026-11-12. The row IDs refer to [feature-matrix.tsv](../discovery/feature-matrix.tsv).

| Step | Works today | Evidence | Rows |
|---|---|---|---|
| Project | Create/open/save single-file `.solarcraft`, undo, history, portable export; Easy/Detailed modes with value provenance | `crates/store`, `crates/model/tests/modes.rs`, 1,000-kill test (spike F) | PRJ-001 (partial), PRJ-007, PRJ-008, PRJ-013 (partial), PRJ-014 |
| Weather | PVGIS TMY fetch (consented), EPW and PVGIS JSON import, provenance, QC | `crates/weather` tests; live Bogotá fetch | SITE-001 (manual), SITE-004, SITE-007 (EPW), SITE-008 (basic) |
| Layout | Easy-mode sizing (tilt, mounting, inverter, strings within inverter capacity), Detailed-mode manual counts; **no 2D roof editor yet** | sweep test, 1–400 kWp, −55° to 70° | LAY-001, ELEC-001, ELEC-002 |
| Simulate | Hourly SPA, five sky models, physical IAM, soiling, Faiman by mounting, CEC 6-parameter module (fitted from datasheet), MPP window, PVWatts inverter, clipping, night tare, AC/availability losses | pvlib parity (machine precision); PVGIS cross-check within 3.2% | SIM-001..003, SIM-005..007, SIM-012, SIM-015, SIM-017, SIM-018, ELEC-007 |
| Explain | Irradiation and energy waterfalls that close exactly; monthly table; coded notes | `crates/engine/tests/ledger.rs` | SIM-012, RPT-007 (partial) |
| Report | Customer report as PDF (Typst, tagged) and editable DOCX, in en/es/de/pt-BR | `crates/report/tests/render.rs`; LibreOffice opens the DOCX | RPT-001, RPT-011 (JSON via `--json`) |
| Automation | CLI and MCP server (stdio) over the same app layer | `crates/mcp/tests/stdio.rs` | MCP-001, MCP-002 (partial), MCP-003 (dry run, undo), MCP-005 |
| Native shells | WinUI 3 (x64, ARM64) and SwiftUI (universal): project → site & weather (consented PVGIS, EPW/PVGIS import, offline demo) → design (easy/detailed, calculated-value hints, checks) → results (KPIs, monthly chart, loss table) → report export; UI in en/es/de/pt-BR from one string table in the core | Self-tests run the full offline workflow through the same JSON API (`sc_invoke`) | PRJ-002 (partial), PRJ-006 (partial), PRJ-009 |

## Remaining for slice 1
1. Interactive check of both shells on real desktops: CI proves they build and that the API calls the UI makes work, not the look and feel. Screenshots from a real machine are needed.
2. 2D roof layout (polygon, setbacks, module placement), starting the IngeTrazo geometry port.
3. Port SAM's 6-parameter solver so every CEC datasheet fits exactly (SIM-007).
