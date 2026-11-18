// SPDX-License-Identifier: GPL-3.0-or-later
// Plant workflow pages: grid, consumption and storage, electrical design and
// equipment, simulation inspection, economics and variants, report contents.
// Every edit goes through the core's validated JSON API (set_section,
// set_stringing, add_equipment, …), the same contract the CLI and MCP use.
import Charts
import SwiftUI

// MARK: - helpers

func num(_ d: JSONDict, _ k: String, _ fallback: Double = 0) -> Double { (d[k] as? NSNumber)?.doubleValue ?? fallback }
func int(_ d: JSONDict, _ k: String, _ fallback: Int = 0) -> Int { (d[k] as? NSNumber)?.intValue ?? fallback }
func str(_ d: JSONDict, _ k: String) -> String { d[k] as? String ?? "" }
func fmt(_ v: Double, _ digits: Int = 0) -> String { v.formatted(.number.precision(.fractionLength(digits))) }
func dicts(_ v: Any?) -> [JSONDict] { v as? [JSONDict] ?? [] }

struct NumField: View {
    let label: String
    @Binding var value: Double
    var body: some View {
        LabeledContent(label) { TextField("", value: $value, format: .number).frame(width: 120).multilineTextAlignment(.trailing) }
    }
}

struct IssueList: View {
    let issues: [JSONDict]
    var body: some View {
        ForEach(Array(issues.enumerated()), id: \.offset) { _, i in
            let sev = str(i, "severity")
            Label(str(i, "message"), systemImage: sev == "error" ? "xmark.octagon.fill" : "exclamationmark.triangle")
                .foregroundStyle(sev == "error" ? Color.red : Color.orange).font(.callout)
        }
    }
}

/// A compact table from rows of strings.
struct SimpleTable: View {
    let header: [String]
    let rows: [[String]]
    var body: some View {
        Grid(alignment: .trailing, horizontalSpacing: 14, verticalSpacing: 3) {
            if !header.isEmpty {
                GridRow { ForEach(Array(header.enumerated()), id: \.offset) { i, h in Text(h).font(.caption.weight(.semibold)).foregroundStyle(.secondary).gridColumnAlignment(i == 0 ? .leading : .trailing) } }
                Divider()
            }
            ForEach(Array(rows.enumerated()), id: \.offset) { _, r in
                GridRow { ForEach(Array(r.enumerated()), id: \.offset) { i, c in Text(c).monospacedDigit().gridColumnAlignment(i == 0 ? .leading : .trailing) } }
            }
        }
    }
}

// MARK: - site, climate and grid

struct GridSection: View {
    @EnvironmentObject var m: AppModel
    @State private var feedIn = "surplus"
    @State private var limit = ""
    @State private var phases = 1
    @State private var voltage = 230.0
    var body: some View {
        GroupBox(m.t(m.isIslanded ? "island_bus" : "grid")) {
            Form {
                if m.systemType == "custom" {
                    Picker(m.t("feed_in"), selection: $feedIn) { Text(m.t("surplus")).tag("surplus"); Text(m.t("full_feed_in")).tag("full") }
                }
                if !m.isIslanded {
                    LabeledContent(m.t("export_limit")) { TextField("", text: $limit).frame(width: 120) }
                }
                Picker(m.t("phases"), selection: $phases) { Text("1").tag(1); Text("3").tag(3) }.pickerStyle(.segmented).frame(width: 240)
                NumField(label: m.t("voltage"), value: $voltage)
                Button(m.t("apply")) {
                    let lim: Any = Double(limit.replacingOccurrences(of: ",", with: ".")).map { $0 as Any } ?? NSNull()
                    let value: JSONDict = ["feed_in": feedIn, "export_limit_kw": lim, "phases": phases, "voltage_v": voltage]
                    m.setSection("grid", value)
                }
            }
        }
        .onAppear(perform: load)
    }
    private func load() {
        let g = m.section("grid")
        feedIn = g["feed_in"] as? String ?? "surplus"
        limit = (g["export_limit_kw"] as? NSNumber).map { fmt($0.doubleValue, 1) } ?? ""
        phases = int(g, "phases", 1)
        voltage = num(g, "voltage_v", 230)
    }
}

struct SiteClimatePage: View {
    var body: some View { VStack(alignment: .leading, spacing: 18) { SystemWorkflowSection(); SitePage(); GridSection() } }
}

// MARK: - consumption and storage

struct OutageEdit: Identifiable { let id = UUID(); var day = 172.0; var hour = 18.0; var hours = 4.0 }

struct StorageLoadsPage: View {
    var consumptionOnly: Bool
    @EnvironmentObject var m: AppModel
    // Loads
    @State private var profile = "residential"
    @State private var annual = 4000.0
    @State private var critical = 0.3
    // Storage
    @State private var enabled = false
    @State private var batteryId = ""
    @State private var units = 1
    @State private var coupling = "dc_hybrid"
    @State private var unitId = ""
    @State private var batInvId = ""
    @State private var model = "simple"
    @State private var socMin = 10.0
    @State private var socMax = 100.0
    @State private var initSoc = 50.0
    @State private var backup = 20.0
    @State private var services = 0.0
    @State private var gsKind = "none"
    @State private var gsLimit = 2.0
    @State private var gsStart = 18.0
    @State private var gsEnd = 21.0
    @State private var gsPower = 3.0
    @State private var gridCharge = false
    @State private var outages: [OutageEdit] = []
    @State private var generatorID = ""
    @State private var generatorEditor = false
    private var generators: [JSONDict] { dicts(lib["generators"]) }
    private var requiresStorage: Bool { ["grid_battery", "off_grid", "off_grid_generator"].contains(m.systemType) }

