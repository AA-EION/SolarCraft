// SPDX-License-Identifier: GPL-3.0-or-later
import AppKit
import SwiftUI

/// Builds a complete demo project through the JSON API (offline) and returns its path.
/// Shared by the self-test and the screenshot mode so both exercise the real workflow.
enum DemoProject {
    static func build(in dir: URL) throws -> String {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = dir.appendingPathComponent("Casa Usaquén.solarcraft").path
        try? FileManager.default.removeItem(atPath: file)
        _ = try Core.invokeRaw("create_project", ["file": file, "name": "Casa Usaquén", "latitude": 4.711, "longitude": -74.072, "target_kwp": 6.0])
        _ = try Core.invokeRaw("set_mode", ["file": file, "mode": "detailed"])
        _ = try Core.invokeRaw("use_demo_weather", ["file": file])
        let ids = try Core.invokeJSON("add_generic_storage_equipment", ["file": file])["ids"] as? [String] ?? []
        _ = try Core.invokeRaw("set_fields", ["file": file, "edits": [["pointer": "/arrays/0/inverter_id", "value": ids.first ?? ""], ["pointer": "/arrays/0/tilt", "value": 15.0]]])
        _ = try Core.invokeRaw("set_roof", ["file": file, "outline": [[0.0, 0.0], [12.0, 0.0], [12.0, 6.0], [0.0, 6.0]], "obstacles": [[[5.0, 4.0], [5.8, 4.0], [5.8, 4.8], [5.0, 4.8]]], "obstacle_heights": [1.5], "params": ["setback": 0.4, "obstacle_clearance": 0.3]])
        let st = try Core.invokeJSON("stringing", ["file": file])
        let unit = ((st["inverters"] as? [JSONDict])?.first?["unit"] as? JSONDict)?["id"] as? String ?? ""
        _ = try Core.invokeRaw("set_section", ["file": file, "section": "grid", "value": ["feed_in": "surplus", "export_limit_kw": 4.0, "phases": 1, "voltage_v": 120.0]])
        _ = try Core.invokeRaw("set_section", ["file": file, "section": "loads", "value": ["profile": ["kind": "synthetic", "profile": "residential"], "annual_kwh": 4200.0, "critical_fraction": 0.3]])
        _ = try Core.invokeRaw("set_section", ["file": file, "section": "storage", "value": [
            "battery_id": ids.count > 1 ? ids[1] : "", "units": 1, "coupling": ["kind": "dc_hybrid", "unit_id": unit], "model": "detailed", "soc_min_pct": 5.0, "soc_max_pct": 100.0, "initial_soc_pct": 50.0,
            "backup_reserve_pct": 30.0, "grid_services_pct": 0.0, "charge_priority": ["backup", "self_consumption", "grid_services"], "grid_charge_backup": false, "grid_service": ["kind": "none"],
            "outages": [["start_hour": 171 * 24 + 3, "hours": 5]]] as [String: Any]])
        _ = try Core.invokeRaw("set_section", ["file": file, "section": "tariff", "value": ["name": "TOU", "currency": "COP", "import_price": 800.0, "tou": [["name": "Peak", "price": 1100.0, "start_hour": 18, "end_hour": 22, "weekdays": true, "weekends": true, "months": [Int]()]],
            "export": ["kind": "fixed", "price": 300.0], "fixed_per_month": 12000.0, "demand_charge_per_kw_month": 0.0, "import_escalation_pct": 4.0, "export_escalation_pct": 2.0] as [String: Any]])
        _ = try Core.invokeRaw("set_section", ["file": file, "section": "economics", "value": ["horizon_years": 20, "discount_rate_pct": 9.0, "cost_fixed": 3_000_000.0, "cost_per_kwp": 3_500_000.0, "cost_per_battery_kwh": 1_800_000.0,
            "om_pct_of_investment": 1.0, "om_escalation_pct": 4.0, "inverter_replacement_year": 13, "inverter_replacement_cost": 4_000_000.0, "battery_replacement_year": 12, "battery_replacement_cost": 0.0, "degradation_pct_per_year": 0.4, "subsidy": 0.0, "loan": NSNull()] as [String: Any]])
        _ = try Core.invokeRaw("simulate", ["file": file])
        _ = try Core.invokeRaw("create_variant", ["file": file, "name": "Reserve 30 %"])
        return file
    }
}

/// `--screenshots <dir>`: render every workflow page of a simulated demo project to PNG.
enum Screenshots {
    @MainActor
    static func run(_ out: String) -> Int32 {
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.prohibited)
        let dir = URL(fileURLWithPath: out)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let model = AppModel()
        do { model.url = URL(fileURLWithPath: try DemoProject.build(in: dir.appendingPathComponent("project"))) } catch { print("FAIL demo project: \(error)"); return 1 }
        model.reload()
        model.loadDay(172)
        model.loadPreview("en")
        model.loadLibrary()
        model.variantRows = (try? Core.invokeJSON("compare_variants", ["file": model.url!.path])["variants"] as? [JSONDict]) ?? []
        var failures = 0
        for page in Page.allCases {
            let root = PageView(page: page).environmentObject(model).padding(20).frame(width: 1000, alignment: .topLeading).background(Color(nsColor: .windowBackgroundColor))
            let host = NSHostingView(rootView: root)
            let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1000, height: 800), styleMask: [.borderless], backing: .buffered, defer: false)
            window.contentView = host
            host.layoutSubtreeIfNeeded()
            let size = host.fittingSize
            let h = min(max(size.height, 400), 3200)
            window.setContentSize(NSSize(width: 1000, height: h))
            host.frame = NSRect(x: 0, y: 0, width: 1000, height: h)
            host.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.3))
            guard let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { failures += 1; continue }
            host.cacheDisplay(in: host.bounds, to: rep)
            let url = dir.appendingPathComponent(String(format: "%02d-%@.png", Page.allCases.firstIndex(of: page)! + 1, page.rawValue))
            do { try rep.representation(using: .png, properties: [:])?.write(to: url); print("PASS screenshot \(url.lastPathComponent) \(Int(h)) px") } catch { failures += 1; print("FAIL \(url.lastPathComponent): \(error)") }
        }
        if let e = model.error { print("note: model error: \(e)") }
        return failures == 0 ? 0 : 1
    }
}
