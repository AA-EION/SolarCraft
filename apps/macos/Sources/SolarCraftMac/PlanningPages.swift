// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct SystemWorkflowSection: View {
    @EnvironmentObject var m: AppModel
    @State private var system = "custom"
    @State private var tools = "both"
    var body: some View {
        GroupBox(m.t("workflow_type")) {
            Form {
                Picker(m.t("workflow_type"), selection: $system) {
                    ForEach(["custom", "grid_export", "grid_consumption", "grid_battery", "grid_ev", "grid_ev_battery", "grid_thermal", "grid_thermal_battery", "off_grid", "off_grid_generator"], id: \.self) { key in
                        Text(m.t(key)).tag(key)
                    }
                }
                Picker(m.t("workflow_tools"), selection: $tools) {
                    ForEach(["both", "roof", "scene"], id: \.self) { key in Text(m.t("tools_" + key)).tag(key) }
                }
                Text(m.t("workflow_type_hint")).foregroundStyle(.secondary)
                Button(m.t("apply")) { m.setWorkflow(system: system, tools: tools) }
            }
        }.onAppear {
            system = m.systemType
            tools = m.workflowSettings["tools"] as? String ?? "both"
        }
    }
}

private struct CableDraft: Identifiable {
    let id = UUID()
    let index: Int?
    var label: String
    var kind: String
    var metal: String
    var length: Double
    var extra: Double
    var area: Double
    var temperature: Double
    var voltage: Double
    var current: Double
    var count: Double
    var target: Double
    var ampacity: String
    var derating: Double
    var sizingFactor: Double
    var protection: String
    var link: String
    var candidates: String
    let source: String?

    init(_ value: JSONDict? = nil, index: Int? = nil, name: String) {
        let v = value ?? [:]
        self.index = index
        label = v["label"] as? String ?? name
        kind = v["kind"] as? String ?? "dc_string"
        metal = v["metal"] as? String ?? "copper"
        length = num(v, "length_m", 20)
        extra = num(v, "extra_m", 0)
        area = num(v, "area_mm2", 4)
        temperature = num(v, "temperature_c", 60)
        voltage = num(v, "voltage_v", 400)
        current = num(v, "current_a", 10)
        count = num(v, "circuits", 1)
        target = num(v, "target_drop_pct", 1.5)
        ampacity = (v["ampacity_a"] as? NSNumber)?.stringValue ?? ""
        derating = num(v, "derating", 1)
        sizingFactor = num(v, "sizing_factor", 1)
        protection = (v["protection_a"] as? NSNumber)?.stringValue ?? ""
        if let binding = v["binding"] as? JSONDict {
            let id = str(binding, "unit_id")
            link = str(binding, "kind") == "mppt" ? "dc|\(id)|\(int(binding, "input"))" : "ac|\(id)"
        } else { link = "" }
        candidates = dicts(v["candidates"]).map { "\(num($0, "area_mm2")),\(num($0, "ampacity_a"))" }.joined(separator: "; ")
        source = value?["source_route"] as? String
    }

    var encoded: JSONDict? {
        var sizes: [JSONDict] = []
        for item in candidates.split(separator: ";") {
            let pair = item.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
            guard pair.count == 2, let area = Double(pair[0]), let amps = Double(pair[1]), area.isFinite, amps.isFinite else { return nil }
            sizes.append(["area_mm2": area, "ampacity_a": amps])
        }
        var value: JSONDict = ["label": label, "kind": kind, "metal": metal, "length_m": length, "extra_m": kind.hasPrefix("dc_") ? extra : 0,
            "area_mm2": area, "temperature_c": temperature, "voltage_v": voltage, "current_a": current, "circuits": Int(count), "target_drop_pct": target]
        if !ampacity.isEmpty {
            guard let a = Double(ampacity.replacingOccurrences(of: ",", with: ".")), a.isFinite else { return nil }
            value["ampacity_a"] = a
        }
        value["derating"] = derating
        value["sizing_factor"] = sizingFactor
        if !protection.isEmpty {
            guard let rating = Double(protection.replacingOccurrences(of:",",with:".")), rating.isFinite else { return nil }
            value["protection_a"] = rating
        }
        let parts = link.split(separator: "|").map(String.init)
        if kind.hasPrefix("dc_"), parts.count == 3, parts[0] == "dc", let input = Int(parts[2]) {
            value["binding"] = ["kind":"mppt", "unit_id":parts[1], "input":input] as JSONDict
        } else if !kind.hasPrefix("dc_"), parts.count == 2, parts[0] == "ac" {
            value["binding"] = ["kind":"inverter_ac", "unit_id":parts[1]]
        }
        value["candidates"] = sizes
        if let source { value["source_route"] = source }
        return value
    }
}