    private var lib: JSONDict { m.section("library") }
    private var batteries: [JSONDict] { dicts(lib["batteries"]) }
    private var batInverters: [JSONDict] { dicts(lib["battery_inverters"]) }
    private var hybridUnits: [JSONDict] { dicts(m.stringingDoc?["inverters"]).filter { ($0["has_battery_port"] as? Bool) == true }.compactMap { $0["unit"] as? JSONDict } }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if consumptionOnly { GroupBox(m.t("loads")) {
                Form {
                    Picker(m.t("load_profile"), selection: $profile) {
                        ForEach(["none", "residential", "office", "retail", "flat", "series"], id: \.self) { Text(m.t("profile_" + $0)).tag($0) }
                    }
                    if profile != "series" && profile != "none" { NumField(label: m.t("annual_kwh"), value: $annual) }
                    if profile == "series" { Text((m.section("loads")["profile"] as? JSONDict).map { str($0, "name") } ?? "").foregroundStyle(.secondary) }
                    if !m.isIslanded { NumField(label: m.t("critical_share"), value: $critical) }
                    HStack {
                        Button(m.t("apply")) { applyLoads() }
                        Button(m.t("import_load")) { m.importLoad() }
                    }
                }
            }
            } else { GroupBox(m.t("use_battery")) {
                VStack(alignment: .leading, spacing: 10) {
                    Toggle(m.t("use_battery"), isOn: $enabled).disabled(requiresStorage)
                    if enabled {
                        if batteries.isEmpty { Button(m.t("add_generic_storage")) { m.addGenericStorage() } }
                        Form {
                            Picker(m.t("battery"), selection: $batteryId) { ForEach(batteries.indices, id: \.self) { i in Text("\(str(batteries[i], "manufacturer")) \(str(batteries[i], "model"))").tag(str(batteries[i], "id")) } }
                            Stepper("\(m.t("units")): \(units)", value: $units, in: 1...20)
                            Picker(m.t("coupling"), selection: $coupling) { Text(m.t("dc_hybrid")).tag("dc_hybrid"); Text(m.t("ac_coupled")).tag("ac_coupled"); Text(m.t("integrated")).tag("integrated") }
                            if coupling == "dc_hybrid" {
                                Picker(m.t("inverter"), selection: $unitId) { ForEach(hybridUnits.indices, id: \.self) { i in Text(str(hybridUnits[i], "name")).tag(str(hybridUnits[i], "id")) } }
                            } else if coupling == "ac_coupled" {
                                Picker(m.t("inverter"), selection: $batInvId) { ForEach(batInverters.indices, id: \.self) { i in Text("\(str(batInverters[i], "manufacturer")) \(str(batInverters[i], "model"))").tag(str(batInverters[i], "id")) } }
                            }
                            Picker(m.t("battery_model"), selection: $model) { Text(m.t("model_simple")).tag("simple"); Text(m.t("model_detailed")).tag("detailed") }
                            HStack { NumField(label: m.t("soc_window"), value: $socMin); Text("–"); TextField("", value: $socMax, format: .number).frame(width: 60) }
                            NumField(label: m.t("initial_soc"), value: $initSoc)
                        }
                        if !m.isIslanded {
                            Text(m.t("allocation")).font(.headline)
                            AllocationBar(backup: backup, services: services, labels: [m.t("backup_reserve"), m.t("self_consumption"), m.t("grid_services")])
                            Form {
                                NumField(label: m.t("backup_reserve"), value: $backup)
                                NumField(label: m.t("grid_services"), value: $services)
                                LabeledContent(m.t("self_consumption")) { Text("\(fmt(100 - backup - services)) %").monospacedDigit() }
                                Picker(m.t("grid_service"), selection: $gsKind) { Text(m.t("gs_none")).tag("none"); Text(m.t("gs_peak")).tag("peak_shaving"); Text(m.t("gs_window")).tag("export_window") }
                                if gsKind == "peak_shaving" { NumField(label: "kW", value: $gsLimit) }
                                if gsKind == "export_window" { HStack { NumField(label: m.t("hour"), value: $gsStart); TextField("", value: $gsEnd, format: .number).frame(width: 50); NumField(label: "kW", value: $gsPower) } }
                                Toggle(m.t("grid_charge_backup"), isOn: $gridCharge)
                            }
                            Text(m.t("allocation_note")).font(.caption).foregroundStyle(.secondary)
                            Text(m.t("outages")).font(.headline)
                            ForEach($outages) { $o in
                                HStack {
                                    NumField(label: m.t("day"), value: $o.day)
                                    NumField(label: m.t("hour"), value: $o.hour)
                                    NumField(label: m.t("hours_n"), value: $o.hours)
                                    Button(m.t("remove")) { outages.removeAll { $0.id == o.id } }
                                }
                            }
                            Button(m.t("add_outage")) { outages.append(OutageEdit()) }
                        }
                        if m.systemType != "off_grid" {
                            Picker(m.t("generator"), selection: $generatorID) {
                                Text(m.t("gs_none")).tag("")
                                ForEach(generators.indices, id: \.self) { i in
                                    Text("\(str(generators[i], "manufacturer")) \(str(generators[i], "model"))").tag(str(generators[i], "id"))
                                }
                            }
                            Button(m.t("new_generator")) { generatorEditor = true }
                        }
                    }
                    Button(m.t("apply")) { applyStorage() }.keyboardShortcut(.defaultAction)
                    IssueList(issues: m.issues.filter { $0.pointer.hasPrefix("/storage") || $0.pointer.hasPrefix("/loads") }.map { ["severity": $0.severity, "message": $0.message] })
                }
            }
            }
        }
        .onAppear(perform: load)
        .sheet(isPresented: $generatorEditor) { GeneratorEditor().environmentObject(m) }
    }

    private func applyLoads() {
        let p: Any
        switch profile {
        case "none": p = ["kind": "none"]
        case "series": p = m.section("loads")["profile"] ?? (["kind": "none"] as JSONDict)
        default: p = ["kind": "synthetic", "profile": profile]
        }
        let value: JSONDict = ["profile": p, "annual_kwh": annual, "critical_fraction": critical]
        m.setSection("loads", value)
    }

    private func applyStorage() {
        guard enabled else { m.setSection("storage", NSNull()); return }
        let c: JSONDict
        switch coupling {
        case "dc_hybrid": c = ["kind": coupling, "unit_id": unitId]
        case "ac_coupled": c = ["kind": coupling, "battery_inverter_id": batInvId]
        default: c = ["kind": "integrated"]
        }
        let gs: JSONDict
        switch gsKind {
        case "peak_shaving": gs = ["kind": "peak_shaving", "import_limit_kw": gsLimit]
        case "export_window": gs = ["kind": "export_window", "start_hour": Int(gsStart), "end_hour": Int(gsEnd), "power_kw": gsPower]
        default: gs = ["kind": "none"]
        }
        let value: JSONDict = [
            "battery_id": batteryId, "units": units, "coupling": c, "model": model, "soc_min_pct": socMin, "soc_max_pct": socMax, "initial_soc_pct": initSoc,
            "backup_reserve_pct": backup, "grid_services_pct": services, "charge_priority": ["backup", "self_consumption", "grid_services"],
            "grid_charge_backup": gridCharge, "grid_service": gs,
            "generator_id": generatorID.isEmpty ? NSNull() as Any : generatorID,
            "outages": outages.map { ["start_hour": max(0, Int($0.day) - 1) * 24 + Int($0.hour), "hours": Int($0.hours)] },
        ]
        m.setSection("storage", value)
    }

    private func load() {
        let l = m.section("loads")
        let p = l["profile"] as? JSONDict ?? [:]
        profile = str(p, "kind") == "synthetic" ? str(p, "profile") : (str(p, "kind") == "series" ? "series" : (str(p, "kind").isEmpty ? "residential" : "none"))
        annual = num(l, "annual_kwh", 4000)
        critical = num(l, "critical_fraction", 0.3)
        let s = m.section("storage")
        enabled = requiresStorage || !s.isEmpty
        generatorID = str(s, "generator_id")
        batteryId = str(s, "battery_id").isEmpty ? (batteries.first.map { str($0, "id") } ?? "") : str(s, "battery_id")
        units = int(s, "units", 1)
        let c = s["coupling"] as? JSONDict ?? [:]
        coupling = str(c, "kind").isEmpty ? "dc_hybrid" : str(c, "kind")
        unitId = str(c, "unit_id").isEmpty ? (hybridUnits.first.map { str($0, "id") } ?? "") : str(c, "unit_id")
        batInvId = str(c, "battery_inverter_id").isEmpty ? (batInverters.first.map { str($0, "id") } ?? "") : str(c, "battery_inverter_id")
        model = str(s, "model").isEmpty ? "simple" : str(s, "model")
        socMin = num(s, "soc_min_pct", 10); socMax = num(s, "soc_max_pct", 100); initSoc = num(s, "initial_soc_pct", 50)
        backup = num(s, "backup_reserve_pct", 20); services = num(s, "grid_services_pct", 0)
        gridCharge = (s["grid_charge_backup"] as? Bool) ?? false
        let gs = s["grid_service"] as? JSONDict ?? [:]
        gsKind = str(gs, "kind").isEmpty ? "none" : str(gs, "kind")
        gsLimit = num(gs, "import_limit_kw", 2); gsStart = num(gs, "start_hour", 18); gsEnd = num(gs, "end_hour", 21); gsPower = num(gs, "power_kw", 3)
        outages = dicts(s["outages"]).map { o in OutageEdit(day: Double(int(o, "start_hour") / 24 + 1), hour: Double(int(o, "start_hour") % 24), hours: num(o, "hours", 1)) }
    }
}

