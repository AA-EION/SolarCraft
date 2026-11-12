# 3. PV\*SOL premium feature matrix

The source of truth is [`feature-matrix.tsv`](feature-matrix.tsv). It has 167 rows, each with a stable ID, and opens in any spreadsheet. Columns:

`id · area · feature · baseline_version · evidence · sources · user_job · preconditions · inputs_units · outputs · edge_cases · reference_artifacts · candidate_implementation · platform_data_dependencies · phase · acceptance_test · status`

IDs never change or get reused. A dropped feature keeps its row with status `omitted-by-design` and a reason.

## Coverage summary

| Area | Rows | Highest-risk rows |
|---|---:|---|
| Project and desktop | 18 | PRJ-007 autosave/recovery, PRJ-008 portable project |
| Site, grid and climate | 21 | SITE-003 replacing Meteonorm, SITE-009 minute synthesis, SITE-016/017 Google Solar and Rexplorer equivalents |
| Layout and 3D | 21 | LAY-005 roof modeller, LAY-013/014 shading engine, LAY-017 scale |
| Electrical plant | 14 | ELEC-003 polystring I–V composition, ELEC-009 single-line diagram, ELEC-013 bypass diodes |
| Simulation | 20 | SIM-007 module model fits, SIM-011 adaptive step, SIM-012 loss ledger |
| Storage, demand, EV and thermal | 15 | STO-001 topology graph, STO-005 reserves, STO-003 detailed Li-ion |
| Economics | 14 | ECO-002 TOU calendar, ECO-005 dynamic tariffs, ECO-010 regional incentives |
| Results and reporting | 14 | RPT-001..003 three report layouts, RPT-006 24 report languages |
| Catalog and operations | 13 | CAT-010 hosted server, CAT-005 pinned snapshots |
| Interoperability | 12 | INT-002/003 SKP, INT-006 Revit, INT-009 PV\*SOL import |
| Automation (CLI/MCP) | 5 | MCP-003 transactions and confirmations |

Phases: S1 has 41 rows, S2 31, S3 55, S4 28 and S5 10. Two rows are `n/a` (omitted or awaiting owner). Status: 145 planned, 13 awaiting the owner, 4 awaiting a provider, 3 awaiting a fixture, 1 replaced by a published equivalent, 1 omitted by design.

Evidence: 57 rows rest on static recovery alone. **None is runtime-confirmed.** Before any parity claim on these rows, an owner-authorized live reference run or saved project has to confirm the behaviour.

## What the evidence does and does not support

- **Strong static evidence, with public equivalents:** transposition (SIM-003), module fit (SIM-007), inverter curve (ELEC-007), DC/AC battery control (STO-001/004), legacy economics (ECO-001..009), project container (PRJ-008), shading file layouts (LAY-013/014).
- **Static only, vendor-specific, not to be reproduced:** neural diffuse model (SIM-004) and minute transition matrices (SITE-009). Both are replaced with published methods. Trial and watermark logic (PRJ-016) is omitted by design.
- **Documented only (post-R4 or never analysed):** R5–R8 items (Rexplorer API, new map providers, load profiles) and 2027 R1 items (capacity tariffs, dynamic tariffs, peak shaving, larger limits).
- **Gaps still to audit** (they will become new rows, not silent omissions): the full menu tree beyond the 64 fixed subscriptions (dynamic menus), every help-manual chapter page by page, SolarEdge/optimizer rule details, mounting configurator payloads and a per-country tariff database. **Next step:** a chapter-by-chapter pass over the [page-by-page manual](https://help.valentin-software.com/pvsol/en/pages/). I did not complete that in this session. That pass is planned for D1–D2 and will probably add 30–60 rows.

## Deliberate improvements (tracked separately from parity)

PRJ-005 variants in one project, PRJ-007 transactional store, PRJ-014 CLI, SIM-012 conservation ledger checked every step, SIM-018 reproducible run manifests, STO-005 explicit reserve policies, LAY-017 ≥100k modules, CAT-005 content-hash pinning, MCP-001..005 automation, INT-008 export fidelity reports.

## Definition of done for a row

A row is `complete` only when all four hold: (1) the acceptance test runs in CI or in a documented manual protocol, (2) its UI exists in **both** shells, (3) the CLI/MCP path gives identical results where applicable, and (4) user docs exist. A UI mock, a decompiler listing or a backend API alone never completes a row.
