# Foundation spikes (D3–D8)

| Spike | Question | Status | Report |
|---|---|---|---|
| A | Rust core over a C ABI into WinUI 3 (x64/ARM64) and SwiftUI (universal) | **Pass** on all four targets in CI (Intel macOS via Rosetta only) | [A-ffi-shells.md](A-ffi-shells.md) |
| B | Shared wgpu viewport in native surfaces | **Not started.** It needs a GPU on real hardware (CI VMs have no reliable GPU). Planned with slice 1's 2D/3D layout view. | — |
| C | Shading engine accuracy and speed | Accuracy pass; speed pass by extrapolation | [C-shading.md](C-shading.md) |
| D | PV physics vs pvlib | Pass (machine precision) | [D-pv-chain.md](D-pv-chain.md) |
| E | IngeTrazo port and CAD exchange | Next. Port order: topology, triangulation, then importers via the headless Python helper. | — |
| F | Project store crash safety | Pass on Linux | [F-project-store.md](F-project-store.md) |
