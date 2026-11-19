// SPDX-License-Identifier: GPL-3.0-or-later
import AppKit
import Charts
import SwiftUI

@main
enum Entry {
    static func main() {
        if CommandLine.arguments.contains("--selftest") { exit(SelfTest.run()) }
        if let i = CommandLine.arguments.firstIndex(of: "--screenshots"), i + 1 < CommandLine.arguments.count {
            let out = CommandLine.arguments[i + 1]
            exit(MainActor.assumeIsolated { Screenshots.run(out) })
        }
        SolarCraftApp.main()
    }
}

struct SolarCraftApp: App {
    @StateObject private var model = AppModel()
    var body: some Scene {
        WindowGroup(model.t("app_title")) {
            RootView().environmentObject(model).frame(minWidth: 900, minHeight: 600)
        }
        .commands {
            CommandGroup(replacing: .newItem) {
                Button(model.t("new_project")) { model.newProject() }.keyboardShortcut("n")
                Button(model.t("open_project")) { model.openProject() }.keyboardShortcut("o")
            }
            CommandGroup(replacing: .undoRedo) {
                Button(model.t("undo")) { model.undo() }.keyboardShortcut("z").disabled(model.url == nil)
            }
        }
    }
}

enum Page: String, CaseIterable, Identifiable {
    case project, workflow_site, loads, ev_page, heat_page, workflow_pv_modules, roof, scene3d, electrical, battery, workflow_cables, workflow_plans, economics, island_page, workflow_results, workflow_presentation
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .project: return "doc.text"
        case .workflow_site: return "sun.max"
        case .loads: return "bolt.circle"
        case .ev_page: return "ev.charger"
        case .heat_page: return "heat.waves"
        case .island_page: return "powersleep"
        case .workflow_pv_modules: return "rectangle.split.2x2"
        case .roof: return "house"
        case .scene3d: return "cube"
        case .electrical: return "bolt"
        case .battery: return "battery.75percent"
        case .workflow_cables: return "point.bottomleft.forward.to.point.topright.scurvepath"
        case .workflow_plans: return "list.bullet.rectangle"
        case .economics: return "banknote"
        case .workflow_results: return "chart.bar"
        case .workflow_presentation: return "doc.richtext"
        }
    }
    var workflowTag: String {
        switch self {
        case .workflow_site: return "site"
        case .ev_page: return "ev"
        case .heat_page: return "heating"
        case .island_page: return "island"
        case .workflow_pv_modules: return "design"
        case .scene3d: return "scene"
        case .battery: return "storage"
        case .workflow_cables: return "cables"
        case .workflow_plans: return "plans"
        case .workflow_results: return "results"
        case .workflow_presentation: return "report"
        default: return rawValue
        }
    }
    static let groups: [(String, [Page])] = [
        ("workflow_setup", [.project, .workflow_site, .loads, .ev_page, .heat_page]),
        ("workflow_system", [.workflow_pv_modules, .roof, .scene3d, .electrical, .battery, .workflow_cables, .workflow_plans]),
        ("workflow_output", [.economics, .island_page, .workflow_results, .workflow_presentation]),
    ]
}