struct CablePlannerSection: View {
    @EnvironmentObject var m: AppModel
    @State private var data: JSONDict = [:]
    @State private var draft: CableDraft?
    @State private var selected = 0
    private var circuits: [JSONDict] { dicts(data["circuits"]) }
    private var estimates: [JSONDict] { (data["estimates"] as? [Any] ?? []).map { $0 as? JSONDict ?? [:] } }

    var body: some View {
        GroupBox(m.t("cable_planner")) {
            VStack(alignment: .leading, spacing: 10) {
                Text(m.t("cable_planner_hint")).foregroundStyle(.secondary)
                Text(m.t("cable_hourly_hint")).foregroundStyle(.secondary)
                if !circuits.isEmpty {
                    Picker(m.t("cable_planner"), selection: $selected) {
                        ForEach(circuits.indices, id: \.self) { i in Text(str(circuits[i], "label")).tag(i) }
                    }
                }
                HStack {
                    Button(m.t("cable_add")) { draft = CableDraft(name: m.t("cable_dc_string")) }
                    Button(m.t("cable_edit")) {
                        draft = CableDraft(circuits[selected], index: selected, name: m.t("cable_dc_string"))
                    }.disabled(!circuits.indices.contains(selected))
                    Button(m.t("remove")) {
                        var rows = circuits; rows.remove(at: selected)
                        run("preview", rows)
                    }.disabled(!circuits.indices.contains(selected))
                    Button(m.t("cable_import")) { run("import_routes", circuits) }
                }
                if !estimates.isEmpty {
                    ScrollView(.horizontal) {
                        SimpleTable(header: [m.t("cable_planner"), m.t("cable_drop"), "W", m.t("cable_area_target"), m.t("cable_current_check"), m.t("cable_suggested")],
                            rows: zip(circuits, estimates).map { c, r in
                                if r.isEmpty { return [str(c,"label"),"–","–","–","–","–"] }
                                let target = num(c,"design_current_a",num(c,"current_a")) * num(c,"sizing_factor",1)
                                let current = r["allowed_a"] is NSNumber ? "\(fmt(num(r, "allowed_a"), 1)) / \(fmt(target, 1))" : "–"
                                let size = (r["suggested"] as? JSONDict).map { fmt(num($0, "area_mm2"), 1) + " mm²" } ?? "–"
                                return [str(c, "label"), fmt(num(r, "drop_pct"), 2) + " %", fmt(num(r, "loss_w"), 1),
                                    fmt(num(r, "area_for_target_mm2"), 2), current, size]
                            })
                    }
                    if estimates.indices.contains(selected), let recommendation = estimates[selected]["suggested"] as? JSONDict {
                        Button(m.t("cable_use_size")) {
                            var rows = circuits
                            rows[selected]["area_mm2"] = recommendation["area_mm2"]
                            rows[selected]["ampacity_a"] = recommendation["ampacity_a"]
                            run("preview", rows)
                        }
                    }
                    Text("\(m.t("workflow_dc_loss")) \(percent("dc_loss_pct"))   \(m.t("workflow_ac_loss")) \(percent("ac_loss_pct"))")
                        .monospacedDigit()
                }
                HStack {
                    Button(m.t("cable_save")) { run("save", circuits) }
                    Button(m.t("cable_apply")) { run("apply", circuits) }.disabled(circuits.isEmpty)
                    Button(m.t("cable_hourly")) { run("apply_hourly", circuits) }.buttonStyle(.borderedProminent)
                        .disabled(circuits.isEmpty)
                    Button(m.t("cable_apply_routed")) { run("apply_routed", circuits) }
                        .disabled(!circuits.contains { $0["binding"] is JSONDict })
                }
                ForEach((data["row_errors"] as? [Any] ?? []).compactMap { $0 as? String }, id: \.self) { detail in
                    Text(detail).foregroundStyle(.red)
                }
                let losses = dicts(m.summary?["routed_cables"])
                if !losses.isEmpty {
                    SimpleTable(header: [m.t("cable_planner"), m.t("cable_routed_loss")],
                        rows: losses.map { [str($0,"label"), fmt(num($0,"loss_kwh"),2)] })
                }
            }
        }
        .onAppear { if let state = m.cablePlan(mode: "preview") { data = state } }
        .sheet(item: $draft) { start in
            CableEditor(value: start) { value in
                guard let encoded = value.encoded else { m.error = m.t("cable_candidates"); return false }
                guard let check = m.cablePlan(mode: "preview", circuits: [encoded]) else { return false }
                if let issue = (check["row_errors"] as? [Any])?.first as? String { m.error = issue; return false }
                var rows = circuits
                if let index = value.index { rows[index] = encoded }
                else { rows.append(encoded) }
                let saved = run("preview", rows)
                if saved { selected = value.index ?? rows.count - 1 }
                return saved
            }.environmentObject(m)
        }
    }

