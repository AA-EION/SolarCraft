// SPDX-License-Identifier: GPL-3.0-or-later
import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct PhotoPlanSheet: View {
    @EnvironmentObject var m: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var array = 0
    @State private var source: URL?
    @State private var sourceHash = ""
    @State private var original: NSImage?
    @State private var rectified: NSImage?
    @State private var corners: [CGPoint] = []
    @State private var outline: [CGPoint] = []
    @State private var obstacles: [[CGPoint]] = []
    @State private var heights: [Double] = []
    @State private var active = 0
    @State private var width = 10.0
    @State private var depth = 5.0
    @State private var status = ""

    init(initialArray: Int = 0) {
        _array = State(initialValue: initialArray)
    }

    private var arrays: [ProjectDoc.ArrayDoc] { m.project?.arrays ?? [] }
    private var canResume: Bool {
        arrays.indices.contains(array) && (m.project?.roof_photos?.contains(where: { $0.array_id == arrays[array].id }) ?? false)
    }
    private var ready: Bool {
        original != nil && corners.count == 4 && width > 0 && depth > 0
            && (outline.isEmpty || outline.count >= 3) && obstacles.allSatisfy { $0.count >= 3 }
    }
    private var selected: [CGPoint] {
        get {
            if active == 0 { return corners }
            if active == 1 { return outline }
            return obstacles[active - 2]
        }
        nonmutating set {
            if active == 0 { corners = newValue }
            else if active == 1 { outline = newValue }
            else { obstacles[active - 2] = newValue }
            invalidatePreview()
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(m.t("photo_plan")).font(.title2.bold())
            HStack {
                Picker(m.t("photo_array"), selection: $array) {
                    ForEach(arrays.indices, id: \.self) { index in Text(arrays[index].name.value).tag(index) }
                }.onChange(of: array) { _, _ in
                    source = nil; original = nil; reset()
                }
                Button(m.t("photo_open"), action: open)
                if canResume { Button(m.t("photo_resume"), action: resume) }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let original {
                        HStack {
                            Picker("", selection: $active) {
                                Text(m.t("photo_corners")).tag(0)
                                Text(m.t("photo_outline")).tag(1)
                                ForEach(obstacles.indices, id: \.self) { Text("\(m.t("photo_obstacle")) \($0 + 1)").tag($0 + 2) }
                            }.accessibilityLabel(m.t("photo_clear_trace"))
                            if active >= 2 && heights.indices.contains(active - 2) {
                                TextField(m.t("photo_height"), value: $heights[active - 2], format: .number)
                            }
                        }
                        Text(m.t(active == 0 ? "photo_instructions" : "photo_trace")).foregroundStyle(.secondary)
                        photograph(original)
                        HStack {
                            Button(m.t("undo_point")) { selected = Array(selected.dropLast()) }.disabled(selected.isEmpty)
                            Button(m.t("photo_clear_trace")) { selected = [] }.disabled(selected.isEmpty)
                            Spacer()
                            Button(m.t("photo_add_obstacle")) {
                                obstacles.append([]); heights.append(1); active = obstacles.count + 1
                            }.disabled(corners.count != 4 || obstacles.count >= 48)
                            if active >= 2 {
                                Button(m.t("photo_remove_obstacle")) {
                                    let index = active - 2
                                    active = 1; obstacles.remove(at: index); heights.remove(at: index)
                                    invalidatePreview()
                                }
                            }
                        }
                        HStack {
                            NumField(label: m.t("photo_width"), value: $width)
                            NumField(label: m.t("photo_depth"), value: $depth)
                        }
                        if let rectified {
                            Image(nsImage: rectified).resizable().scaledToFit().frame(maxHeight: 220)
                        }
                    }
                    if !status.isEmpty { Text(status).textSelection(.enabled) }
                }
            }
            Divider()
            HStack {
                Button(m.t("photo_preview")) { run(preview: true) }.disabled(!ready)
                Spacer()
                Button(m.t("cancel")) { dismiss() }.keyboardShortcut(.cancelAction)
                Button(m.t("photo_apply")) { run(preview: false) }.disabled(!ready)
                    .buttonStyle(.borderedProminent)
            }
        }.padding(18).frame(width: 710, height: 650)
            .onChange(of: width) { _, _ in invalidatePreview() }
            .onChange(of: depth) { _, _ in invalidatePreview() }
            .onChange(of: heights) { _, _ in invalidatePreview() }
    }

    private func photograph(_ image: NSImage) -> some View {
        let scale = min(650.0 / image.size.width, 275.0 / image.size.height)
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        return Image(nsImage: image).resizable().frame(width: size.width, height: size.height)
            .overlay {
                Canvas { context, _ in
                    func mark(_ points: [CGPoint], _ color: Color, reference: Bool = false) {
                        let mapped = points.map { CGPoint(x: $0.x * size.width, y: $0.y * size.height) }
                        guard let first = mapped.first else { return }
                        var path = Path()
                        path.move(to: first)
                        for point in mapped.dropFirst() { path.addLine(to: point) }
                        if mapped.count >= (reference ? 4 : 3) { path.closeSubpath() }
                        context.stroke(path, with: .color(color), lineWidth: 2)
                        for (i, p) in mapped.enumerated() {
                            let r: CGFloat = reference ? 11 : 4
                            context.fill(Path(ellipseIn: CGRect(x: p.x - r, y: p.y - r, width: r * 2, height: r * 2)), with: .color(color))
                            if reference { context.draw(Text("\(i + 1)").font(.caption.bold()).foregroundColor(.black), at: p) }
                        }
                    }
                    mark(corners, .mint, reference: true)
                    mark(outline, .white)
                    for obstacle in obstacles { mark(obstacle, .yellow) }
                }.allowsHitTesting(false)
            }
            .clipped()
            .gesture(SpatialTapGesture().onEnded { event in
                guard active == 0 || corners.count == 4, selected.count < (active == 0 ? 4 : 128) else { return }
                selected = selected + [CGPoint(x: min(max(event.location.x / size.width, 0), 1),
                                              y: min(max(event.location.y / size.height, 0), 1))]
            })
    }

    private func image(_ response: JSONDict) -> NSImage? {
        guard let png = response["png_base64"] as? String, let data = Data(base64Encoded: png) else { return nil }
        return NSImage(data: data)
    }

    private func invalidatePreview() { rectified = nil; status = "" }

    private func reset() {
        corners = []; outline = []; obstacles = []; heights = []; active = 0
        width = 10; depth = 5; invalidatePreview()
    }

    private func open() {
        let picker = NSOpenPanel()
        picker.allowedContentTypes = [.png, .jpeg]
        guard picker.runModal() == .OK, let url = picker.url else { return }
        do {
            let result = try Core.invokeJSON("photo_source", ["source": url.path])
            reset()
            source = url; sourceHash = str(result, "source_hash"); original = image(result)
        } catch { status = error.localizedDescription }
    }

    private func resume() {
        guard let project = m.url else { return }
        do {
            let result = try Core.invokeJSON("photo_saved", ["file": project.path, "array": array])
            let control = result["control"] as? JSONDict ?? [:]
            func points(_ value: Any?) -> [CGPoint] {
                (value as? [[Double]] ?? []).compactMap { $0.count == 2 ? CGPoint(x: $0[0], y: $0[1]) : nil }
            }
            reset()
            width = num(control, "width_m"); depth = num(control, "depth_m")
            corners = points(control["corners"]); outline = points(result["outline"])
            obstacles = (result["obstacles"] as? [[[Double]]] ?? []).map { points($0) }
            heights = result["obstacle_heights"] as? [Double] ?? []
            while heights.count < obstacles.count { heights.append(1) }
            source = nil; sourceHash = str(result, "source_hash"); original = image(result)
        } catch { status = error.localizedDescription }
    }

    private func run(preview: Bool) {
        guard let project = m.url else { return }
        func coordinates(_ p: [CGPoint]) -> [[Double]] { p.map { [Double($0.x), Double($0.y)] } }
        let control: JSONDict = ["corners": coordinates(corners), "width_m": width, "depth_m": depth]
        var options: JSONDict = ["array": array, "source_hash": sourceHash, "control": control,
                                 "obstacles": obstacles.map(coordinates), "obstacle_heights": heights]
        if !outline.isEmpty { options["outline"] = coordinates(outline) }
        var request: JSONDict = ["file": project.path, "preview": preview, "options": options]
        if let source { request["source"] = source.path }
        do {
            let result = try Core.invokeJSON("photo_layout", request)
            if preview {
                rectified = image(result)
                let count = dicts((result["roof"] as? JSONDict)?["modules"]).count
                status = "\(count) \(m.t("modules_fit"))"
            } else { m.reload(); dismiss() }
        } catch { status = error.localizedDescription }
    }
}