struct RootView: View {
    @EnvironmentObject var m: AppModel
    @State private var page: Page? = .project
    private var steps: [Page] {
        guard let visible = m.workflow["steps"] as? [String] else { return Page.allCases }
        return Page.allCases.filter { visible.contains($0.workflowTag) }
    }
    private var canSimulate: Bool {
        m.project?.weather != nil && !m.busy && !m.issues.contains { $0.severity == "error" }
    }
    var body: some View {
        NavigationSplitView {
            List(selection: $page) {
                ForEach(Page.groups.indices, id: \.self) { index in
                    let (group, pages) = Page.groups[index]
                    Section(m.t(group)) {
                        ForEach(pages.filter { steps.contains($0) }) { p in Label(m.t(p.rawValue), systemImage: p.icon).tag(p) }
                    }
                }
            }.disabled(m.project == nil || m.busy).navigationSplitViewColumnWidth(220)
        } detail: {
            VStack(spacing: 0) {
                if let e = m.error { Text(e).foregroundStyle(.white).padding(8).frame(maxWidth: .infinity).background(.red.opacity(0.85)) }
                if let n = m.notice { Text(n).padding(6).frame(maxWidth: .infinity).background(.green.opacity(0.15)) }
                if m.project == nil {
                    VStack(spacing: 12) {
                        Text(m.t("workflow_welcome")).font(.title2.bold())
                        Text(m.t("workflow_welcome_hint")).foregroundStyle(.secondary)
                        HStack {
                            Button(m.t("new_project")) { m.newProject() }.buttonStyle(.borderedProminent)
                            Button(m.t("open_project")) { m.openProject() }
                        }
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    let current = page ?? .project
                    let index = steps.firstIndex(of: current) ?? 0
                    HStack(alignment: .top, spacing: 12) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(m.t(current.rawValue)).font(.title2.bold()).accessibilityAddTraits(.isHeader)
                            if current == .workflow_presentation && m.summary == nil {
                                Text(m.t("workflow_needs_results")).foregroundStyle(.secondary)
                            } else if current == .workflow_results && m.project?.weather == nil {
                                Text(m.t("no_weather")).foregroundStyle(.secondary)
                            }
                        }
                        Spacer()
                        if index > 0 {
                            Button(m.t("workflow_back")) { page = steps[index - 1] }
                        }
                        if index + 1 < steps.count {
                            Button(m.t("workflow_next")) { page = steps[index + 1] }
                                .disabled(current == .workflow_results && m.summary == nil)
                        }
                    }.padding(.horizontal, 20).padding(.vertical, 10).disabled(m.busy)
                    Divider()
                    ScrollView {
                        VStack(alignment: .leading, spacing: 14) {
                            if current == .workflow_results {
                                if m.project?.weather == nil {
                                    Button(m.t("workflow_site")) { page = .workflow_site }
                                } else {
                                    Button(m.t("simulate")) { m.simulate() }.buttonStyle(.borderedProminent).disabled(!canSimulate)
                                    IssueList(issues: m.issues.filter { $0.severity == "error" }.map {
                                        ["severity": $0.severity, "message": $0.message]
                                    })
                                }
                            }
                            PageView(page: current).id(current)
                        }.padding(20).frame(maxWidth: 980, alignment: .leading)
                    }.disabled(m.busy)
                }
            }
            .onChange(of: m.completedRuns) { _, _ in page = .workflow_results }
            .onChange(of: m.url) { _, _ in page = .project }
            .onChange(of: steps) { _, visible in
                if let page, !visible.contains(page) { self.page = .workflow_site }
            }
            .toolbar {
                if m.busy { ProgressView(value: m.progress).frame(width: 120); Button(m.t("cancel")) { m.cancel() } }
                Button { m.simulate() } label: { Label(m.t("simulate"), systemImage: "play.fill") }.disabled(!canSimulate)
            }
        }
    }
}

struct PageView: View {
    let page: Page
    var includeReportPreview = false
    @ViewBuilder var body: some View {
        switch page {
        case .project: ProjectPage()
        case .workflow_site: SiteClimatePage()
        case .loads: StorageLoadsPage(consumptionOnly: true)
        case .ev_page: DemandEditorPage(section: "ev")
        case .heat_page: DemandEditorPage(section: "heating")
        case .island_page: IslandControlsPage()
        case .workflow_pv_modules: DesignPage()
        case .roof: RoofPage()
        case .scene3d: ScenePage()
        case .electrical: ElectricalPage()
        case .battery: StorageLoadsPage(consumptionOnly: false)
        case .workflow_cables: CablesPage()
        case .workflow_plans: EngineeringPlansPage()
        case .workflow_results: SimulationPage()
        case .economics: EconomicsPage()
        case .workflow_presentation: ReportPage(includePreview: includeReportPreview)
        }
    }
}

