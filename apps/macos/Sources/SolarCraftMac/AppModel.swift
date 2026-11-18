// SPDX-License-Identifier: GPL-3.0-or-later
import AppKit
import Foundation
import UniformTypeIdentifiers

@MainActor
final class AppModel: ObservableObject {
    @Published var strings: [String: String] = [:]
    @Published var url: URL?
    @Published var project: ProjectDoc?
    @Published var issues: [Issue] = []
    @Published var results: Explain?
    @Published var busy = false
    @Published var progress = 0.0
    @Published var error: String?
    @Published var notice: String?
    /// Raw project and result documents for the plant pages (JSON objects from the core).
    @Published var raw: JSONDict = [:]
    @Published var summary: JSONDict?
    @Published var designSchedule: JSONDict?
    @Published var workflow: JSONDict = [:]
    @Published private(set) var completedRuns = 0
    @Published var stringingDoc: JSONDict?
    @Published var dayRows: [JSONDict] = []
    @Published var cash: JSONDict?
    @Published var variantRows: [JSONDict] = []
    @Published var libraryItems: [JSONDict] = []
    @Published var catalogHits: [JSONDict] = []
    private var running: Core.Progress?

    static let projectType = UTType(filenameExtension: "solarcraft") ?? .data
    let uiLang = Locale.preferredLanguages.first ?? "en"

    init() {
        strings = (try? Core.invoke("ui_strings", ["lang": uiLang], as: [String: String].self)) ?? [:]
    }

    func t(_ key: String) -> String { strings[key] ?? key }
    private var file: String { url?.path ?? "" }

    private func run(_ body: () throws -> Void) {
        do { try body(); error = nil } catch { self.error = error.localizedDescription }
    }

    func reload() {
        guard url != nil else { return }
        run {
            project = try Core.invoke("get_project", ["file": file], as: ProjectDoc.self)
            issues = try Core.invoke("validate", ["file": file], as: IssuesResult.self).issues
            results = try? Core.invoke("explain", ["file": file], as: Explain.self)
            raw = (try? Core.invokeJSON("get_project", ["file": file])) ?? [:]
            workflow = (try? Core.invokeJSON("workflow_get", ["file": file])) ?? [:]
            summary = try? Core.invokeJSON("results_summary", ["file": file])
            stringingDoc = try? Core.invokeJSON("stringing", ["file": file])
            designSchedule = try? Core.invokeJSON("design_schedule", ["file": file])
            cash = try? Core.invokeJSON("cash_flow", ["file": file])
        }
    }

    // MARK: plant operations (all through the same JSON API as the CLI and MCP)

    func section(_ name: String) -> JSONDict { raw[name] as? JSONDict ?? [:] }

    func setSection(_ name: String, _ value: Any) {
        run {
            let r = try Core.invokeJSON("set_section", ["file": file, "section": name, "value": value])
            let list = (r["issues"] as? [JSONDict]) ?? []
            let errs = list.filter { ($0["severity"] as? String) == "error" }.compactMap { $0["message"] as? String }
            notice = errs.isEmpty ? t("apply") + " ✓" : nil
            if !errs.isEmpty { error = errs.joined(separator: "\n") }
        }
        reload()
    }

    var systemType: String { workflowSettings["system"] as? String ?? "custom" }
    var workflowSettings: JSONDict { workflow["settings"] as? JSONDict ?? [:] }
    var isIslanded: Bool { workflow["islanded"] as? Bool ?? false }
    func setWorkflow(system: String, tools: String) {
        run { workflow = try Core.invokeJSON("workflow_set", ["file": file, "value": ["system": system, "tools": tools]]) }
        if error == nil { reload() }
    }

    func addArray() -> Int? {
        var index: Int?
        run { index = try Core.invokeJSON("add_array", ["file": file])["array"] as? Int }
        if error == nil { reload() }
        return index
    }

    func cablePlan(mode: String, circuits: [JSONDict]? = nil) -> JSONDict? {
        var result: JSONDict?
        run {
            var request: JSONDict = ["file": file, "mode": mode]
            if let circuits { request["circuits"] = circuits }
            result = try Core.invokeJSON("cable_plan", request)
        }
        if error == nil && (["apply", "apply_hourly", "apply_routed", "save"].contains(mode)) { reload() }
        return result
    }