/// The usable energy split into its three uses, drawn to scale.
struct AllocationBar: View {
    let backup: Double
    let services: Double
    let labels: [String]
    var body: some View {
        let b = max(0, min(100, backup)), g = max(0, min(100 - b, services)), s = max(0, 100 - b - g)
        GeometryReader { geo in
            HStack(spacing: 0) {
                Rectangle().fill(Color.orange.opacity(0.8)).frame(width: geo.size.width * b / 100)
                Rectangle().fill(Color.teal.opacity(0.8)).frame(width: geo.size.width * s / 100)
                Rectangle().fill(Color.purple.opacity(0.7)).frame(width: geo.size.width * g / 100)
            }
        }
        .frame(height: 18).clipShape(RoundedRectangle(cornerRadius: 4))
        HStack(spacing: 14) {
            Label("\(labels[0]) \(fmt(b)) %", systemImage: "square.fill").foregroundStyle(.orange)
            Label("\(labels[1]) \(fmt(s)) %", systemImage: "square.fill").foregroundStyle(.teal)
            Label("\(labels[2]) \(fmt(g)) %", systemImage: "square.fill").foregroundStyle(.purple)
        }.font(.caption)
    }
}

// MARK: - electrical design and equipment

struct GroupEdit: Identifiable { let id = UUID(); var arrayId: String; var modules: Int; var strings: Int }
struct UnitEdit: Identifiable { let id: String; var name: String; var inverterId: String; var mppts: [[GroupEdit]] }

struct ElectricalPage: View {
    @EnvironmentObject var m: AppModel
    @State private var units: [UnitEdit] = []
    @State private var manual = false
    @State private var editor: String?
    @State private var query = ""