/// Numeric field that saves on Return/focus loss and shows whether the value is calculated.
struct NumberRow: View {
    @EnvironmentObject var m: AppModel
    let label: String
    let field: Field<Double>
    let pointer: String
    @State private var text = ""
    var body: some View {
        LabeledContent(label) {
            HStack {
                TextField("", text: $text).frame(width: 120).multilineTextAlignment(.trailing)
                    .onSubmit { if let v = Double(text.replacingOccurrences(of: ",", with: ".")) { m.set(pointer, v) } }
                Text(field.isUser ? " " : m.t("derived")).font(.caption).foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
            }
        }
        .onAppear { text = String(format: "%g", field.value) }
        .onChange(of: field.value) { _, v in text = String(format: "%g", v) }
    }
}

struct IntRow: View {
    @EnvironmentObject var m: AppModel
    let label: String
    let field: Field<Int>
    let pointer: String
    var body: some View {
        LabeledContent(label) {
            HStack {
                Stepper(value: Binding(get: { field.value }, set: { m.set(pointer, $0) }), in: 1...500) { Text("\(field.value)").monospacedDigit() }
                Text(field.isUser ? " " : m.t("derived")).font(.caption).foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
            }
        }
    }
}

struct TextRow: View {
    @EnvironmentObject var m: AppModel
    let label: String
    let value: String
    let pointer: String
    @State private var text = ""
    var body: some View {
        LabeledContent(label) { TextField("", text: $text).frame(width: 320).onSubmit { m.set(pointer, text) } }
            .onAppear { text = value }
    }
}

struct ProjectPage: View {
    @EnvironmentObject var m: AppModel
    var body: some View {
        if let p = m.project {
            Form {
                TextRow(label: m.t("name"), value: p.name.value, pointer: "/name")
                TextRow(label: m.t("customer"), value: p.customer.name, pointer: "/customer/name")
                TextRow(label: m.t("address"), value: p.customer.address, pointer: "/customer/address")
            }
        }
    }
}

