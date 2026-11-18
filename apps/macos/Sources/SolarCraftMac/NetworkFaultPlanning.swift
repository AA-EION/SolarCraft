// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation
import SwiftUI

struct NetworkFaultPlanner: View {
    @EnvironmentObject var m: AppModel
    @State private var study: JSONDict = [:]
    @State private var preview: JSONDict = [:]
    @State private var projectFile: String?
    @State private var editor = false
    @State private var draft = ""
    @State private var detail: String?

    var body: some View {
        DisclosureGroup(m.t("network_faults")) {
            VStack(alignment: .leading, spacing: 8) {
                Text(m.t("network_faults_hint")).foregroundStyle(.secondary)
                Button(m.t("network_fault_edit")) {
                    draft = encode(study["studies"])
                    preview = [:]
                    detail = nil
                    editor = true
                }.disabled(projectFile == nil)
                results(study)
            }.padding(.vertical, 6)
        }
        .task(id: m.url) {
            study = [:]
            projectFile = m.url?.path
            guard let projectFile else { return }
            do { study = try Core.invokeJSON("network_faults", ["file": projectFile]) }
            catch { m.error = error.localizedDescription }
        }
        .sheet(isPresented: $editor) {
            VStack(alignment: .leading, spacing: 12) {
                Text(m.t("network_faults")).font(.headline)
                Text(m.t("network_fault_json"))
                TextEditor(text: $draft).font(.system(.body, design: .monospaced)).frame(minHeight: 200, maxHeight: 280)
                HStack {
                    Button(m.t("network_fault_example")) {
                        draft = encode(study["example"])
                        preview = [:]
                        detail = nil
                    }
                    Button(m.t("network_fault_mesh_example")) {
                        draft = encode(study["mesh_example"])
                        preview = [:]
                        detail = nil
                    }
                    Button(m.t("network_fault_preview")) { evaluate(save: false) }
                    Spacer()
                    Button(m.t("cancel")) { editor = false }
                    Button(m.t("network_fault_save")) { evaluate(save: true) }.buttonStyle(.borderedProminent)
                }
                if let detail { Text(detail).foregroundStyle(.red) }
                if !dicts(preview["results"]).isEmpty {
                    ScrollView { results(preview) }
                }
            }.padding().frame(minWidth: 540, maxHeight: 640)
        }
    }

    private func encode(_ value: Any?) -> String {
        guard let value, let data = try? JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]) else { return "[]" }
        return String(decoding: data, as: UTF8.self)
    }

    private func evaluate(save: Bool) {
        guard let projectFile, m.url?.path == projectFile else { return }
        preview = [:]
        do {
            let value = try JSONSerialization.jsonObject(with: Data(draft.utf8))
            guard let studies = value as? [JSONDict] else { detail = m.t("network_fault_json"); return }
            let result = try Core.invokeJSON("network_faults", ["file": projectFile, "studies": studies, "mode": save ? "save" : "preview"])
            detail = nil
            if save { study = result; editor = false }
            else { preview = result }
        } catch { detail = error.localizedDescription }
    }

    private func status(_ row: JSONDict) -> String { m.t("code_" + str(row, "status")) }
    private func number(_ row: JSONDict, _ key: String) -> String {
        (row[key] as? NSNumber).map { fmt($0.doubleValue, 3) } ?? "–"
    }

    private func results(_ state: JSONDict) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            let studies = dicts(state["results"])
            ForEach(studies.indices, id: \.self) { index in
                let study = studies[index]
                Text(str(study,"name") + " · " + status(study)).font(.headline)
                let faults = dicts(study["faults"])
                ForEach(faults.indices, id: \.self) { index in
                    let fault = faults[index]
                    DisclosureGroup(str(fault,"bus") + " · " + status(fault) + " · " + m.t("network_fault_min_max")
                        + " " + number(fault,"minimum_ka") + " / " + number(fault,"maximum_ka")) {
                        details(fault)
                    }
                }
            }
        }
    }

    private func details(_ fault: JSONDict) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            let outside = fault["outside_source_envelope"] as? [String] ?? []
            let unprotected = fault["source_paths_without_device"] as? [String] ?? []
            ForEach(outside, id: \.self) { Text(m.t("network_fault_outside") + " " + $0) }
            ForEach(unprotected, id: \.self) { Text(m.t("network_fault_no_device") + " " + $0) }
            if fault["staged_clearing_required"] as? Bool == true { Text(m.t("network_fault_staged")) }
            if let sequence = fault["post_trip"] as? JSONDict {
                let stages = dicts(sequence["stages"])
                Text(m.t("network_fault_sequence") + " · " + status(sequence)).font(.headline)
                ForEach(stages.indices, id: \.self) { index in
                    let stage = stages[index]
                    let opened = (stage["open_links"] as? [String] ?? []).joined(separator: ", ")
                    DisclosureGroup(number(stage,"elapsed_seconds") + " s · " + opened + " · " + status(stage)) {
                        Text(m.t("network_fault_remaining") + " " + number(stage,"remaining_maximum_ka"))
                        let devices = dicts(stage["devices"])
                        ForEach(devices.indices, id: \.self) { i in
                            let device = devices[i]
                            Text(str(device,"link") + " · " + status(device) + " · " + number(device,"maximum_ka") + " kA")
                        }
                    }
                }
            }
            let sources = dicts(fault["sources"])
            ForEach(sources.indices, id: \.self) { index in
                let source = sources[index]
                Text(m.t("network_fault_source") + " " + str(source,"name") + " · " + m.t("network_fault_min_max")
                    + " " + number(source,"minimum_state_ka") + " / " + number(source,"maximum_state_ka"))
            }
            let devices = dicts(fault["devices"])
            ForEach(devices.indices, id: \.self) { index in
                let device = devices[index]
                Text(str(device,"link") + " · " + str(device,"model") + " · " + status(device)
                    + " · " + m.t("network_fault_peak") + " " + number(device,"peak_ka"))
                SimpleTable(header: ["code_protection", "network_fault_status", "code_actual", "code_threshold"].map { m.t($0) },
                    rows: dicts(device["checks"]).map { [m.t("check_" + str($0,"id")), status($0), number($0,"actual"), number($0,"limit")] })
            }
            let pairs = dicts(fault["coordination"])
            ForEach(pairs.indices, id: \.self) { index in
                let pair = pairs[index]
                Text(str(pair,"backup_link") + " → " + str(pair,"primary_link") + " · " + status(pair)
                    + " · " + m.t("network_fault_ratio") + " " + number(pair,"time_ratio"))
            }
        }
    }
}
