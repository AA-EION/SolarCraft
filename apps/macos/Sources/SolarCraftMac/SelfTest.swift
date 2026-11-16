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
        // Full offline workflow through the JSON API used by the UI.
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("sc-selftest-\(ProcessInfo.processInfo.processIdentifier)")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = dir.appendingPathComponent("t.solarcraft").path
        let pdf = dir.appendingPathComponent("r.pdf").path
        do {
            _ = try Core.invokeRaw("create_project", ["file": file, "name": "Selftest", "latitude": 6.2442, "longitude": -75.5812, "target_kwp": 4.0])
            let s = try Core.invoke("set_fields", ["file": file, "edits": [["pointer": "/arrays/0/tilt", "value": 12.0]]], as: SetResult.self)
            check(s.project.arrays[0].tilt.value == 12 && s.project.arrays[0].tilt.isUser, "set_fields decodes into UI models")
            let roof = try Core.invoke("set_roof", ["file": file, "outline": [[0.0, 0.0], [11.0, 0.0], [8.0, 5.0], [3.0, 5.0]], "obstacles": parseObstacles("5,2,0.8,0.8,2.0"), "obstacle_heights": parseObstacleHeights("5,2,0.8,0.8,2.0")], as: RoofResult.self)
            let doc = try Core.invoke("get_project", ["file": file], as: ProjectDoc.self)
            check((roof.modules_fit ?? 0) >= 8 && doc.arrays[0].roof?.modules.count == roof.modules_fit, "roof layout decodes (\(roof.modules_fit ?? 0) modules)")
            _ = try Core.invokeRaw("use_demo_weather", ["file": file])
            _ = try Core.invokeRaw("simulate", ["file": file])
            let e = try Core.invoke("explain", ["file": file], as: Explain.self)
            check(e.monthly.count == 12 && e.annual.ac_kwh > 1000, "workflow simulate/explain \(Int(e.annual.ac_kwh)) kWh")
            let shading = e.irradiance_kwh_m2.first { $0.key == "shading" }?.delta_pct ?? 0
            check(shading < 0, "near shading counted (\(shading) %)")
            let r = try Core.invoke("export_report", ["file": file, "output": pdf, "format": "pdf", "lang": "es"], as: ReportOutput.self)
            check(r.bytes > 20_000, "report PDF \(r.bytes) bytes")
            let ui = try Core.invoke("ui_strings", ["lang": "es"], as: [String: String].self)
            check(ui["simulate"] == "Simular", "UI strings (es)")
        } catch { check(false, "workflow: \(error.localizedDescription)") }
        // Plant workflow: stringing, consumption, DC-coupled battery with reserve, tariff, variants, report contents.
        do {
            let f = try DemoProject.build(in: dir.appendingPathComponent("plant"))
            let s = try Core.invokeJSON("results_summary", ["file": f])
            let b = s["balance"] as? JSONDict ?? [:]
            check(num(b, "batt_discharge_kwh") > 100 && num(b, "load_kwh") > 4000 && num(b, "max_abs_residual_wh") < 1e-3, "battery dispatch and closed balance (\(Int(num(b, "batt_discharge_kwh"))) kWh discharged)")
            let day = try Core.invokeJSON("results_hours", ["file": f, "day_of_year": 172])["hours"] as? [JSONDict] ?? []
            check(day.count == 24 && day.filter { ($0["outage"] as? Bool) == true }.count == 5, "hourly inspection with outage hours")
            let fin = try Core.invokeJSON("cash_flow", ["file": f])
            check((fin["years"] as? [JSONDict])?.count == 21, "cash flow 20 years")
            let cmp = try Core.invokeJSON("compare_variants", ["file": f])["variants"] as? [JSONDict] ?? []
            check(cmp.count == 2, "variant comparison")
            let prev = try Core.invokeJSON("report_preview", ["file": f, "lang": "es"])["sections"] as? [JSONDict] ?? []
            check(prev.count >= 4, "report plant sections (\(prev.count))")
            let bad = try Core.invokeJSON("set_section", ["file": f, "section": "storage", "value": ["battery_id": "x", "units": 1, "coupling": ["kind": "dc_hybrid", "unit_id": "nope"], "model": "simple", "soc_min_pct": 5.0, "soc_max_pct": 100.0, "initial_soc_pct": 50.0,
                "backup_reserve_pct": 70.0, "grid_services_pct": 40.0, "charge_priority": ["backup", "self_consumption", "grid_services"], "grid_charge_backup": false, "grid_service": ["kind": "none"], "outages": [Any]()] as [String: Any]])
            let codes = (bad["issues"] as? [JSONDict] ?? []).compactMap { $0["code"] as? String }
            check(codes.contains("BATTERY_MISSING"), "invalid storage reported (\(codes))")
        } catch { check(false, "plant workflow: \(error.localizedDescription)") }
        do { _ = try Core.invokeRaw("fetch_weather_pvgis", ["file": file]); check(false, "consent enforced") } catch let e as CoreError { check(e.code == "CONSENT_REQUIRED", "consent enforced") } catch { check(false, "consent enforced") }
        try? FileManager.default.removeItem(at: dir)
        return failures == 0 ? 0 : 1
    }
}