struct SitePage: View {
    @EnvironmentObject var m: AppModel
    var body: some View {
        if let p = m.project {
            Form {
                NumberRow(label: m.t("latitude"), field: p.site.latitude, pointer: "/site/latitude")
                NumberRow(label: m.t("longitude"), field: p.site.longitude, pointer: "/site/longitude")
                NumberRow(label: m.t("elevation"), field: p.site.elevation, pointer: "/site/elevation")
                NumberRow(label: m.t("albedo"), field: p.site.albedo, pointer: "/site/albedo")
                Section(m.t("weather")) {
                    if let w = p.weather { Text(w.provider).bold(); Text(w.summary).foregroundStyle(.secondary) } else { Text(m.t("no_weather")).foregroundStyle(.secondary) }
                    HStack {
                        Button(m.t("fetch_pvgis")) { m.fetchPVGIS() }
                        Button(m.t("import_weather")) { m.importWeather() }
                        Button(m.t("demo_weather")) { m.demoWeather() }
                    }
                    Text(m.t("demo_warning")).font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }
}

struct DesignPage: View {
    @EnvironmentObject var m: AppModel
    @State private var selectedArray = 0
    var body: some View {
        if let p = m.project, !p.arrays.isEmpty {
            let index = min(selectedArray, p.arrays.count - 1)
            let a = p.arrays[index]
            let rawArrays = dicts(m.raw["arrays"])
            let tracking = rawArrays.indices.contains(index) ? rawArrays[index]["tracking"] as? JSONDict : nil
            let arrayPath = "/arrays/\(index)"
            Form {
                Picker(m.t("photo_array"), selection: $selectedArray) {
                    ForEach(p.arrays.indices, id: \.self) { n in Text(p.arrays[n].name.value).tag(n) }
                }
                Picker(m.t("module"), selection: Binding(get: { a.module_id.value }, set: { m.set(arrayPath + "/module_id", $0) })) {
                    ForEach(p.library.modules, id: \.id) { Text("\($0.manufacturer) \($0.model)").tag($0.id) }
                }
                Picker(m.t("inverter"), selection: Binding(get: { a.inverter_id.value }, set: { m.set(arrayPath + "/inverter_id", $0) })) {
                    ForEach(p.library.inverters, id: \.id) { Text("\($0.manufacturer) \($0.model)").tag($0.id) }
                }.disabled(m.stringingDoc?["manual"] as? Bool == true)
                Picker(m.t("mode"), selection: Binding(get: { p.mode }, set: { m.setMode($0) })) {
                    Text(m.t("easy")).tag("easy"); Text(m.t("detailed")).tag("detailed")
                }.pickerStyle(.segmented)
                Text(m.t("mode_hint")).font(.caption).foregroundStyle(.secondary)
                NumberRow(label: m.t("target_kwp"), field: a.target_kwp, pointer: arrayPath + "/target_kwp").id(arrayPath + "/target_kwp")
                NumberRow(label: m.t("tilt"), field: a.tilt, pointer: arrayPath + "/tilt").id(arrayPath + "/tilt")
                NumberRow(label: m.t("azimuth"), field: a.azimuth, pointer: arrayPath + "/azimuth").id(arrayPath + "/azimuth")
                TrackerEditor(pointer: arrayPath + "/tracking",
                              settings: tracking ?? [:],
                              facing: a.azimuth.value)
                    .id((m.url?.path ?? "") + arrayPath + "/tracking")
                Picker(m.t("mounting"), selection: Binding(get: { p.simulation.mounting.value }, set: { m.set("/simulation/mounting", $0) })) {
                    Text(m.t("open_rack")).tag("open_rack"); Text(m.t("roof_mounted")).tag("roof_mounted"); Text(m.t("integrated")).tag("integrated")
                }
                Picker(m.t("pv_model"), selection: Binding(get: { p.simulation.pv_model }, set: { m.set("/simulation/pv_model", $0) })) {
                    Text(m.t("single_diode")).tag("single_diode")
                    Text(m.t("two_diode")).tag("two_diode")
                }
                if p.mode == "detailed" {
                    IntRow(label: m.t("modules_per_string"), field: a.modules_per_string, pointer: arrayPath + "/modules_per_string").id(arrayPath + "/modules_per_string")
                    IntRow(label: m.t("strings"), field: a.strings, pointer: arrayPath + "/strings").id(arrayPath + "/strings")
                } else {
                    LabeledContent(m.t("strings")) { Text("\(a.strings.value) × \(a.modules_per_string.value)").monospacedDigit() }
                }
                Section(m.t("issues")) {
                    if m.issues.isEmpty { Text(m.t("no_issues")).foregroundStyle(.secondary) }
                    ForEach(m.issues) { i in
                        Label(i.message, systemImage: i.severity == "error" ? "xmark.octagon.fill" : "exclamationmark.triangle.fill")
                            .foregroundStyle(i.severity == "error" ? .red : .orange)
                    }
                }
            }
        }
    }
}

struct Kpi: View {
    let label: String; let value: String; let unit: String
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption).foregroundStyle(.secondary)
            HStack(alignment: .firstTextBaseline, spacing: 3) { Text(value).font(.title2.bold()).monospacedDigit(); Text(unit).font(.caption).foregroundStyle(.secondary) }
        }.padding(10).frame(maxWidth: .infinity, alignment: .leading).background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
    }
}