    func saveIsland(plan: JSONDict, costs: JSONDict, currency: String) {
        let edits: [JSONDict] = [
            ["pointer": "/island_plan", "value": plan],
            ["pointer": "/tariff/currency", "value": currency],
        ] + costs.map { ["pointer": "/economics/" + $0.key, "value": $0.value] }
        do {
            _ = try Core.invokeJSON("set_fields", ["file": file, "edits": edits])
            reload()
        } catch { self.error = error.localizedDescription }
    }

    func setCableLosses(dc: Double, ac: Double) {
        let edits: [JSONDict] = [
            ["pointer": "/losses/dc_wiring_pct", "value": dc],
            ["pointer": "/losses/ac_wiring_pct", "value": ac],
            ["pointer": "/hourly_cables", "value": JSONDict()],
        ]
        do {
            _ = try Core.invokeJSON("set_fields", ["file": file, "edits": edits])
            reload()
        } catch { self.error = error.localizedDescription }
    }

    func setStringing(manual: Bool, inverters: [JSONDict]) {
        run { stringingDoc = try Core.invokeJSON("set_stringing", ["file": file, "manual": manual, "inverters": inverters]) }
        reload()
    }

    func addEquipment(_ kind: String, _ record: JSONDict, saveToLibrary: Bool) {
        run {
            _ = try Core.invokeRaw("add_equipment", ["file": file, "kind": kind, "record": record])
            if saveToLibrary { _ = try Core.invokeRaw("library_save", ["kind": kind, "record": record]) }
            notice = "\(kind) \(record["id"] as? String ?? "") ✓"
        }
        if error == nil { reload() }
    }

    func addGenericStorage() { run { _ = try Core.invokeRaw("add_generic_storage_equipment", ["file": file]) }; reload() }

    func loadLibrary(_ query: String = "") {
        run { libraryItems = (try Core.invokeJSON("library_list", query.isEmpty ? [:] : ["query": query])["items"] as? [JSONDict]) ?? [] }
    }

    func pin(_ kind: String, _ id: String) { run { _ = try Core.invokeRaw("library_pin", ["file": file, "kind": kind, "id": id]) }; reload() }

    func importLoad() {
        let panel = NSOpenPanel()
        guard panel.runModal() == .OK, let u = panel.url else { return }
        run { _ = try Core.invokeRaw("import_load", ["file": file, "source": u.path]) }
        reload()
    }

    func loadDay(_ day: Int) { run { dayRows = (try Core.invokeJSON("results_hours", ["file": file, "day_of_year": day])["hours"] as? [JSONDict]) ?? [] } }
    func loadTop(_ channel: String) { run { dayRows = (try Core.invokeJSON("results_hours", ["file": file, "top": channel, "count": 24])["hours"] as? [JSONDict]) ?? [] } }

    func saveVariant(_ name: String) { run { _ = try Core.invokeRaw("create_variant", ["file": file, "name": name]) }; reload() }
    func applyVariant(_ id: String) { run { _ = try Core.invokeRaw("apply_variant", ["file": file, "id": id, "keep_current": true]) }; reload() }
    func compareVariants() {
        busy = true
        let f = file
        Task.detached {
            let rows = (try? Core.invokeJSON("compare_variants", ["file": f])["variants"] as? [JSONDict]) ?? []
            let data = (try? JSONSerialization.data(withJSONObject: rows)) ?? Data()
            await MainActor.run {
                self.variantRows = (try? JSONSerialization.jsonObject(with: data) as? [JSONDict]) ?? []
                self.busy = false
            }
        }
    }

    func catalogSearch(_ family: String, _ query: String) {
        run { catalogHits = (try Core.invokeJSON("catalog_search", ["family": family, "query": query, "limit": 50])["hits"] as? [JSONDict]) ?? [] }
    }
    func catalogPin(_ family: String, _ id: String) { run { _ = try Core.invokeRaw("catalog_pin", ["file": file, "family": family, "id": id]); notice = "\(family) ✓" }; reload() }
    func catalogTariff(_ family: String, _ id: String) {
        run {
            let side = family == "feedintariff" ? "export" : "import"
            let r = try Core.invokeJSON("catalog_apply_tariff", ["file": file, side: ["family": family, "id": id]])
            let applied = (r["applied"] as? [JSONDict])?.first
            notice = ((applied?["notes"] as? [String]) ?? []).joined(separator: "; ")
        }
        reload()
    }
    func importCatalog() {
        let panel = NSOpenPanel()
        guard panel.runModal() == .OK, let u = panel.url else { return }
        background("catalog_import", ["source": u.path])
    }