    private var inverters: [JSONDict] { dicts(m.section("library")["inverters"]) }
    private var arrays: [JSONDict] { dicts(m.raw["arrays"]) }
    private func mpptCount(_ invId: String) -> Int { inverters.first { str($0, "id") == invId }.map { int($0, "mppt_count", 1) } ?? 1 }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            GroupBox(m.t("stringing")) {
                VStack(alignment: .leading, spacing: 8) {
                    Toggle(m.t("manual_stringing"), isOn: $manual)
                    ForEach($units) { $u in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                TextField("", text: $u.name).frame(width: 220).disabled(!manual)
                                Picker("", selection: $u.inverterId) { ForEach(inverters.indices, id: \.self) { i in Text("\(str(inverters[i], "manufacturer")) \(str(inverters[i], "model"))").tag(str(inverters[i], "id")) } }
                                    .frame(width: 260).disabled(!manual)
                            }
                            ForEach(0..<max(u.mppts.count, mpptCount(u.inverterId)), id: \.self) { mi in
                                HStack(alignment: .top) {
                                    Text("MPPT \(mi + 1)").frame(width: 60, alignment: .leading).foregroundStyle(.secondary)
                                    VStack(alignment: .leading) {
                                        if mi < u.mppts.count {
                                            ForEach($u.mppts[mi]) { $g in
                                                HStack {
                                                    Picker("", selection: $g.arrayId) { ForEach(arrays.indices, id: \.self) { i in Text((arrays[i]["name"] as? JSONDict).map { str($0, "value") } ?? str(arrays[i], "id")).tag(str(arrays[i], "id")) } }.frame(width: 140)
                                                    Stepper("\(m.t("modules_per_string_short")): \(g.modules)", value: $g.modules, in: 1...60)
                                                    Stepper("\(m.t("strings_short")): \(g.strings)", value: $g.strings, in: 1...40)
                                                    Button { u.mppts[mi].removeAll { $0.id == g.id } } label: { Image(systemName: "minus.circle") }.buttonStyle(.borderless)
                                                }.disabled(!manual)
                                            }
                                        }
                                        if manual {
                                            Button(m.t("add_string")) {
                                                while u.mppts.count <= mi { u.mppts.append([]) }
                                                u.mppts[mi].append(GroupEdit(arrayId: arrays.first.map { str($0, "id") } ?? "array-1", modules: 8, strings: 1))
                                            }.buttonStyle(.link)
                                        }
                                    }
                                }
                            }
                            Divider()
                        }
                    }
                    HStack {
                        Button(m.t("apply")) { apply() }.keyboardShortcut(.defaultAction)
                        if manual { Button("+ \(m.t("inverter"))") { units.append(UnitEdit(id: "inv-\(units.count + 1)", name: "Inverter \(units.count + 1)", inverterId: inverters.first.map { str($0, "id") } ?? "", mppts: [])) } }
                    }
                    IssueList(issues: dicts(m.stringingDoc?["issues"]))
                }
            }
            GroupBox(m.t("equipment")) {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(["modules", "inverters", "batteries", "battery_inverters"], id: \.self) { k in
                        ForEach(dicts(m.section("library")[k]).indices, id: \.self) { i in
                            let r = dicts(m.section("library")[k])[i]
                            HStack { Text(["modules": "module", "inverters": "inverter", "batteries": "battery", "battery_inverters": "battery inverter"][k] ?? k).foregroundStyle(.secondary).frame(width: 110, alignment: .leading); Text("\(str(r, "manufacturer")) \(str(r, "model"))"); Spacer(); Text(str(r, "provenance")).font(.caption).foregroundStyle(.secondary) }
                        }
                    }
                    HStack {
                        Button(m.t("new_module")) { editor = "module" }
                        Button(m.t("new_inverter")) { editor = "inverter" }
                        Button(m.t("new_battery")) { editor = "battery" }
                    }
                }
            }
            CatalogPanel()
            GroupBox(m.t("my_library")) {
                VStack(alignment: .leading) {
                    HStack { TextField("", text: $query).frame(width: 220); Button("🔍") { m.loadLibrary(query) } }
                    ForEach(m.libraryItems.indices, id: \.self) { i in
                        let it = m.libraryItems[i]
                        let rec = it["record"] as? JSONDict ?? [:]
                        HStack { Text(str(it, "kind")).foregroundStyle(.secondary).frame(width: 110, alignment: .leading); Text("\(str(rec, "manufacturer")) \(str(rec, "model")) · rev \(int(it, "rev"))"); Spacer(); Button(m.t("use_in_project")) { m.pin(str(it, "kind"), str(it, "id")) } }
                    }
                }
            }
        }
        .onAppear { load(); m.loadLibrary() }
        .sheet(item: Binding(get: { editor.map { EditorKind(kind: $0) } }, set: { editor = $0?.kind })) { k in EquipmentEditor(kind: k.kind) { editor = nil } }
    }

    private func load() {
        manual = (m.stringingDoc?["manual"] as? Bool) ?? false
        units = dicts(m.stringingDoc?["inverters"]).compactMap { $0["unit"] as? JSONDict }.map { u in
            UnitEdit(id: str(u, "id"), name: str(u, "name"), inverterId: str(u, "inverter_id"),
                     mppts: (u["mppts"] as? [[JSONDict]] ?? []).map { $0.map { GroupEdit(arrayId: str($0, "array_id"), modules: int($0, "modules_per_string"), strings: int($0, "strings")) } })
        }
    }

    private func apply() {
        if !manual { m.setStringing(manual: false, inverters: []); load(); return }
        let inv: [JSONDict] = units.map { u -> JSONDict in
            let mppts: [[JSONDict]] = u.mppts.map { groups in groups.map { g -> JSONDict in ["array_id": g.arrayId, "modules_per_string": g.modules, "strings": g.strings] } }
            return ["id": u.id, "name": u.name, "inverter_id": u.inverterId, "mppts": mppts]
        }
        m.setStringing(manual: true, inverters: inv)
        load()
    }
}

struct EditorKind: Identifiable { let kind: String; var id: String { kind } }

/// Offline equipment authoring with the same fields the catalog uses.
struct EquipmentEditor: View {
    @EnvironmentObject var m: AppModel
    let kind: String
    let done: () -> Void
    @State private var text: [String: String] = [:]
    @State private var values: [String: Double] = [:]
    @State private var batteryPort = true
    @State private var saveToLibrary = true