struct ResultsPage: View {
    @EnvironmentObject var m: AppModel
    var body: some View {
        if let r = m.results {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Kpi(label: m.t("system_size"), value: r.annual.dc_kwp.formatted(.number.precision(.fractionLength(2))), unit: "kWp")
                    Kpi(label: m.t("annual_energy"), value: r.annual.ac_kwh.formatted(.number.precision(.fractionLength(0))), unit: "kWh")
                    Kpi(label: m.t("specific_yield"), value: r.annual.specific_yield_kwh_kwp.formatted(.number.precision(.fractionLength(0))), unit: "kWh/kWp")
                    Kpi(label: m.t("performance_ratio"), value: (r.annual.performance_ratio * 100).formatted(.number.precision(.fractionLength(1))), unit: "%")
                }
                Text(m.t("monthly_energy")).font(.headline)
                Chart(r.monthly) { BarMark(x: .value("Month", $0.month), y: .value("kWh", $0.ac_kwh)) }
                    .chartXScale(domain: 0.5...12.5).frame(height: 220).accessibilityLabel(m.t("monthly_energy"))
                Text(m.t("irradiation")).font(.headline)
                Grid(alignment: .trailing, horizontalSpacing: 16, verticalSpacing: 4) {
                    ForEach(r.irradiance_kwh_m2) { s in
                        GridRow {
                            Text(m.t("irr_" + s.key)).gridColumnAlignment(.leading)
                            Text(s.value.formatted(.number.precision(.fractionLength(1))) + " kWh/m²").monospacedDigit()
                            Text(["ghi", "effective"].contains(s.key) ? "" : s.delta_pct.formatted(.number.precision(.fractionLength(2))) + " %").monospacedDigit().foregroundStyle(.secondary)
                        }
                    }
                }
                Text(m.t("losses")).font(.headline)
                Grid(alignment: .trailing, horizontalSpacing: 16, verticalSpacing: 4) {
                    ForEach(r.energy_kwh) { s in
                        GridRow {
                            Text(s.label).gridColumnAlignment(.leading).fontWeight(["nominal", "dc_out", "grid"].contains(s.key) ? .semibold : .regular)
                            Text(s.value.formatted(.number.precision(.fractionLength(0)))).monospacedDigit()
                            Text(["nominal", "dc_out", "grid"].contains(s.key) ? "" : s.delta_pct.formatted(.number.precision(.fractionLength(2))) + " %").monospacedDigit().foregroundStyle(.secondary)
                        }
                    }
                }
            }
        } else {
            Text(m.t("no_results")).foregroundStyle(.secondary)
        }
    }
}

