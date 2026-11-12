// SPDX-License-Identifier: GPL-3.0-or-later
import CSolarCraft
import Foundation

/// Headless checks used by CI on each architecture slice.
enum SelfTest {
    static func run() -> Int32 {
        var failures = 0
        func check(_ ok: Bool, _ msg: String) { print((ok ? "PASS " : "FAIL ") + msg); if !ok { failures += 1 } }
        #if arch(arm64)
        let arch = "arm64"
        #else
        let arch = "x86_64"
        #endif
        print("solarcraft selftest arch=\(arch) core=\(Core.version) os=\(ProcessInfo.processInfo.operatingSystemVersionString)")
        let input = ScQuickInput(latitude: 4.711, longitude: -74.072, elevation_m: 2640, tilt_deg: 10, azimuth_deg: 180, modules: 10, year: 2026)
        let job = Core.Job(onProgress: { _ in })
        do {
            let r = try Core.quickAnnual(input, job: job)
            check(r.summary.steps == 8760 && r.summary.specific_yield_kwh_per_kwp > 1500, "annual run yield=\(r.summary.specific_yield_kwh_per_kwp)")
        } catch { check(false, "annual run threw \(error)") }
        let cj = Core.Job(onProgress: { _ in }); cj.cancel()
        do { _ = try Core.quickAnnual(input, job: cj); check(false, "cancel") } catch Core.Failure.cancelled { check(true, "cancel") } catch { check(false, "cancel \(error)") }
        var buf = [Double](repeating: 0, count: 131_072)
        _ = buf.withUnsafeMutableBufferPointer { sc_fill_probe($0.baseAddress, $0.count) } // warm-up (first call pays page faults / translation)
        let t0 = DispatchTime.now().uptimeNanoseconds
        let sum = buf.withUnsafeMutableBufferPointer { sc_fill_probe($0.baseAddress, $0.count) }
        let ms = Double(DispatchTime.now().uptimeNanoseconds - t0) / 1e6
        check(sum == 4_294_934_528 && ms < 5, "1 MiB FFI probe \(ms) ms")
        check(sc_panic_probe() == Int32(SC_INTERNAL_ERROR), "core panic is contained")
        return failures == 0 ? 0 : 1
    }
}