    private var numeric: [(String, String, Double)] {
        switch kind {
        case "module": return [("cells_in_series", "Cells in series", 72), ("length_m", "Length (m)", 1.76), ("width_m", "Width (m)", 1.13), ("v_oc", "Voc (V)", 49.6), ("i_sc", "Isc (A)", 11.6), ("v_mp", "Vmp (V)", 41.4), ("i_mp", "Imp (A)", 10.87), ("alpha_sc", "α Isc (A/K)", 0.0058), ("beta_voc", "β Voc (V/K)", -0.134), ("gamma_pmp", "γ Pmp (%/K)", -0.35)]
        case "inverter": return [("pac_max_w", "Pac max (W)", 5000), ("eta_nominal", "Efficiency", 0.97), ("mppt_v_min", "MPPT V min", 120), ("mppt_v_max", "MPPT V max", 500), ("v_dc_max", "V DC max", 600), ("mppt_count", "MPPT inputs", 2), ("i_max_per_mppt", "I max per MPPT (A)", 16), ("night_w", "Night consumption (W)", 2), ("port_v_min", "Battery port V min", 150), ("port_v_max", "Battery port V max", 500), ("port_p", "Battery port power (W)", 5000)]
        default: return [("capacity_kwh", "Capacity (kWh)", 10), ("nominal_voltage_v", "Nominal voltage (V)", 204.8), ("soc_min_pct", "SOC min (%)", 5), ("soc_max_pct", "SOC max (%)", 100), ("p_charge_max_kw", "Charge power (kW)", 5), ("p_discharge_max_kw", "Discharge power (kW)", 5), ("roundtrip_efficiency", "Round-trip efficiency", 0.95), ("self_discharge_pct_per_month", "Self-discharge (%/month)", 2), ("cycle_life", "Cycles to 80 %", 6000)]
        }
    }

    var body: some View {
        VStack(alignment: .leading) {
            Text(m.t("new_" + kind)).font(.title3)
            Form {
                ForEach(["id", "manufacturer", "model"], id: \.self) { k in
                    LabeledContent(k) { TextField("", text: Binding(get: { text[k] ?? "" }, set: { text[k] = $0 })).frame(width: 240) }
                }
                ForEach(numeric.indices, id: \.self) { i in
                    let f = numeric[i]
                    LabeledContent(f.1) { TextField("", value: Binding(get: { values[f.0] ?? f.2 }, set: { values[f.0] = $0 }), format: .number).frame(width: 120).multilineTextAlignment(.trailing) }
                }
                if kind == "inverter" { Toggle("Battery port (hybrid)", isOn: $batteryPort) }
                Toggle(m.t("save_to_library"), isOn: $saveToLibrary)
            }
            HStack { Spacer(); Button(m.t("cancel")) { done() }; Button(m.t("apply")) { save(); done() }.keyboardShortcut(.defaultAction) }
        }
        .padding(20).frame(width: 520)
    }

    private func v(_ k: String) -> Double { values[k] ?? numeric.first { $0.0 == k }?.2 ?? 0 }

    private func save() {
        var r: JSONDict = [:]
        r["id"] = text["id"] ?? "user-\(kind)-\(Int(Date().timeIntervalSince1970))"
        r["manufacturer"] = text["manufacturer"] ?? "User"
        r["model"] = text["model"] ?? kind
        switch kind {
        case "module":
            r["technology"] = "c-Si"
            for (k, _, _) in numeric { if k == "cells_in_series" { r[k] = Int(v(k)) } else { r[k] = v(k) } }
        case "inverter":
            for k in ["pac_max_w", "eta_nominal", "mppt_v_min", "mppt_v_max", "v_dc_max", "i_max_per_mppt", "night_w"] { r[k] = v(k) }
            r["mppt_count"] = Int(v("mppt_count"))
            if batteryPort { let port: JSONDict = ["v_min": v("port_v_min"), "v_max": v("port_v_max"), "p_charge_max_w": v("port_p"), "p_discharge_max_w": v("port_p"), "dc_dc_efficiency": 0.98]; r["battery_port"] = port }
        default:
            r["chemistry"] = "lfp"
            for (k, _, _) in numeric { r[k] = v(k) }
        }
        m.addEquipment(kind, r, saveToLibrary: saveToLibrary)
    }
}

// MARK: - simulation