struct ReportPage: View {
    @EnvironmentObject var m: AppModel
    var includePreview = false
    @State private var lang = String((Locale.preferredLanguages.first ?? "en").prefix(2))
    @State private var format = "pdf"
    @State private var reportType = "customer_extended"
    @State private var designer = ""
    @State private var contents: [JSONDict] = []
    @State private var excluded: Set<String> = []
    var body: some View {
        Form {
            Picker(m.t("report_type"), selection: $reportType) {
                Text(m.t("customer_short")).tag("customer_short")
                Text(m.t("customer_extended")).tag("customer_extended")
                Text(m.t("system_planner")).tag("system_planner")
            }
            Picker(m.t("language"), selection: $lang) { Text("English").tag("en"); Text("Español").tag("es"); Text("Deutsch").tag("de"); Text("Português (BR)").tag("pt") }
            Picker(m.t("format"), selection: $format) { Text("PDF").tag("pdf"); Text("DOCX").tag("docx") }.pickerStyle(.segmented)
            TextField(m.t("designer"), text: $designer)
            Button(m.t("export_report")) {
                m.exportReport(lang: lang, format: format, designer: designer, reportType: reportType, excludeSections: excluded.sorted())
            }.disabled(m.results == nil)
            if m.results != nil {
                Button(m.t("report_preview")) { contents = m.reportContents(lang: lang, reportType: reportType, designer: designer) }
                if !contents.isEmpty {
                    Section(m.t("report_sections")) {
                        ForEach(contents.indices, id: \.self) { index in
                            let section = contents[index]
                            let id = str(section, "id")
                            Toggle(str(section, "title"), isOn: Binding(get: { !excluded.contains(id) }, set: { include in
                                if include { excluded.remove(id) } else { excluded.insert(id) }
                            }))
                            if !excluded.contains(id) {
                                let blocks = dicts(section["blocks"])
                                ForEach(blocks.indices, id: \.self) { i in
                                    let block = blocks[i]
                                    if str(block, "kind") == "table" {
                                        ScrollView(.horizontal) {
                                            SimpleTable(header: block["header"] as? [String] ?? [],
                                                        rows: Array((block["rows"] as? [[String]] ?? []).prefix(12))).font(.caption)
                                        }
                                    } else {
                                        let text = str(block, "kind") == "text" ? str(block, "text") : str(block, "caption")
                                        if !text.isEmpty { Text(text).foregroundStyle(.secondary) }
                                        if let bytes = Data(base64Encoded: str(block, "png_base64")), let picture = NSImage(data: bytes) {
                                            Image(nsImage: picture).resizable().scaledToFit().frame(maxWidth: 700, maxHeight: 320)
                                                .accessibilityLabel(text.isEmpty ? str(section, "title") : text)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        .onAppear {
            if includePreview && m.results != nil {
                contents = m.reportContents(lang: lang, reportType: reportType, designer: designer)
            }
        }
        .onChange(of: reportType) { contents.removeAll(); excluded.removeAll() }
        .onChange(of: lang) { contents.removeAll() }
        .onChange(of: m.url) { contents.removeAll(); excluded.removeAll() }
        .onChange(of: m.results == nil) { _, empty in if empty { contents.removeAll() } }
    }
}

/// Parses "x,y; x,y; …" into points.
func parsePoints(_ text: String) -> [[Double]] {
    text.split(separator: ";").compactMap { pair -> [Double]? in
        let v = pair.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
        return v.count == 2 ? v : nil
    }
}

/// Heights (5th value, metres; default 1) for the obstacles accepted by `parseObstacles`.
func parseObstacleHeights(_ text: String) -> [Double] {
    text.split(separator: ";").compactMap { item -> Double? in
        let v = item.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
        guard v.count == 4 || v.count == 5, v[2] > 0, v[3] > 0 else { return nil }
        return v.count == 5 ? v[4] : 1.0
    }
}

/// Parses "x,y,w,d[,h]; …" into rectangular obstacle polygons.
func parseObstacles(_ text: String) -> [[[Double]]] {
    text.split(separator: ";").compactMap { item -> [[Double]]? in
        let v = item.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
        guard v.count == 4 || v.count == 5, v[2] > 0, v[3] > 0 else { return nil }
        return [[v[0], v[1]], [v[0] + v[2], v[1]], [v[0] + v[2], v[1] + v[3]], [v[0], v[1] + v[3]]]
    }
}

struct RoofPage: View {
    @EnvironmentObject var m: AppModel
    @State private var showPhoto = false
    @State private var selectedArray = 0
    private var arrays: [ProjectDoc.ArrayDoc] { m.project?.arrays ?? [] }
    private var roof: ProjectDoc.RoofDoc? { arrays.indices.contains(selectedArray) ? arrays[selectedArray].roof : nil }
    @State private var shape = "rect"
    @State private var eave = 10.0
    @State private var ridge = 6.0
    @State private var slope = 5.0
    @State private var custom = ""
    @State private var obstacles = ""
    @State private var setback = 0.4
    @State private var clearance = 0.3

    private func outline() -> [[Double]]? {
        switch shape {
        case "rect": return [[0, 0], [eave, 0], [eave, slope], [0, slope]]
        case "trapezoid":
            let inset = max(0, (eave - ridge) / 2)
            return [[0, 0], [eave, 0], [eave - inset, slope], [inset, slope]]
        case "custom":
            let p = parsePoints(custom)
            return p.count >= 3 ? p : nil
        default: return nil
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Form {
                Picker(m.t("photo_array"), selection: $selectedArray) {
                    ForEach(arrays.indices, id: \.self) { i in Text(arrays[i].name.value).tag(i) }
                }
                Button(m.t("add_pv_array")) { if let index = m.addArray() { selectedArray = index } }
                Button(m.t("photo_plan")) { showPhoto = true }.disabled(m.url == nil)
                Picker(m.t("roof_shape"), selection: $shape) {
                    Text(m.t("roof_rect")).tag("rect")
                    Text(m.t("roof_trapezoid")).tag("trapezoid")
                    Text(m.t("roof_custom")).tag("custom")
                    Text(m.t("roof_none")).tag("none")
                }
                if shape == "rect" || shape == "trapezoid" {
                    TextField(m.t("eave_length"), value: $eave, format: .number)
                    if shape == "trapezoid" { TextField(m.t("ridge_length"), value: $ridge, format: .number) }
                    TextField(m.t("slope_length"), value: $slope, format: .number)
                }
                if shape == "custom" {
                    TextField(m.t("custom_points"), text: $custom)
                    Text(m.t("click_to_add")).font(.caption).foregroundStyle(.secondary)
                    DraftCanvas(text: $custom).frame(height: 280)
                    HStack {
                        Button(m.t("undo_point")) {
                            var pts = parsePoints(custom)
                            if !pts.isEmpty { pts.removeLast() }
                            custom = formatPoints(pts)
                        }
                        Button(m.t("clear_points")) { custom = "" }
                    }
                }
                if shape != "none" {
                    TextField(m.t("obstacles"), text: $obstacles)
                    Text(m.t("roof_obstacle_formats")).font(.caption).foregroundStyle(.secondary)
                    TextField(m.t("setback"), value: $setback, format: .number)
                    TextField(m.t("clearance"), value: $clearance, format: .number)
                }
                Text(m.t("roof_hint")).font(.caption).foregroundStyle(.secondary)
                Button(m.t("apply_roof")) {
                    let ring = outline()
                    guard shape == "none" || ring != nil else { m.error = m.t("custom_points"); return }
                    let text = obstacles.trimmingCharacters(in: .whitespacesAndNewlines)
                    if text.hasPrefix("{") {
                        guard let o = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? JSONDict,
                              let polygons = o["polygons"] as? [[[Double]]], let heights = o["heights"] as? [Double]
                        else { m.error = m.t("roof_obstacle_formats"); return }
                        m.setRoof(array: selectedArray, outline: ring, obstacles: polygons, heights: heights, setback: setback, clearance: clearance)
                    } else {
                        m.setRoof(array: selectedArray, outline: ring, obstacles: parseObstacles(text), heights: parseObstacleHeights(text), setback: setback, clearance: clearance)
                    }
                }.keyboardShortcut(.return, modifiers: [.command])
            }
            if let roof {
                RoofPreview(roof: roof).frame(height: 300)
                Text("\(roof.modules.count) \(m.t("modules_fit"))").font(.headline)
            }
        }
        .onAppear(perform: loadFromProject)
        .onChange(of: selectedArray) { _, _ in loadFromProject() }
        .sheet(isPresented: $showPhoto, onDismiss: loadFromProject) { PhotoPlanSheet(initialArray: selectedArray).environmentObject(m) }
    }

    private func loadFromProject() {
        eave = 10; ridge = 6; slope = 5
        guard let roof else {
            shape = "rect"; custom = ""; obstacles = ""; setback = 0.4; clearance = 0.3
            return
        }
        shape = "custom"
        custom = roof.face.outline.map { "\($0[0]),\($0[1])" }.joined(separator: "; ")
        if roof.face.obstacles.isEmpty { obstacles = "" } else {
            let payload: JSONDict = ["polygons": roof.face.obstacles, "heights": roof.face.obstacle_heights]
            if let bytes = try? JSONSerialization.data(withJSONObject: payload) {
                obstacles = String(decoding: bytes, as: UTF8.self)
            }
        }
        setback = roof.params.setback
        clearance = roof.params.obstacle_clearance
    }
}

/// Draws the roof face (eave at the bottom), obstacles and placed modules.
struct RoofPreview: View {
    let roof: ProjectDoc.RoofDoc
    var body: some View {
        Canvas { ctx, size in
            let pts = roof.face.outline
            guard let minX = pts.map({ $0[0] }).min(), let maxX = pts.map({ $0[0] }).max(),
                  let minY = pts.map({ $0[1] }).min(), let maxY = pts.map({ $0[1] }).max(), maxX > minX, maxY > minY else { return }
            let pad = 12.0
            let scale = min((size.width - 2 * pad) / (maxX - minX), (size.height - 2 * pad) / (maxY - minY))
            func pt(_ x: Double, _ y: Double) -> CGPoint { CGPoint(x: pad + (x - minX) * scale, y: pad + (maxY - y) * scale) }
            func poly(_ p: [[Double]]) -> Path {
                var path = Path()
                for (i, v) in p.enumerated() { if i == 0 { path.move(to: pt(v[0], v[1])) } else { path.addLine(to: pt(v[0], v[1])) } }
                path.closeSubpath()
                return path
            }
            ctx.fill(poly(pts), with: .color(.gray.opacity(0.12)))
            ctx.stroke(poly(pts), with: .color(.primary), lineWidth: 1.5)
            for r in roof.modules {
                let rect = CGRect(x: pad + (r.x0 - minX) * scale, y: pad + (maxY - r.y1) * scale, width: (r.x1 - r.x0) * scale, height: (r.y1 - r.y0) * scale)
                ctx.fill(Path(rect), with: .color(Color(red: 0.06, green: 0.36, blue: 0.39)))
                ctx.stroke(Path(rect), with: .color(.white), lineWidth: 0.6)
            }
            for ob in roof.face.obstacles { ctx.fill(poly(ob), with: .color(.gray)) }
        }
        .accessibilityLabel("Roof layout")
    }
}

func formatPoints(_ pts: [[Double]]) -> String {
    pts.map { String(format: "%.1f,%.1f", $0[0], $0[1]) }.joined(separator: "; ")
}

/// Click-to-add outline editor on a 1 m grid (origin bottom-left, y up the slope).
struct DraftCanvas: View {
    @Binding var text: String
    var body: some View {
        let pts = parsePoints(text)
        let w = max(12.0, (pts.map { $0[0] }.max() ?? 0) + 2)
        let h = max(8.0, (pts.map { $0[1] }.max() ?? 0) + 2)
        return GeometryReader { geo in
            let scale = min(geo.size.width / w, geo.size.height / h)
            Canvas { ctx, _ in
                func pt(_ x: Double, _ y: Double) -> CGPoint { CGPoint(x: x * scale, y: (h - y) * scale) }
                var grid = Path()
                for gx in 0...Int(w) { grid.move(to: pt(Double(gx), 0)); grid.addLine(to: pt(Double(gx), h)) }
                for gy in 0...Int(h) { grid.move(to: pt(0, Double(gy))); grid.addLine(to: pt(w, Double(gy))) }
                ctx.stroke(grid, with: .color(.gray.opacity(0.25)), lineWidth: 0.5)
                if pts.count >= 2 {
                    var poly = Path()
                    for (i, p) in pts.enumerated() { if i == 0 { poly.move(to: pt(p[0], p[1])) } else { poly.addLine(to: pt(p[0], p[1])) } }
                    if pts.count >= 3 { poly.closeSubpath(); ctx.fill(poly, with: .color(.gray.opacity(0.15))) }
                    ctx.stroke(poly, with: .color(.primary), lineWidth: 1.5)
                }
                for p in pts {
                    let c = pt(p[0], p[1])
                    ctx.fill(Path(ellipseIn: CGRect(x: c.x - 4, y: c.y - 4, width: 8, height: 8)), with: .color(.accentColor))
                }
            }
            .contentShape(Rectangle())
            .onTapGesture(coordinateSpace: .local) { loc in
                let x = (loc.x / scale * 10).rounded() / 10
                let y = ((h * scale - loc.y) / scale * 10).rounded() / 10
                guard x >= 0, y >= 0, x <= w, y <= h else { return }
                text = formatPoints(parsePoints(text) + [[x, y]])
            }
        }
        .accessibilityLabel("Roof outline editor")
    }
}