    private func percent(_ key: String) -> String {
        data[key] is NSNumber ? fmt(num(data, key), 3) : "–"
    }

    @discardableResult
    private func run(_ mode: String, _ rows: [JSONDict]) -> Bool {
        guard let result = m.cablePlan(mode: mode, circuits: rows) else { return false }
        data = result
        selected = min(selected, max(0, circuits.count - 1))
        return true
    }
}

private struct CableEditor: View {
    @EnvironmentObject var m: AppModel
    @Environment(\.dismiss) private var dismiss
    @State var value: CableDraft
    let submit: (CableDraft) -> Bool
    @State private var error = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Form {
                TextField(m.t("cable_planner"), text: $value.label)
                Picker(m.t("cable_kind"), selection: $value.kind) {
                    ForEach(["dc_string", "dc_main", "ac_single", "ac_three"], id: \.self) { Text(m.t("cable_" + $0)).tag($0) }
                }
                Picker(m.t("cable_link"), selection: $value.link) {
                    Text(m.t("cable_unbound")).tag("")
                    let units = dicts(m.stringingDoc?["inverters"]).compactMap { $0["unit"] as? JSONDict }
                    ForEach(units.indices, id: \.self) { j in
                        let id = str(units[j], "id")
                        let name = str(units[j], "name")
                        if value.kind.hasPrefix("dc_") {
                            ForEach(0..<(units[j]["mppts"] as? [Any] ?? []).count, id: \.self) { i in
                                Text("\(name) / MPPT \(i+1)").tag("dc|\(id)|\(i)")
                            }
                        } else { Text(name).tag("ac|\(id)") }
                    }
                }
                Picker(m.t("cable_material"), selection: $value.metal) {
                    ForEach(["copper", "aluminium"], id: \.self) { Text(m.t("cable_" + $0)).tag($0) }
                }
                NumField(label: m.t("cable_length"), value: $value.length)
                if value.kind.hasPrefix("dc_") { NumField(label: m.t("cable_extra"), value: $value.extra) }
                NumField(label: m.t("cable_area"), value: $value.area)
                NumField(label: m.t("cable_temp"), value: $value.temperature)
                NumField(label: m.t("cable_voltage"), value: $value.voltage).disabled(value.link.hasPrefix("dc|"))
                NumField(label: m.t("cable_current"), value: $value.current).disabled(!value.link.isEmpty)
                NumField(label: m.t("cable_count"), value: $value.count)
                NumField(label: m.t("cable_target"), value: $value.target)
                TextField(m.t("cable_ampacity"), text: $value.ampacity)
                NumField(label: m.t("cable_derating"), value: $value.derating)
                NumField(label: m.t("cable_sizing_factor"), value: $value.sizingFactor)
                TextField(m.t("cable_protection"), text: $value.protection)
                TextField(m.t("cable_candidates"), text: $value.candidates)
            }
            if !error.isEmpty { Text(error).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button(m.t("cancel")) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(m.t("cable_preview")) {
                    if submit(value) { dismiss() } else { error = m.error ?? m.t("issues") }
                }.buttonStyle(.borderedProminent).disabled(value.count < 1 || value.count > 10_000 || value.count != value.count.rounded())
            }
        }.padding(20).frame(width: 495, height: 630)
        .onChange(of: value.kind) { old, new in
            if old.hasPrefix("dc_") != new.hasPrefix("dc_") { value.link = "" }
        }
    }
}

struct GeneratorEditor: View {
    @EnvironmentObject var m: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var rated = 3000.0
    @State private var maxPower = 3000.0
    @State private var fuel = 1.0
    @State private var unit = "L"
    var body: some View {
        Form {
            TextField(m.t("model"), text: $name)
            NumField(label: m.t("generator_rated"), value: $rated)
            NumField(label: m.t("generator_max"), value: $maxPower)
            NumField(label: m.t("generator_fuel_rate"), value: $fuel)
            TextField(m.t("generator_fuel_unit"), text: $unit)
            HStack {
                Button(m.t("cancel")) { dismiss() }
                Button(m.t("apply")) {
                    let grid = m.section("grid")
                    let record: JSONDict = ["id": "generator-" + UUID().uuidString, "manufacturer": "", "model": name,
                        "rated_va": rated, "max_va": maxPower, "voltage_v": num(grid, "voltage_v", 230),
                        "phases": int(grid, "phases", 1), "fuel_curve": [[1.0, fuel]], "fuel_unit": unit]
                    m.addEquipment("generator", record, saveToLibrary: false)
                    if m.error == nil { dismiss() }
                }.disabled(name.isEmpty || rated <= 0 || maxPower < rated || fuel < 0)
            }
        }.padding(20).frame(width: 440)
    }
}