struct SimulationPage: View {
    @EnvironmentObject var m: AppModel
    @State private var day = 172.0
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if let s = m.summary, let b = s["balance"] as? JSONDict {
                HStack {
                    if m.isIslanded {
                        Kpi(label: m.t("island_results"), value: fmt(num(b, "load_served_kwh"), 1), unit: "kWh")
                        Kpi(label: m.t("island_unserved"), value: fmt(num(b, "unserved_kwh"), 1), unit: "kWh")
                        Kpi(label: m.t("generator"), value: fmt(num(b, "generator_kwh"), 1), unit: "kWh")
                    } else {
                        Kpi(label: m.t("self_consumption"), value: fmt(num(b, "self_consumption_rate") * 100, 1), unit: "%")
                        Kpi(label: m.t("autarky"), value: fmt(num(b, "autarky") * 100, 1), unit: "%")
                        Kpi(label: m.t("grid_import"), value: fmt(num(b, "import_kwh")), unit: "kWh")
                        Kpi(label: m.t("grid_export"), value: fmt(num(b, "export_kwh")), unit: "kWh")
                    }
                }
                GroupBox(m.t("energy_balance")) {
                    SimpleTable(header: [], rows: [
                        ["PV DC", fmt(num(b, "pv_dc_kwh")) + " kWh"], [m.t("grid_import"), fmt(num(b, "import_kwh")) + " kWh"],
                        [m.t("loads"), fmt(num(b, "load_served_kwh")) + " kWh"], [m.t("grid_export"), fmt(num(b, "export_kwh")) + " kWh"],
                        [m.t("curtailed"), fmt(num(b, "curtailed_kwh")) + " kWh"], [m.t("clipped"), fmt(num(b, "clipped_kwh")) + " kWh"],
                        [m.t("batt_charge") + " / " + m.t("batt_discharge"), fmt(num(b, "batt_charge_kwh")) + " / " + fmt(num(b, "batt_discharge_kwh")) + " kWh"],
                        ["Δ", String(format: "%.1e Wh", num(b, "max_abs_residual_wh"))],
                    ])
                }
                GroupBox(m.t("stringing")) {
                    SimpleTable(header: [m.t("inverter") + " / MPPT", "kWp", "DC kWh", "mismatch", "limits", "h"], rows: dicts(s["electrical"]).flatMap { u -> [[String]] in
                        [[str(u, "name"), fmt(num(u, "stc_kw"), 2), fmt(num(u, "dc_kwh")), "", "", ""]] + dicts(u["mppts"]).map { mp in
                            ["  MPPT \(int(mp, "index")) · \(str(mp, "strings"))", fmt(num(mp, "stc_kw"), 2), fmt(num(mp, "dc_kwh")), fmt(num(mp, "mismatch_kwh"), 1), fmt(num(mp, "limit_loss_kwh"), 1), "\(int(mp, "limited_hours"))"]
                        }
                    })
                }
            }
            ResultsPage()
            if m.summary != nil {
                GroupBox(m.t("hours_inspector")) {
                    VStack(alignment: .leading) {
                        HStack {
                            Stepper("\(m.t("day")) \(Int(day))", value: $day, in: 1...365)
                            Button(m.t("apply")) { m.loadDay(Int(day)) }
                            Text(m.t("top_hours")).foregroundStyle(.secondary)
                            ForEach(["import", "export", "clipped", "curtailed", "unserved", "-soc"], id: \.self) { c in Button(c) { m.loadTop(c) }.buttonStyle(.link) }
                        }
                        if !m.dayRows.isEmpty {
                            Chart {
                                ForEach(m.dayRows.indices, id: \.self) { i in
                                    let r = m.dayRows[i]
                                    LineMark(x: .value("h", i), y: .value("Wh", num(r, "pv_ac_wh")), series: .value("s", "PV")).foregroundStyle(.yellow)
                                    LineMark(x: .value("h", i), y: .value("Wh", num(r, "load_wh")), series: .value("s", "Load")).foregroundStyle(.blue)
                                    LineMark(x: .value("h", i), y: .value("Wh", num(r, "import_wh")), series: .value("s", "Import")).foregroundStyle(.red)
                                    LineMark(x: .value("h", i), y: .value("Wh", num(r, "batt_discharge_wh") - num(r, "batt_charge_wh")), series: .value("s", "Battery")).foregroundStyle(.teal)
                                }
                            }.frame(height: 180)
                            SimpleTable(header: [m.t("day"), m.t("hour"), "PV", m.t("loads"), "→🔋", "🔋→", m.t("grid_import"), m.t("grid_export"), m.t("soc"), m.t("backup_reserve") + " Wh"],
                                        rows: m.dayRows.map { r in ["\(int(r, "day_of_year"))", String(format: "%02d:00", int(r, "hour")) + ((r["outage"] as? Bool) == true ? " ⚡︎" : ""), fmt(num(r, "pv_ac_wh")), fmt(num(r, "load_wh")), fmt(num(r, "batt_charge_wh")), fmt(num(r, "batt_discharge_wh")), fmt(num(r, "import_wh")), fmt(num(r, "export_wh")), fmt(num(r, "soc_pct"), 1), fmt(num(r, "backup_wh"))] })
                                .font(.caption)
                        }
                    }
                }
            }
        }
    }
}

// MARK: - economics and variants

struct TouEdit: Identifiable { let id = UUID(); var name = "Peak"; var price = 0.4; var start = 18.0; var end = 22.0; var weekdays = true; var weekends = true }

struct EconomicsPage: View {
    @EnvironmentObject var m: AppModel
    @State private var t: JSONDict = [:]
    @State private var e: JSONDict = [:]
    @State private var currency = "EUR"
    @State private var importPrice = 0.3
    @State private var exportKind = "fixed"
    @State private var exportPrice = 0.08
    @State private var fixedMonthly = 0.0
    @State private var demand = 0.0
    @State private var escI = 2.0
    @State private var escE = 0.0
    @State private var tou: [TouEdit] = []
    @State private var econ: [String: Double] = [:]
    @State private var loanOn = false
    @State private var variantName = ""

