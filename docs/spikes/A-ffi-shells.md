# Spike A: core C ABI and native shells

## Built
- `crates/ffi`: C ABI (`include/solarcraft.h`). Every entry point catches panics and returns `SC_INTERNAL_ERROR` instead of aborting the host. Long jobs report progress through a callback, and the callback's return value cancels the job.
- `apps/windows`: WinUI 3 (Windows App SDK 2.6.5, .NET 10), unpackaged and self-contained, for x64 and ARM64. P/Invoke with function-pointer callbacks; `SolarCraft.exe --selftest <file>` runs headless.
- `apps/macos`: SwiftUI + Swift Charts. The SwiftPM system-library module maps the C header. Universal arm64 + x86_64 binary. `SolarCraftMac --selftest` runs headless.
- Both shells show the same quick design form (site, tilt, azimuth, modules), run the core on a background thread with progress and cancel, and show annual and monthly results.

## Results (GitHub Actions run 43307449143, 2026-11-12, all jobs green)

| Check | Target | Linux (C) | Windows x64 | Windows ARM64 (native) | macOS arm64 | macOS x86_64 slice (Rosetta) |
|---|---|---|---|---|---|---|
| Builds | — | pass | pass (WinUI 3, WinAppSDK 2.6.5, .NET 10) | pass (cross-built, run natively on `windows-11-arm`) | pass (universal binary, Xcode 26.6, macOS 26.7) | same universal binary |
| Annual run via FFI | works | pass | pass | pass (2425 kWh/kWp) | pass | pass (identical result) |
| 1 MiB array across FFI | < 5 ms | 0.45 ms | pass | 0.18 ms | pass | 0.71 ms after warm-up (8.2 ms cold: first-call translation) |
| Cancellation | works | pass | pass | pass | pass | pass |
| Core panic contained | no abort | pass | pass | pass | pass | pass |

Caveats:
- The x86_64 macOS slice ran under **Rosetta, not on Intel hardware** (D-09). It still needs a run on owner hardware.
- The self-contained Windows x64 output is 96 MB (bundled .NET + Windows App SDK). Trimming or a framework-dependent build is a packaging task.

## Decisions taken in the spike
- **C ABI + hand-written façades** rather than UniFFI for now. The ABI is small, and C# bindings for UniFFI are community-maintained. Revisit once the API surface grows past ~40 functions.
- The spike's macOS deployment target is 14 so it builds on current runners. The product target stays macOS 26 (ADR 0001).
