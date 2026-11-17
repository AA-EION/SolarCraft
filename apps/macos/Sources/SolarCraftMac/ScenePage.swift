// SPDX-License-Identifier: GPL-3.0-or-later
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// 3D scene: the core renders the viewport (z-buffered PNG) and answers picks,
/// so this page and the Windows shell show the same picture. Every change is a
/// scene transaction through the shared JSON API (undoable).
struct ScenePage: View {
    @EnvironmentObject var m: AppModel
    @State private var yaw = 25.0
    @State private var pitch = 35.0
    @State private var zoom = 1.0
    @State private var colorBy = "material"
    @State private var image: NSImage?
    @State private var legend: [(String, Color)] = []
    @State private var scene: JSONDict = [:]
    @State private var hit: JSONDict = [:]
    @State private var dragFrom: (Double, Double)?
    @State private var form = "building"
    @State private var w = 14.0, d = 9.0, eave = 5.5, roofType = "gable", pitchDeg = 25.0, rot = 0.0, px = 0.0, py = 0.0
    @State private var crown = 3.0, treeH = 9.0, transparency = 0.25
    @State private var obsL = 0.8, obsW = 0.8, obsH = 1.5, obsZ = 5.0
    @State private var mps = 8
    @State private var exportFormat = "ifc"
    @State private var report = ""
    private let size = CGSize(width: 900, height: 540)

    private var file: String { m.url?.path ?? "" }
    private var selected: String? { hit["owner"] as? String }
    private var hitId: String { hit["hit"] as? String ?? "" }
    private var arrays: [JSONDict] { dicts(m.raw["arrays"]) }

    private func call(_ method: String, _ params: [String: Any]) -> JSONDict? {
        do { let r = try Core.invokeJSON(method, params); m.error = nil; return r } catch { m.error = error.localizedDescription; return nil }
    }
    private var camera: [String: Any] { ["yaw_deg": yaw, "pitch_deg": pitch, "zoom": zoom, "width": Double(size.width), "height": Double(size.height)] }

    func refresh() {
        guard !file.isEmpty else { return }
        if let s = call("scene_get", ["file": file]) { scene = s }
        var p: [String: Any] = ["file": file, "camera": camera, "color_by": colorBy]
        if let s = selected { p["selected"] = s }
        guard let r = call("scene_image", p), let b64 = r["png_base64"] as? String, let data = Data(base64Encoded: b64) else { return }
        image = NSImage(data: data)
        legend = (r["legend"] as? [[Any]] ?? []).compactMap { e in
            guard e.count == 2, let label = e[0] as? String, let c = e[1] as? [NSNumber], c.count == 3 else { return nil }
            return (label, Color(red: c[0].doubleValue / 255, green: c[1].doubleValue / 255, blue: c[2].doubleValue / 255))
        }
    }

    func edit(_ edits: [[String: Any]]) {
        guard let r = call("scene_edit", ["file": file, "edits": edits]) else { return }
        let issues = dicts(r["issues"]).filter { str($0, "severity") == "error" }
        if !issues.isEmpty { m.notice = issues.map { str($0, "message") }.joined(separator: " · ") }
        m.reload(); refresh()
    }

    func pick(_ p: CGPoint) {
        hit = call("scene_pick", ["file": file, "camera": camera, "x": Double(p.x), "y": Double(p.y)]) ?? [:]
        if hit["hit"] is NSNull { hit = [:] }
        refresh()
    }

