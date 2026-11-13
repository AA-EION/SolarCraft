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
        }
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
        run {
            let r = try Core.invoke("set_fields", ["file": file, "edits": [["pointer": pointer, "value": value]]], as: SetResult.self)
            project = r.project
            issues = r.issues
        }
    }

    func setRoof(outline: [[Double]]?, obstacles: [[[Double]]], heights: [Double], setback: Double, clearance: Double) {
        run {
            var params: [String: Any] = ["file": file, "array": 0]
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
                self.reload()
            }
        }
    }

    func exportReport(lang: String, format: String, designer: String) {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [format == "pdf" ? .pdf : (UTType(filenameExtension: "docx") ?? .data)]
        panel.nameFieldStringValue = (project?.name.value ?? "Report") + "." + format
        guard panel.runModal() == .OK, let u = panel.url else { return }
        run {
            let r = try Core.invoke("export_report", ["file": file, "output": u.path, "format": format, "lang": lang, "designer": designer], as: ReportOutput.self)
            notice = "\(t("report_saved")): \(r.path)"
            NSWorkspace.shared.open(u)
        }
    }
}