    private let econFields: [(String, String)] = [("horizon_years", "horizon"), ("discount_rate_pct", "discount_rate"), ("cost_fixed", "cost_fixed"), ("cost_per_kwp", "cost_per_kwp"), ("cost_per_battery_kwh", "cost_per_kwh"), ("om_pct_of_investment", "om"), ("degradation_pct_per_year", "degradation")]

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            GroupBox(m.t("tariff")) {
                Form {
                    LabeledContent(m.t("currency")) { TextField("", text: $currency).frame(width: 80) }
                    NumField(label: m.t("import_price"), value: $importPrice)
                    Picker(m.t("grid_export"), selection: $exportKind) { Text(m.t("export_price")).tag("fixed"); Text(m.t("net_metering")).tag("net_metering"); Text(m.t("gs_none")).tag("none") }
                    if exportKind != "none" { NumField(label: m.t("export_price"), value: $exportPrice) }
                    NumField(label: m.t("fixed_monthly"), value: $fixedMonthly)
                    NumField(label: m.t("demand_charge"), value: $demand)
                    HStack { NumField(label: m.t("escalation"), value: $escI); TextField("", value: $escE, format: .number).frame(width: 60) }
                    Text(m.t("tou_periods")).font(.headline)
                    ForEach($tou) { $p in
                        HStack {
                            TextField("", text: $p.name).frame(width: 90)
                            TextField("", value: $p.price, format: .number).frame(width: 70)
                            TextField("", value: $p.start, format: .number).frame(width: 40); Text("–"); TextField("", value: $p.end, format: .number).frame(width: 40)
                            Toggle("Mo–Fr", isOn: $p.weekdays); Toggle("Sa–Su", isOn: $p.weekends)
                            Button(m.t("remove")) { tou.removeAll { $0.id == p.id } }
                        }
                    }
                    Button(m.t("add_period")) { tou.append(TouEdit(price: importPrice * 1.4)) }
                    Button(m.t("apply")) { applyTariff() }
                }
            }
            GroupBox(m.t("costs")) {
                Form {
                    ForEach(econFields.indices, id: \.self) { i in let f = econFields[i]; NumField(label: m.t(f.1), value: Binding(get: { econ[f.0] ?? 0 }, set: { econ[f.0] = $0 })) }
                    Toggle(m.t("loan"), isOn: $loanOn)
                    if loanOn { HStack { NumField(label: m.t("loan"), value: Binding(get: { econ["loan_amount"] ?? 0 }, set: { econ["loan_amount"] = $0 })); TextField("%", value: Binding(get: { econ["loan_rate"] ?? 5 }, set: { econ["loan_rate"] = $0 }), format: .number).frame(width: 50); TextField("y", value: Binding(get: { econ["loan_years"] ?? 10 }, set: { econ["loan_years"] = $0 }), format: .number).frame(width: 40) } }
                    Button(m.t("apply")) { applyEconomics() }
                }
            }
            if let c = m.cash, !dicts(c["years"]).isEmpty {
                GroupBox(m.t("cash_flow")) {
                    VStack(alignment: .leading) {
                        HStack {
                            Kpi(label: m.t("npv"), value: fmt(num(c, "npv")), unit: str(c, "currency"))
                            Kpi(label: m.t("irr"), value: (c["irr_pct"] as? NSNumber).map { fmt($0.doubleValue, 1) } ?? "–", unit: "%")
                            Kpi(label: m.t("payback"), value: (c["payback_years"] as? NSNumber).map { fmt($0.doubleValue, 1) } ?? "–", unit: "a")
                            Kpi(label: "LCOE", value: fmt(num(c, "lcoe_per_kwh"), 3), unit: str(c, "currency") + "/kWh")
                        }
                        Chart(dicts(c["years"]).indices, id: \.self) { i in
                            let y = dicts(c["years"])[i]
                            BarMark(x: .value("year", int(y, "year")), y: .value("cumulative", num(y, "cumulative"))).foregroundStyle(num(y, "cumulative") < 0 ? Color.red.opacity(0.7) : Color.green.opacity(0.7))
                        }.frame(height: 160)
                        SimpleTable(header: ["", "savings", "export", "O&M", "repl.", "loan", m.t("cash_flow"), "Σ"], rows: dicts(c["years"]).map { y in
                            ["\(int(y, "year"))", fmt(num(y, "savings")), fmt(num(y, "export_revenue")), fmt(num(y, "om")), fmt(num(y, "replacements")), fmt(num(y, "loan")), fmt(num(y, "cash_flow")), fmt(num(y, "cumulative"))]
                        }).font(.caption)
                    }
                }
            }
            GroupBox(m.t("variants")) {
                VStack(alignment: .leading) {
                    HStack { TextField("", text: $variantName).frame(width: 220); Button(m.t("save_variant")) { m.saveVariant(variantName.isEmpty ? "Variant" : variantName); variantName = "" }; Button(m.t("compare")) { m.compareVariants() } }
                    ForEach(dicts(m.raw["variants"]).indices, id: \.self) { i in
                        let v = dicts(m.raw["variants"])[i]
                        HStack { Text(str(v, "name")); Spacer(); Button(m.t("activate")) { m.applyVariant(str(v, "id")) } }
                    }
                    if !m.variantRows.isEmpty {
                        SimpleTable(header: ["", "kWp", "kWh bat", "AC kWh", "SC %", m.t("autarky") + " %", m.t("grid_import"), m.t("npv"), m.t("irr"), m.t("payback")], rows: m.variantRows.map { r in
                            [str(r, "name"), fmt(num(r, "dc_kwp"), 2), fmt(num(r, "battery_kwh"), 1), fmt(num(r, "ac_kwh")), fmt(num(r, "self_consumption_pct"), 1), fmt(num(r, "autarky_pct"), 1), fmt(num(r, "import_kwh")), fmt(num(r, "npv")),
                             (r["irr_pct"] as? NSNumber).map { fmt($0.doubleValue, 1) } ?? "–", (r["payback_years"] as? NSNumber).map { fmt($0.doubleValue, 1) } ?? "–"]
                        }).font(.caption)
                    }
                }
            }
        }
        .onAppear(perform: load)
    }

    private func load() {
        t = m.section("tariff"); e = m.section("economics")
        currency = str(t, "currency"); importPrice = num(t, "import_price"); fixedMonthly = num(t, "fixed_per_month"); demand = num(t, "demand_charge_per_kw_month")
        escI = num(t, "import_escalation_pct"); escE = num(t, "export_escalation_pct")
        let ex = t["export"] as? JSONDict ?? [:]
        exportKind = str(ex, "kind").isEmpty ? "fixed" : str(ex, "kind")
        exportPrice = num(ex, exportKind == "net_metering" ? "surplus_price" : "price")
        tou = dicts(t["tou"]).map { TouEdit(name: str($0, "name"), price: num($0, "price"), start: num($0, "start_hour"), end: num($0, "end_hour"), weekdays: ($0["weekdays"] as? Bool) ?? true, weekends: ($0["weekends"] as? Bool) ?? true) }
        for (k, _) in econFields { econ[k] = num(e, k) }
        let loan = e["loan"] as? JSONDict
        loanOn = loan != nil
        econ["loan_amount"] = loan.map { num($0, "amount") } ?? 0; econ["loan_rate"] = loan.map { num($0, "rate_pct") } ?? 5; econ["loan_years"] = loan.map { num($0, "years") } ?? 10
    }

    private func applyTariff() {
        var nt = t
        nt["currency"] = currency; nt["import_price"] = importPrice; nt["fixed_per_month"] = fixedMonthly; nt["demand_charge_per_kw_month"] = demand
        nt["import_escalation_pct"] = escI; nt["export_escalation_pct"] = escE
        let ex: JSONDict
        switch exportKind {
        case "none": ex = ["kind": "none"]
        case "net_metering": ex = ["kind": "net_metering", "surplus_price": exportPrice]
        default: ex = ["kind": "fixed", "price": exportPrice]
        }
        nt["export"] = ex
        nt["tou"] = tou.map { p -> JSONDict in ["name": p.name, "price": p.price, "start_hour": Int(p.start), "end_hour": Int(p.end), "weekdays": p.weekdays, "weekends": p.weekends, "months": [Int]()] }
        if str(nt, "name").isEmpty { nt["name"] = "Tariff" }
        m.setSection("tariff", nt)
    }

    private func applyEconomics() {
        var ne = e
        for (k, _) in econFields { if k == "horizon_years" { ne[k] = Int(econ[k] ?? 20) } else { ne[k] = econ[k] ?? 0 } }
        if loanOn {
            let loan: JSONDict = ["amount": econ["loan_amount"] ?? 0, "rate_pct": econ["loan_rate"] ?? 5, "years": Int(econ["loan_years"] ?? 10)]
            ne["loan"] = loan
        } else {
            ne["loan"] = NSNull()
        }
        m.setSection("economics", ne)
    }
}