    func reportContents(lang: String, reportType: String, designer: String) -> [JSONDict] {
        var contents: [JSONDict] = []
        run {
            let report = try Core.invokeJSON("report_preview", ["file": file, "lang": lang, "report_type": reportType, "designer": designer, "include_figures": true])
            contents = dicts(report["contents"])
        }
        return contents
    }

    func newProject() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [Self.projectType]
        panel.nameFieldStringValue = "Project.solarcraft"
        guard panel.runModal() == .OK, let u = panel.url else { return }
        try? FileManager.default.removeItem(at: u)
        run { _ = try Core.invokeRaw("create_project", ["file": u.path, "name": u.deletingPathExtension().lastPathComponent, "latitude": 4.711, "longitude": -74.072]) }
        url = u
        reload()
    }

    func openProject() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [Self.projectType]
        guard panel.runModal() == .OK, let u = panel.url else { return }
        url = u
        reload()
    }

    /// Set one field by JSON pointer (becomes user-owned; the rest is re-derived).
    func set(_ pointer: String, _ value: Any) {
        do {
            _ = try Core.invokeJSON("set_fields", ["file": file, "edits": [["pointer": pointer, "value": value]]])
            reload()
        } catch { self.error = error.localizedDescription }
    }

    func setRoof(array: Int, outline: [[Double]]?, obstacles: [[[Double]]], heights: [Double], setback: Double, clearance: Double) {
        run {
            var params: [String: Any] = ["file": file, "array": array, "array": 0]
            if let o = outline {
                params["outline"] = o
                params["obstacles"] = obstacles
                params["obstacle_heights"] = heights
                params["params"] = ["setback": setback, "obstacle_clearance": clearance]
            } else {
                params["outline"] = NSNull()
            }
            let r = try Core.invoke("set_roof", params, as: RoofResult.self)
            if let n = r.modules_fit { notice = "\(n) \(t("modules_fit"))" }
        }
        reload()
    }

    func setMode(_ mode: String) { run { _ = try Core.invokeRaw("set_mode", ["file": file, "mode": mode]) }; reload() }
    func undo() { run { _ = try Core.invokeRaw("undo", ["file": file]) }; reload() }

    func fetchPVGIS() {
        let alert = NSAlert()
        alert.messageText = t("fetch_pvgis")
        alert.informativeText = t("fetch_consent")
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: t("cancel"))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        background("fetch_weather_pvgis", ["file": file, "confirm_network": true])
    }

    func importWeather() {
        let panel = NSOpenPanel()
        guard panel.runModal() == .OK, let u = panel.url else { return }
        run { _ = try Core.invokeRaw("import_weather", ["file": file, "source": u.path]) }
        reload()
    }

    func demoWeather() { run { _ = try Core.invokeRaw("use_demo_weather", ["file": file]) }; reload() }

    func simulate() { background("simulate", ["file": file]) }
    func cancel() { running?.cancel() }

    private func background(_ method: String, _ params: [String: Any]) {
        busy = true
        progress = 0
        let p = Core.Progress { f in Task { @MainActor in self.progress = f } }
        running = p
        let paramsData = (try? JSONSerialization.data(withJSONObject: params)) ?? Data()
        Task.detached {
            let params = (try? JSONSerialization.jsonObject(with: paramsData) as? [String: Any]) ?? [:]
            var message: String?
            do { _ = try Core.invokeRaw(method, params, progress: p) } catch { message = error.localizedDescription }
            await MainActor.run {
                self.busy = false
                self.error = message
                self.running = nil
                self.reload()
                if method == "simulate" && message == nil { self.completedRuns += 1 }
            }
        }
    }

    func exportReport(lang: String, format: String, designer: String, reportType: String, excludeSections: [String]) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [format == "pdf" ? .pdf : (UTType(filenameExtension: "docx") ?? .data)]
        panel.nameFieldStringValue = (project?.name.value ?? "Report") + "." + format
        guard panel.runModal() == .OK, let u = panel.url else { return }
        run {
            let r = try Core.invoke("export_report", ["file": file, "output": u.path, "format": format, "lang": lang, "designer": designer, "report_type": reportType, "exclude_sections": excludeSections], as: ReportOutput.self)
            notice = "\(t("report_saved")): \(r.path)"
            NSWorkspace.shared.open(u)
        }
    }
}
