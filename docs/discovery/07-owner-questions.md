# 7. Owner decisions, batch 1 (answered 2026-11-12)

These are recorded as approved decisions. The foundation record is [docs/decisions/0001-foundation.md](../decisions/0001-foundation.md).

| ID | Question | Owner decision | Consequences |
|---|---|---|---|
| D-01 | Licence and business model | **GPL-3.0 app with paid services** | Desktop app, core, CLI and MCP under GPL-3.0-or-later. Revenue comes from the hosted catalog, data, support and team services. Mac App Store distribution is excluded; direct download with notarization is used instead. |
| D-02 | Exact terms | **GPL-3.0-or-later; hosted catalog server under AGPL-3.0-or-later** | Catalog *data* is proprietary and licensed per subscription. Inbound contributions are licensed GPL-3.0-or-later, matching IngeTrazo. |
| D-03 | IngeTrazo | **Embed IngeTrazo code**: port its geometry/topology algorithms to the Rust core (with attribution) and run its Python importers headless as a bundled helper until ported | GPL-compatible. IngeTrazo is credited in `AUTHORS`/`NOTICE`. Upstream fixes are offered back. |
| D-04 | Reference material | Use the findings, **including the solar mathematics**, as fully as possible. **Do not** use catalog data extracted from PV\*SOL; the owner feeds the catalog manually or builds their own DB. Users can add their own equipment manually. | See [02 §1](02-commercial-licensing-memo.md#1-reference-material-policy). Counsel review is recommended before public release (L1). |
| D-05 | Markets | **All segments**: residential, C&I and utility; DACH/EU, US, **Colombia and Latin America** | Spanish (es-CO / LatAm) and Portuguese (pt-BR) join EN/DE early. Tariff, net-metering and incentive frameworks must be data-driven per country. Weather providers must cover LatAm (PVGIS covers the Americas; NASA POWER is global). |
| D-06 | Design modes | **Easy and detailed modes, switchable at any time mid-project in both directions, with no data loss** | One project model. Easy mode derives defaults and hides detail. Switching back never discards detailed data, and derived values are marked as such. |
| D-07 | CAD/BIM at first public release | SketchUp `.skp`, AutoCAD DXF/DWG, Revit via IFC, glTF/OBJ/CityGML, **COLLADA**, the formats PV\*SOL supports, and **PV\*SOL project import as a full SolarCraft project** (not only 3D) | DWG via LibreDWG (GPL, now compatible). PV\*SOL `.pvprj`/`.wbv` import in scope. |
| D-08 | Repository | **Will go public; citations moved** to a private evidence index outside the repo | Feature rows cite `EV-<id>` keys. Branch history was rewritten on 2026-11-12 (owner-approved); no commit contains bundle citations. |
| D-09 | Intel Mac | **Universal macOS build, Intel on macOS 26** | Intel slice is tested on owner hardware, or labelled build-verified only. |
| D-10 | Data budget | **Free/open data only for now** | PVGIS, NASA POWER, Overture/OSM, Copernicus DEM, CEC lists, plus owner/user manual catalog entry. Paid weather, imagery and CAD SDK rows stay visible as deferred. |
| D-11 | Parity baseline | **PV\*SOL premium 2026 (R4–R8); track 2027 R1** as next-version rows | |
| D-12 | Build start | **Approved: spikes, then vertical slice 1** | Spike reports go in `docs/spikes/`. Slice 1 covers project → weather → layout → simulate → explain → report, plus CLI/MCP and easy/detailed modes. |
| D-13 | Intel Mac verification | **Label the x86_64 macOS build "tested under emulation (Rosetta) only"** (2026-11-12) | Release notes and the download page carry the label until the owner supplies Intel hardware. |