// MARK: - design schedules

struct CablesPage: View {
    @EnvironmentObject var m: AppModel
    @State private var dc = 0.0
    @State private var ac = 0.0

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(m.t("workflow_cable_hint")).foregroundStyle(.secondary)
            Form {
                NumField(label: m.t("workflow_dc_loss"), value: $dc)
                NumField(label: m.t("workflow_ac_loss"), value: $ac)
                Button(m.t("apply")) {
                    m.setCableLosses(dc: dc, ac: ac)
                }.disabled(!(0..<100).contains(dc) || !(0..<100).contains(ac))
            }
            CablePlannerSection()
            let routes = dicts(m.designSchedule?["routes"])
            if routes.isEmpty {
                Text(m.t("workflow_no_routes")).foregroundStyle(.secondary)
            } else {
                ScrollView(.horizontal) {
                    SimpleTable(header: ["photo_array", "workflow_string", "module", "workflow_jumper", "workflow_home_run"].map { m.t($0) },
                                rows: routes.map { route in [
                                    str(route, "field"), str(route, "id"), fmt(num(route, "modules")),
                                    fmt(num(route, "jumper_m"), 1),
                                    route["home_run_m"] is NSNumber ? fmt(num(route, "home_run_m"), 1) : "–",
                                ] })
                }
            }
        }.onAppear {
            let losses = m.section("losses")
            dc = num(losses["dc_wiring_pct"] as? JSONDict ?? [:], "value")
            ac = num(losses["ac_wiring_pct"] as? JSONDict ?? [:], "value")
        }
    }
}

struct EngineeringPlansPage: View {
    @EnvironmentObject var m: AppModel
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(m.t("workflow_parts")).font(.headline)
            let equipment = dicts(m.designSchedule?["equipment"])
            SimpleTable(header: [m.t("family"), m.t("model"), m.t("workflow_quantity")],
                        rows: equipment.map { [m.t(str($0, "kind")), str($0, "model"), fmt(num($0, "quantity"))] })
            let arrays = m.project?.arrays ?? []
            ForEach(arrays.indices, id: \.self) { index in
                let array = arrays[index]
                if let roof = array.roof {
                    Divider()
                    Text(array.name.value).font(.headline)
                    RoofPreview(roof: roof).frame(height: 280)
                    Text("\(roof.modules.count) \(m.t("modules_fit"))").foregroundStyle(.secondary)
                }
            }
        }
    }
}

// MARK: - catalog

/// Search the imported catalog (companies, countries, revisions) and use entries in the project.
struct CatalogPanel: View {
    @EnvironmentObject var m: AppModel
    @State private var family = "module"
    @State private var query = ""
    private let families = ["module", "pvinverter", "batterysystem", "batteryinverter", "generator", "demandtariff", "netmeteringtariff", "feedintariff"]
    var body: some View {
        GroupBox(m.t("catalog")) {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Picker("", selection: $family) { ForEach(families, id: \.self) { Text($0).tag($0) } }.frame(width: 180)
                    TextField("", text: $query).frame(width: 240).onSubmit { m.catalogSearch(family, query) }
                    Button(m.t("search")) { m.catalogSearch(family, query) }
                    Spacer()
                    Button(m.t("import_catalog")) { m.importCatalog() }
                }
                ForEach(m.catalogHits.indices, id: \.self) { i in
                    let h = m.catalogHits[i]
                    HStack {
                        VStack(alignment: .leading) {
                            Text(str(h, "name")).fontWeight(.medium)
                            Text("\(str(h, "company")) \(str(h, "country")) · v\(int(h, "version")) · \(int(h, "versions")) \(m.t("revisions")) · \(h["summary"] as? String ?? "")").font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        if str(h, "family").hasSuffix("tariff") {
                            Button(str(h, "family") == "feedintariff" ? m.t("use_as_export") : m.t("use_as_import")) { m.catalogTariff(str(h, "family"), str(h, "id")) }
                        } else {
                            Button(m.t("use_in_project")) { m.catalogPin(str(h, "family"), str(h, "id")) }
                        }
                    }
                }
            }
        }
    }
}
