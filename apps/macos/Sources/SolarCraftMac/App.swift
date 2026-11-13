// SPDX-License-Identifier: GPL-3.0-or-later
import Charts
import SwiftUI

@main
enum Entry {
    static func main() {
        if CommandLine.arguments.contains("--selftest") { exit(SelfTest.run()) }
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
    case project, site_weather, design, roof, results, report
    var id: String { rawValue }
    var icon: String {
        switch self {
        case .project: return "doc.text"
        case .site_weather: return "sun.max"
        case .design: return "square.grid.3x3"
        case .roof: return "house"
        case .results: return "chart.bar"
        case .report: return "doc.richtext"
        }
    }
}

struct RootView: View {
    @EnvironmentObject var m: AppModel
    @State private var page: Page? = .project
    var body: some View {
        NavigationSplitView {
            List(Page.allCases, selection: $page) { p in Label(m.t(p.rawValue), systemImage: p.icon).tag(p) }
                .disabled(m.project == nil)
                .navigationSplitViewColumnWidth(190)
        } detail: {
            VStack(spacing: 0) {
                if let e = m.error { Text(e).foregroundStyle(.white).padding(8).frame(maxWidth: .infinity).background(.red.opacity(0.85)) }
                if let n = m.notice { Text(n).padding(6).frame(maxWidth: .infinity).background(.green.opacity(0.15)) }
                if m.project == nil {
                    VStack(spacing: 12) {
                        Text(m.t("no_project")).font(.title3)
                        HStack { Button(m.t("new_project")) { m.newProject() }; Button(m.t("open_project")) { m.openProject() } }
                    }.frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        PageView(page: page ?? .project).padding(20).frame(maxWidth: 760, alignment: .leading)
                    }
                }
            }
            .toolbar {
                if m.busy { ProgressView(value: m.progress).frame(width: 120); Button(m.t("cancel")) { m.cancel() } }
                Button { m.simulate() } label: { Label(m.t("simulate"), systemImage: "play.fill") }.disabled(m.project == nil || m.busy)
            }
        }
    }
}

struct PageView: View {
    let page: Page
    @ViewBuilder var body: some View {
        switch page {
        case .project: ProjectPage()
        case .site_weather: SitePage()
        case .design: DesignPage()
        case .roof: RoofPage()
        case .results: ResultsPage()
        case .report: ReportPage()
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
    var body: some View {
        if let p = m.project, let a = p.arrays.first {
            Form {
                Picker(m.t("mode"), selection: Binding(get: { p.mode }, set: { m.setMode($0) })) {
                    Text(m.t("easy")).tag("easy"); Text(m.t("detailed")).tag("detailed")
                }.pickerStyle(.segmented)
                Text(m.t("mode_hint")).font(.caption).foregroundStyle(.secondary)
                NumberRow(label: m.t("target_kwp"), field: a.target_kwp, pointer: "/arrays/0/target_kwp")
                NumberRow(label: m.t("tilt"), field: a.tilt, pointer: "/arrays/0/tilt")
                NumberRow(label: m.t("azimuth"), field: a.azimuth, pointer: "/arrays/0/azimuth")
                Picker(m.t("mounting"), selection: Binding(get: { p.simulation.mounting.value }, set: { m.set("/simulation/mounting", $0) })) {
                    Text(m.t("open_rack")).tag("open_rack"); Text(m.t("roof_mounted")).tag("roof_mounted"); Text(m.t("integrated")).tag("integrated")
                }
                if p.mode == "detailed" {
                    IntRow(label: m.t("modules_per_string"), field: a.modules_per_string, pointer: "/arrays/0/modules_per_string")
                    IntRow(label: m.t("strings"), field: a.strings, pointer: "/arrays/0/strings")
                } else {
                    LabeledContent(m.t("strings")) { Text("\(a.strings.value) × \(a.modules_per_string.value)").monospacedDigit() }
                }
                LabeledContent(m.t("module")) { Text(p.library.modules.first { $0.id == a.module_id.value }.map { "\($0.manufacturer) \($0.model)" } ?? a.module_id.value) }
                LabeledContent(m.t("inverter")) { Text(p.library.inverters.first { $0.id == a.inverter_id.value }.map { "\(a.inverter_count.value) × \($0.manufacturer) \($0.model)" } ?? "—") }
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
    @State private var lang = String((Locale.preferredLanguages.first ?? "en").prefix(2))
    @State private var format = "pdf"
    @State private var designer = ""
    var body: some View {
        Form {
            Picker(m.t("language"), selection: $lang) { Text("English").tag("en"); Text("Español").tag("es"); Text("Deutsch").tag("de"); Text("Português (BR)").tag("pt") }
            Picker(m.t("format"), selection: $format) { Text("PDF").tag("pdf"); Text("DOCX").tag("docx") }.pickerStyle(.segmented)
            TextField(m.t("designer"), text: $designer)
            Button(m.t("export_report")) { m.exportReport(lang: lang, format: format, designer: designer) }.disabled(m.results == nil)
            if m.results == nil { Text(m.t("no_results")).foregroundStyle(.secondary) }
        }
    }
}

/// Parses "x,y; x,y; …" into points.
func parsePoints(_ text: String) -> [[Double]] {
    text.split(separator: ";").compactMap { pair -> [Double]? in
        let v = pair.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
        return v.count == 2 ? v : nil
    }
}

/// Parses "x,y,w,h; …" into rectangular obstacle polygons.
func parseObstacles(_ text: String) -> [[[Double]]] {
    text.split(separator: ";").compactMap { item -> [[Double]]? in
        let v = item.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
        guard v.count == 4, v[2] > 0, v[3] > 0 else { return nil }
        return [[v[0], v[1]], [v[0] + v[2], v[1]], [v[0] + v[2], v[1] + v[3]], [v[0], v[1] + v[3]]]
    }
}

struct RoofPage: View {
    @EnvironmentObject var m: AppModel
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
                if shape == "custom" { TextField(m.t("custom_points"), text: $custom) }
                if shape != "none" {
                    TextField(m.t("obstacles"), text: $obstacles)
                    TextField(m.t("setback"), value: $setback, format: .number)
                    TextField(m.t("clearance"), value: $clearance, format: .number)
                }
                Text(m.t("roof_hint")).font(.caption).foregroundStyle(.secondary)
                Button(m.t("apply_roof")) {
                    m.setRoof(outline: shape == "none" ? nil : outline(), obstacles: parseObstacles(obstacles), setback: setback, clearance: clearance)
                }.keyboardShortcut(.return, modifiers: [.command])
            }
            if let roof = m.project?.arrays.first?.roof {
                RoofPreview(roof: roof).frame(height: 300)
                Text("\(roof.modules.count) \(m.t("modules_fit"))").font(.headline)
            }
        }
        .onAppear(perform: loadFromProject)
    }

    private func loadFromProject() {
        guard let roof = m.project?.arrays.first?.roof else { return }
        shape = "custom"
        custom = roof.face.outline.map { "\($0[0]),\($0[1])" }.joined(separator: "; ")
        obstacles = roof.face.obstacles.compactMap { pts -> String? in
            let xs = pts.map { $0[0] }, ys = pts.map { $0[1] }
            guard let x0 = xs.min(), let x1 = xs.max(), let y0 = ys.min(), let y1 = ys.max() else { return nil }
            return "\(x0),\(y0),\(x1 - x0),\(y1 - y0)"
        }.joined(separator: "; ")
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
