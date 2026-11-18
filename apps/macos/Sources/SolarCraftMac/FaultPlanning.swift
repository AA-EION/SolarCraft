// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct FaultOptions {
    var studyEnabled: Bool
    var pairEnabled: Bool
    var localRule: String
    var values: [String: String] = [:]

    static let paths = ["line_neutral","line_earth","line_line","three_phase"]
    static let ruleNames = ["retie_inverter_ac","retie_service","retie_overhead_service","retie_feeder_branch","authority"]
    static let impedance = [("r_mohm","fault_r"),("x_mohm","fault_x"),("voltage_factor","fault_voltage_factor")]
    static let conductor = [("remote_temperature_c","fault_temp"),("path_x_mohm_per_m","fault_path_x"),("return_area_mm2","fault_return_area")]
    static let settings = [("minimum_ratio","fault_margin"),("curve_limit_ka","fault_curve_limit"),("maximum_clear_seconds","fault_clear_limit")]
    static let vendor = [("maximum_ka","fault_vendor_ka"),("voltage_v","cable_voltage")]

    init(_ row: JSONDict) {
        let code = row["code_protection"] as? JSONDict ?? [:]
        let study = code["fault_study"] as? JSONDict
        let pair = code["coordination"] as? JSONDict
        let local = row["local_rule"] as? JSONDict ?? [:]
        studyEnabled = study != nil
        pairEnabled = pair != nil
        localRule = str(local,"rule")
        func read(_ prefix: String, from object: JSONDict, keys: [String]) {
            for key in keys {
                if let n = object[key] as? NSNumber { values[prefix + key] = n.stringValue }
                else if let text = object[key] as? String { values[prefix + key] = text }
            }
        }
        read("study.", from:study ?? [:], keys:Self.conductor.map { $0.0 } + ["return_metal"])
        for source in ["strong","weak"] {
            let config = study?[source] as? JSONDict ?? [:]
            read(source + ".", from:config, keys:Self.impedance.map { $0.0 } + ["path"])
        }
        values["strong.path"] = values["strong.path"] ?? "line_neutral"
        values["weak.path"] = values["weak.path"] ?? "line_neutral"
        read("pair.", from:pair ?? [:], keys:Self.settings.map { $0.0 })
        for role in ["upstream","downstream"] {
            let curve = pair?[role] as? JSONDict ?? [:]
            read(role + ".", from:curve, keys:["kind","model","reference"])
            values[role + ".kind"] = values[role + ".kind"] ?? "breaker"
            if let points = curve["points"] as? [[NSNumber]] {
                values[role + ".points"] = points.map { $0.map(\.stringValue).joined(separator:",") }.joined(separator:"; ")
            }
        }
        read("vendor.", from:pair?["manufacturer"] as? JSONDict ?? [:], keys:Self.vendor.map { $0.0 } + ["reference"])
        read("local.", from:local, keys:["other_segments_pct","reviewed_segment_pct","authority_limit_pct","authority_reference","wire_awg"])
        for key in ["aerial_cable_certified","aa8000"] {
            if let state = local[key] as? Bool { values["local." + key] = state ? "yes" : "no" }
        }
    }

    private enum Input: Error { case number }

    private func numbers(_ prefix: String, _ keys: [String]) throws -> JSONDict {
        var result: JSONDict = [:]
        for key in keys {
            let text = values[prefix + key]?.trimmingCharacters(in:.whitespacesAndNewlines) ?? ""
            if text.isEmpty { continue }
            guard let number = Double(text.replacingOccurrences(of:",",with:".")), number.isFinite else { throw Input.number }
            result[key] = number
        }
        return result
    }

    func study() throws -> JSONDict? {
        guard studyEnabled else { return nil }
        var config = try numbers("study.", Self.conductor.map { $0.0 })
        for source in ["strong","weak"] {
            var spec = try numbers(source + ".", Self.impedance.map { $0.0 })
            spec["path"] = values[source + ".path"]
            config[source] = spec
        }
        if let metal = values["study.return_metal"], !metal.isEmpty { config["return_metal"] = metal }
        return config
    }

    func pair() throws -> JSONDict? {
        guard pairEnabled else { return nil }
        var config = try numbers("pair.", Self.settings.map { $0.0 })
        for role in ["upstream","downstream"] {
            var spec: JSONDict = [:]
            for key in ["kind","model","reference"] { spec[key] = values[role + "." + key] }
            let raw = values[role + ".points"] ?? ""
            spec["points"] = try raw.split(whereSeparator: { $0 == ";" || $0 == "\n" || $0 == "\r" }).map { line -> [Double] in
                let pair = line.split(separator:",",omittingEmptySubsequences:false)
                guard pair.count == 2,
                    let a = Double(pair[0].trimmingCharacters(in:.whitespaces)),
                    let s = Double(pair[1].trimmingCharacters(in:.whitespaces)), a.isFinite, s.isFinite else { throw Input.number }
                return [a,s]
            }
            config[role] = spec
        }
        var certificate = try numbers("vendor.", Self.vendor.map { $0.0 })
        let reference = values["vendor.reference"] ?? ""
        if !reference.isEmpty || !certificate.isEmpty {
            certificate["reference"] = reference
            config["manufacturer"] = certificate
        }
        return config
    }

    func local() throws -> JSONDict? {
        guard !localRule.isEmpty else { return nil }
        var result = try numbers("local.", ["other_segments_pct","reviewed_segment_pct"])
        result["rule"] = localRule
        if localRule == "authority" {
            result["authority_limit_pct"] = try numbers("local.", ["authority_limit_pct"])["authority_limit_pct"]
            result["authority_reference"] = values["local.authority_reference"]
        }
        if localRule == "retie_overhead_service" {
            if let gauge = values["local.wire_awg"], !gauge.isEmpty {
                guard let awg = Int(gauge) else { throw Input.number }
                result["wire_awg"] = awg
            }
            for key in ["aerial_cable_certified","aa8000"] {
                if let selected = values["local." + key], !selected.isEmpty { result[key] = selected == "yes" }
            }
        }
        return result
    }
}

