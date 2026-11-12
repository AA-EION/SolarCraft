# ADR 0001: Foundation

- Status: **Accepted** (owner, 2026-11-12). Stack items marked *pending spike* are confirmed or replaced by the spike reports in `docs/spikes/`.
- Context: [discovery documents](../discovery/README.md), owner decisions D-01..D-12 in [07](../discovery/07-owner-questions.md).

## Decisions

1. **Licensing.** SolarCraft code is GPL-3.0-or-later. The hosted catalog server is AGPL-3.0-or-later. Catalog datasets are proprietary and licensed separately. Contributions are licensed inbound under the project licence, with a DCO sign-off.
2. **IngeTrazo.** Its geometry and topology algorithms (GPL-3.0-or-later, © Marco Sumari Tellez and contributors) are ported to Rust inside `crates/geometry` with per-file attribution. Its Python importers run headless as a bundled helper until ported. The pinned upstream commit is `ffc7187bbaf8222ea5f5f19e33f6c4ed81ad4ad5` (v0.6.1).
3. **Core.** Rust workspace with one computation path for the GUI, CLI, MCP and server.
4. **Shells.** WinUI 3 / C# on Windows 11 x64 and ARM64. SwiftUI + AppKit on macOS 26+ as a universal binary (arm64 + x86_64). *FFI binding choice pending spike A.*
5. **Viewport.** wgpu shared renderer embedded in native surfaces. *Pending spike B.*
6. **Project store.** Single-file SQLite (`.solarcraft`) holding model, change log, pinned equipment snapshot, weather and results. *Pending spike F.*
7. **Modes.** One project model with an `Easy`/`Detailed` presentation mode. Every value records whether it was user-entered or derived. Switching never deletes data.
8. **Data.** Free and open providers only (D-10). The equipment catalog is owner- and user-authored. No vendor-extracted data is used.
9. **Markets.** Internationalised from day one: SI internal units, locale formatting, country packs for tariffs and incentives. Launch locales: en, de, es (incl. es-CO), pt-BR.
10. **Interop.** IFC, glTF/OBJ/COLLADA/CityGML, DXF/DWG (LibreDWG), SKP (openskp) and PV\*SOL project import are targets for the first public release.

## Consequences

- The Mac App Store is not a distribution channel. Use Developer ID + notarization, MSIX/winget.
- Every distributed binary ships with corresponding-source instructions and an SBOM.
