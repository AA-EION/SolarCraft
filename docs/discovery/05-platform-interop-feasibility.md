# 5. Platform, stack and interoperability feasibility plan

Proposed, pending spikes D3–D8 and owner approval. Platform facts were checked 2026-11-12.

## 5.1 Platform targets

| Target | Facts | Plan |
|---|---|---|
| Windows 11 x64 | Windows App SDK 2.x is current stable (2.4.0 on 2026-08-13 per [release channels](https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/release-channels); **verify** before pinning) | WinUI 3, C#/.NET 10 LTS, unpackaged + MSIX |
| Windows 11 ARM64 | ARM64 runtime installers are published for WinAppSDK | Native ARM64 build of shell and Rust core. CI on GitHub `windows-11-arm` runners (**verify** availability). Test on owner hardware (e.g. Snapdragon X). |
| macOS 26+ Apple silicon | Apple-silicon-only from macOS 27 ([AppleInsider](https://appleinsider.com/articles/26/06/08/macos-27-compatibility-list-focuses-entirely-on-apple-silicon)) | SwiftUI + AppKit where needed (viewport, documents), Swift 6 |
| macOS 26 Intel x86_64 | Tahoe 26 is the last Intel macOS. Rosetta ends after macOS 27. | Ship x86_64 slice while Apple's toolchain supports it. **Test on real Intel hardware** (owner Q6). Otherwise disclose "build-verified only". |
| Universal vs separate | Universal = one download, ~1.8× binary size; separate = smaller, two artefacts | **Recommend universal** `.app` (lipo of Rust staticlib per arch). Re-evaluate if size >300 MB. |

## 5.2 Proposed architecture

```
apps/windows (WinUI 3, C#)      apps/macos (SwiftUI/AppKit)
        │  C ABI / UniFFI               │  UniFFI Swift
        └───────────────┬───────────────┘
                 crates/solarcraft-ffi
   ┌──────────┬──────────┬──────────┬───────────┬──────────┐
 model     engine      geometry   viewport    io/adapters
 (schema,  (sim, dispatch, (B-rep, roofs,  (wgpu render  (weather, IFC,
 store,    economics,   shading BVH,  into native   SKP, DXF,
 undo)     ledger)      layout)     surface)     PV*SOL import)
   └──────────┴──── one computation path ────┴──────────┘
       crates/cli       crates/mcp-server       server/catalog (axum + Postgres)
```

- **Shared core language: Rust.** Memory-safe, cross-compiles cleanly to all four targets, strong geometry and GPU ecosystem, and makes the CLI/MCP/server reuse trivial. Alternative: C++20. That gives more mature CAD libraries (OCCT, CGAL; CGAL is GPL/commercial) but higher defect risk and harder packaging. A third option is Swift/C# per platform, which duplicates the engine and is rejected.
- **FFI:** UniFFI for Swift. For C#, either a hand-written C ABI with csbindgen or uniffi-bindgen-cs. Spike A picks one. Long jobs run on core threads, with progress and cancellation over callbacks.
- **Viewport:** a wgpu renderer inside `SwapChainPanel` (WinUI) and `CAMetalLayer` (macOS) keeps one renderer and one shading visualisation. The fallback is a native Metal and D3D12 renderer pair, at roughly twice the effort.
- **Project store:** a single-file SQLite (`.solarcraft`) with tables for the model, a change log (undo and history), pinned catalog snapshots, weather blobs and results as Parquet blobs. WAL mode gives crash safety. The PV\*SOL-style ZIP-rewrite model is rejected because a crash mid-write corrupts the file (PRJ-007).
- **HTTP:** reqwest + rustls with platform verifier roots. Credentials live in Windows Credential Manager / macOS Keychain.
- **Server:** the same Rust crates, axum, Postgres, object storage for signed dataset bundles (ed25519 manifests), and cursor-based change feeds.
- **Packaging/signing:** Windows uses MSIX + winget + an optional unpackaged ZIP, Authenticode (EV or Azure Trusted Signing). macOS uses Developer ID signing + notarization + a DMG, with Mac App Store optional (licence-dependent, see 02 §2). Updates go through Sparkle (MIT) on macOS and MSIX/App Installer on Windows. All updates are optional and never block the offline workflow.
- **Resource budgets (proposed):** cold start <2 s; 20 kWp roof annual hourly run <1 s; 10 MWp/20k modules with shading <60 s; idle memory <300 MB; 100k-module scene <2 GB at 60 fps on M1/Snapdragon X.
- **Accessibility:** UI Automation (Windows) and NSAccessibility (macOS) on all forms. Viewport selections are mirrored in an accessible object list.
- **CI:** GitHub Actions matrix (win-x64, win-arm64, macos-arm64, macos-x86_64 build; Intel tests on owner hardware or a hosted Intel Mac if still offered). Every artefact gets an SBOM, `cargo deny` licence gates and reproducible test baselines.

## 5.3 Monorepo layout (proposal for discussion)

```
crates/       model, engine, geometry, shading, viewport, io-weather, io-cad, io-pvsol, catalog-client, ffi, cli, mcp-server
apps/         windows/, macos/
server/       catalog/ (API, curation tools, billing adapter), deploy/
schemas/      project, equipment, results, MCP tool JSON Schemas (versioned)
fixtures/     public/ (redistributable), private-manifest.tsv (hash-only pointers to non-public lane)
docs/         discovery/, architecture/, decisions/ (ADRs), user/, mcp/, self-hosting/
tools/        release, sbom, provenance
```

## 5.4 Interoperability direction-by-version matrix (target and route)

| Format | Read | Write | Versions | Route | Verification | Risk |
|---|---|---|---|---|---|---|
| IFC | Yes | Yes | 2x3, 4, 4.3 | IfcOpenShell (LGPL) | Opens in Revit, Archicad, BlenderBIM; schema validation | Low |
| glTF/GLB | Yes | Yes | 2.0 | Own | Khronos validator | Low |
| OBJ, STL | Yes | Yes | — | Own | Blender | Low |
| COLLADA | Yes | Maybe | 1.4/1.5 | Own reader | Blender, SketchUp import | Medium |
| CityGML | Yes | No | 2.0, 3.0 | Own | German state LoD2 tiles | Medium |
| DXF | Yes | Yes | R12–2018 | Own / `dxf` crate | AutoCAD (if owner has a licence), LibreCAD | Low–medium |
| DWG | Yes (optional) | No (free route) | R13–2018+ | LibreDWG helper (GPL, separate) or ODA (paid, $3k–7.5k/yr, [ODA 2026 pricing](https://www.opendesign.com/agreements/2026/en/ODA%20Membership%20&%20Extension%20pricing.pdf)) | Compare with DXF export from AutoCAD | Medium–high |
| SketchUp .skp | Yes | Yes (later) | 2013–2026 read | openskp (MIT) | Rights-cleared corpus. Write is verified only in SketchUp (needs a SketchUp licence). | Medium. RE provenance (L4). |
| Revit .rvt/.rfa | Via IFC | Via IFC | Revit 2025–2027 | IFC; later a local add-in (Revit API, requires Revit) or APS Design Automation (cloud upload, paid; owner approval) | In Revit | High for native |
| PV\*SOL .pvprj | Yes | No | Project v15–24 | Clean-room importer | Authorized samples | Legal (L1) |
| PV\*SOL .wbv | Yes | No | 3-channel | Clean-room | Authorized samples | Legal (L1) |
| PVsyst PAN/OND | Yes | Yes | 6.x–8.x | Own (pvlib reader as reference) | Rights-cleared files | Low |
| EPW, TMY3, PVGIS CSV | Yes | Yes | — | Own | Public files | Low |
| IngeTrazo .igz | Via bridge | Via bridge | 0.6.x | GPL bridge (separate repo) | Round trip | Low technical, counsel L3 |

Every export writes a **fidelity report** (dropped fields, unit conversions, coordinate error, ID mapping). SolarCraft metadata travels in documented IFC property sets (`Pset_SolarCraft_*`) or a sidecar JSON. A native round trip that is not yet verified stays as an explicit parity gap in the matrix.

## 5.5 Spike acceptance criteria (for the foundation decision)

| Spike | Pass criteria |
|---|---|
| A FFI + shells | Signed hello-core apps on all 4 targets; a 1 MB result array crosses the FFI in <5 ms; cancellation works; crash in core produces a recoverable error, not a process abort |
| B Viewport | 10k instanced modules at 60 fps on M1 and Snapdragon X; 200k at ≥30 fps; pick/select <16 ms |
| C Shading | Analytic fixtures within 1% shaded area; 20k-module annual direct shading <60 s on 8 cores; results identical across platforms within 1e-6 |
| D PV chain | pvlib/SSC parity within the tolerances in §4.2 for 10 PVGIS sites |
| E Exchange | Roof-with-holes fixture survives IFC/glTF round trip with stable IDs; openskp reads ≥90% of the rights-cleared SKP corpus with counts matching |
| F Store | 1,000 kill tests with no corruption; 50 MB project opens <1 s |