struct FaultForms: View {
    @EnvironmentObject var m: AppModel
    @Binding var options: FaultOptions
    let showStudy: Bool

    private func value(_ key: String) -> Binding<String> {
        Binding(get: { options.values[key] ?? "" }, set: { options.values[key] = $0 })
    }

    private func inputs(_ prefix: String, _ fields: [(String,String)]) -> some View {
        ForEach(fields.indices, id: \.self) { index in
            TextField(m.t(fields[index].1), text:value(prefix + fields[index].0))
        }
    }

    var body: some View {
        if showStudy {
            DisclosureGroup(m.t("fault_study")) {
                Toggle(m.t("advanced_enable"), isOn:$options.studyEnabled)
                if options.studyEnabled {
                    Text(m.t("fault_source_hint")).foregroundStyle(.secondary)
                    ForEach(["strong","weak"], id: \.self) { role in
                        Section(m.t("fault_" + role)) {
                            Picker(m.t("fault_path"), selection:value(role + ".path")) {
                                ForEach(FaultOptions.paths, id: \.self) { Text(m.t("fault_" + $0)).tag($0) }
                            }
                            inputs(role + ".", FaultOptions.impedance)
                        }
                    }
                    inputs("study.", FaultOptions.conductor)
                    Picker(m.t("fault_return_metal"), selection:value("study.return_metal")) {
                        Text(m.t("cable_material")).tag("")
                        Text(m.t("cable_copper")).tag("copper")
                        Text(m.t("cable_aluminium")).tag("aluminium")
                    }
                }
            }
            DisclosureGroup(m.t("fault_coordination")) {
                Toggle(m.t("advanced_enable"), isOn:$options.pairEnabled)
                if options.pairEnabled {
                    inputs("pair.", FaultOptions.settings)
                    ForEach(["upstream","downstream"], id: \.self) { role in
                        Section(m.t("fault_" + role)) {
                            Picker(m.t("fault_device"), selection:value(role + ".kind")) {
                                Text(m.t("fault_breaker")).tag("breaker")
                                Text(m.t("fault_fuse")).tag("fuse")
                            }
                            TextField(m.t("model"), text:value(role + ".model"))
                            TextField(m.t("fault_reference"), text:value(role + ".reference"))
                            TextField(m.t("fault_curve_points"), text:value(role + ".points"))
                        }
                    }
                    DisclosureGroup(m.t("fault_vendor")) {
                        inputs("vendor.", FaultOptions.vendor)
                        TextField(m.t("fault_reference"), text:value("vendor.reference"))
                    }
                }
            }
        }
        localRules
    }

    private var localRules: some View {
        DisclosureGroup(m.t("local_cable")) {
            Picker(m.t("code_rule_set"), selection:$options.localRule) {
                Text(m.t("none")).tag("")
                ForEach(FaultOptions.ruleNames, id: \.self) { Text(m.t("local_" + $0)).tag($0) }
            }
            if !options.localRule.isEmpty {
                inputs("local.", [("other_segments_pct","local_other"),("reviewed_segment_pct","local_reviewed")])
                if options.localRule == "authority" {
                    inputs("local.", [("authority_limit_pct","local_limit"),("authority_reference","fault_reference")])
                }
                if options.localRule == "retie_overhead_service" {
                    inputs("local.", [("wire_awg","local_awg")])
                    Picker(m.t("local_aerial"), selection:value("local.aerial_cable_certified")) { yesNo }
                    Picker(m.t("local_grade"), selection:value("local.aa8000")) { yesNo }
                }
            }
        }
    }

    @ViewBuilder private var yesNo: some View {
        Text("–").tag("")
        Text(m.t("yes")).tag("yes")
        Text(m.t("no")).tag("no")
    }
}