    private func rectFootprint() -> [[Double]] {
        let (c, s) = (cos(rot * .pi / 180), sin(rot * .pi / 180))
        return [[0.0, 0.0], [w, 0.0], [w, d], [0.0, d]].map { q in [px + q[0] * c - q[1] * s, py + q[0] * s + q[1] * c] }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            GroupBox(m.t("scene3d")) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Picker(m.t("color_by"), selection: $colorBy) {
                            Text(m.t("material")).tag("material"); Text(m.t("string")).tag("string"); Text(m.t("shading")).tag("shading")
                        }.pickerStyle(.segmented).frame(width: 380).onChange(of: colorBy) { refresh() }
                        Spacer()
                        Button { zoom = max(0.3, zoom / 1.25); refresh() } label: { Image(systemName: "minus.magnifyingglass") }
                        Button { zoom = min(12, zoom * 1.25); refresh() } label: { Image(systemName: "plus.magnifyingglass") }
                        Button { yaw = 25; pitch = 35; zoom = 1; refresh() } label: { Image(systemName: "scope") }
                        Button { m.undo(); refresh() } label: { Label(m.t("undo"), systemImage: "arrow.uturn.backward") }
                    }
                    ZStack(alignment: .topLeading) {
                        Group {
                            if let img = image { Image(nsImage: img).resizable().interpolation(.high) } else { Color(nsColor: .controlBackgroundColor) }
                        }
                        .frame(width: size.width, height: size.height)
                        .contentShape(Rectangle())
                        .gesture(DragGesture(minimumDistance: 3).onChanged { g in
                            let start = dragFrom ?? (yaw, pitch)
                            dragFrom = start
                            yaw = start.0 - Double(g.translation.width) * 0.4
                            pitch = min(89, max(3, start.1 + Double(g.translation.height) * 0.3))
                            refresh()
                        }.onEnded { _ in dragFrom = nil })
                        .onTapGesture(coordinateSpace: .local) { p in pick(p) }
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(legend.indices, id: \.self) { i in HStack(spacing: 4) { Rectangle().fill(legend[i].1).frame(width: 10, height: 10); Text(legend[i].0).font(.caption2) } }
                        }.padding(6)
                    }
                    .border(Color.secondary.opacity(0.3))
                    let c = scene["counts"] as? JSONDict ?? [:]
                    Text("\(int(c, "objects")) objects · \(int(c, "faces")) faces · \(int(c, "fields")) fields · \(int(c, "modules")) modules · \(int(c, "strings")) strings").font(.caption).foregroundStyle(.secondary)
                    IssueList(issues: dicts(scene["issues"]))
                }
            }
            HStack(alignment: .top, spacing: 14) {
                GroupBox(m.t("selection")) { inspector.frame(width: 400, alignment: .topLeading) }
                GroupBox(m.t("add_building")) { adder.frame(width: 440, alignment: .topLeading) }
            }
            GroupBox(m.t("fidelity")) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Button(m.t("import_model")) { importModel() }
                        Picker("", selection: $exportFormat) { ForEach(["ifc", "glb", "obj", "dxf", "igz", "gltf", "json"], id: \.self) { Text($0.uppercased()).tag($0) } }.frame(width: 110)
                        Button(m.t("export_model")) { exportModel() }
                    }
                    if !report.isEmpty { Text(report).font(.caption.monospaced()).textSelection(.enabled) }
                }
            }
        }
        .onAppear(perform: refresh)
    }

    @ViewBuilder private var inspector: some View {
        VStack(alignment: .leading, spacing: 6) {
            if hit.isEmpty { Text("Click the model to select a roof face, object or module.").foregroundStyle(.secondary) } else {
                Text(hitId).font(.headline)
                if let o = hit["object"] as? JSONDict {
                    Text("\(str(o, "name")) · \(str((o["kind"] as? JSONDict) ?? [:], "type")) · \(str(o, "layer"))").font(.caption)
                    HStack {
                        ForEach([("←", -0.5, 0.0), ("→", 0.5, 0.0), ("↓", 0.0, -0.5), ("↑", 0.0, 0.5)], id: \.0) { a in
                            Button(a.0) { edit([["op": "move", "id": str(o, "id"), "dx": a.1, "dy": a.2]]) }
                        }
                        Button(role: .destructive) { edit([["op": "remove_object", "id": str(o, "id")]]); hit = [:] } label: { Label(m.t("delete_selected"), systemImage: "trash") }
                    }
                    if hitId.contains("/roof-") {
                        Button(m.t("place_modules")) {
                            guard let r = call("scene_edit", ["file": file, "edits": [["op": "add_field", "name": hitId, "mount": ["type": "roof", "face": hitId]]]]),
                                  let id = (r["touched"] as? [String])?.first else { return }
                            _ = call("scene_bind", ["file": file, "field": id, "array_id": str(arrays.first ?? [:], "id"), "match_array": false])
                            edit([["op": "place_modules", "field": id]])
                        }
                    }
                }
                if let f = hit["field"] as? JSONDict {
                    Text("\(str(f, "name")) · \(int(f, "modules")) modules · \(int(f, "strings")) strings · array \(str(f, "array_id"))").font(.caption)
                    Stepper("\(m.t("modules_per_string")): \(mps)", value: $mps, in: 1...40)
                    HStack {
                        Button(m.t("route_strings")) { edit([["op": "route_strings", "field": str(f, "id"), "modules_per_string": mps]]) }
                        Button(m.t("fit_to_array")) {
                            _ = call("scene_bind", ["file": file, "field": str(f, "id"), "array_id": str(arrays.first ?? [:], "id"), "match_array": true])
                            m.reload(); refresh()
                        }
                        Button(role: .destructive) { edit([["op": "remove_field", "id": str(f, "id")]]); hit = [:] } label: { Image(systemName: "trash") }
                    }
                    if let mod = hit["module"] as? JSONDict, hitId.contains("/m") {
                        Button("Remove module \(hitId.split(separator: "/").last ?? "")") { edit([["op": "remove_modules", "field": str(f, "id"), "modules": [str(mod, "id")]]]) }
                    }
                }
            }
        }
    }

    @ViewBuilder private var adder: some View {
        VStack(alignment: .leading, spacing: 6) {
            Picker("", selection: $form) { Text(m.t("add_building")).tag("building"); Text(m.t("add_tree")).tag("tree"); Text(m.t("add_obstacle")).tag("block") }.pickerStyle(.segmented)
            HStack { NumField(label: "x", value: $px); NumField(label: "y", value: $py) }
            switch form {
            case "building":
                HStack { NumField(label: "W", value: $w); NumField(label: "D", value: $d); NumField(label: m.t("eave_height"), value: $eave) }
                HStack {
                    Picker(m.t("roof_type"), selection: $roofType) { ForEach(["flat", "shed", "gable", "hip"], id: \.self) { Text($0).tag($0) } }.frame(width: 200)
                    NumField(label: m.t("pitch"), value: $pitchDeg)
                }
                NumField(label: m.t("rotation"), value: $rot)
                Button(m.t("add_building")) {
                    var roof: [String: Any] = ["type": roofType]
                    if roofType != "flat" { roof["pitch_deg"] = pitchDeg }
                    if roofType == "shed" { roof["facing_deg"] = 180.0 + rot }
                    edit([["op": "add_object", "name": "Building", "layer": "building", "kind": ["type": "building", "footprint": rectFootprint(), "eave_height": eave, "roof": roof]]])
                }
            case "tree":
                HStack { NumField(label: "R", value: $crown); NumField(label: "H", value: $treeH); NumField(label: m.t("transparency"), value: $transparency) }
                Button(m.t("add_tree")) {
                    edit([["op": "add_object", "name": "Tree", "layer": "vegetation", "kind": ["type": "tree", "center": [px, py], "trunk_height": treeH * 0.35, "trunk_radius": 0.25, "crown_radius": crown, "crown_height": treeH * 0.65, "transparency": transparency]]])
                }
            default:
                HStack { NumField(label: "L", value: $obsL); NumField(label: "W", value: $obsW); NumField(label: "H", value: $obsH); NumField(label: "z", value: $obsZ) }
                Button(m.t("add_obstacle")) {
                    edit([["op": "add_object", "name": "Obstacle", "layer": "obstacles", "kind": ["type": "block", "center": [px, py], "base_z": obsZ, "length": obsL, "width": obsW, "height": obsH, "rotation_deg": rot]]])
                }
            }
        }
    }

    private func describe(_ r: JSONDict) -> String {
        let f = r["fidelity"] as? JSONDict ?? [:]
        var lines = ["\(str(f, "format").uppercased()) \(str(f, "direction")): \(int(f, "objects")) objects, \(int(f, "fields")) fields, \(int(f, "modules")) modules, \(int(f, "triangles")) triangles",
                     "axes: \(str(f, "axes"))", "georeference: \(str(f, "georeference"))",
                     "max coordinate error: \(String(format: "%.2e", num(f, "max_coordinate_error_m"))) m"]
        lines += (f["conversions"] as? [String] ?? []).map { "converted: \($0)" }
        lines += (f["unsupported"] as? [String] ?? []).map { "not carried: \($0)" }
        lines += (r["notes"] as? [String] ?? []).map { "note: \($0)" }
        return lines.joined(separator: "\n")
    }

    private func importModel() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = ["glb", "gltf", "obj", "dxf", "ifc", "igz", "json"].compactMap { UTType(filenameExtension: $0) }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if let r = call("scene_import", ["file": file, "source": url.path, "options": ["relink": true]]) { report = describe(r); m.reload(); refresh() }
    }

    private func exportModel() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(m.url?.deletingPathExtension().lastPathComponent ?? "scene").\(exportFormat)"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        if let r = call("scene_export", ["file": file, "output": url.path, "format": exportFormat]) { report = describe(r) }
    }
}
