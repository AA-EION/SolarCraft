# Spike A: core C ABI and native shells

## Built
- `crates/ffi`: C ABI (`include/solarcraft.h`). Every entry point catches panics and returns `SC_INTERNAL_ERROR` instead of aborting the host. Long jobs report progress through a callback, and the callback's return value cancels the job.
- `apps/windows`: WinUI 3 (Windows App SDK 2.6.5, .NET 10), unpackaged and self-contained, for x64 and ARM64. P/Invoke with function-pointer callbacks; `SolarCraft.exe --selftest <file>` runs headless.
- `apps/macos`: SwiftUI + Swift Charts. The SwiftPM system-library module maps the C header. Universal arm64 + x86_64 binary. `SolarCraftMac --selftest` runs headless.
- Both shells show the same quick design form (site, tilt, azimuth, modules), run the core on a background thread with progress and cancel, and show annual and monthly results.

## Results so far
| Check | Target | Linux (C) | Windows x64 | Windows ARM64 | macOS arm64 | macOS x86_64 |
|---|---|---|---|---|---|---|
| Annual run via FFI | works | pass | CI | CI | CI | CI (Rosetta) |
| 1 MiB array across FFI | < 5 ms | 0.45 ms | CI | CI | CI | CI |
| Cancellation | works | pass (Rust test) | CI | CI | CI | CI |
| Core panic contained | no abort | pass | CI | CI | CI | CI |

"CI" cells are **blocked**. GitHub Actions run 43299739190 (2026-11-12) failed every job within 2 s with no runner assigned and no logs. That pattern usually means Actions refused to start jobs (account spending limit, billing, or Actions disabled for private repositories), not a build error. The Windows and macOS shells are therefore **unverified**: not yet compiled or run. The x86_64 macOS slice runs under Rosetta, which is **not** Intel hardware (D-09).

## Decisions taken in the spike
- **C ABI + hand-written façades** rather than UniFFI for now. The ABI is small, and C# bindings for UniFFI are community-maintained. Revisit once the API surface grows past ~40 functions.
- The spike's macOS deployment target is 14 so it builds on current runners. The product target stays macOS 26 (ADR 0001).
