// SPDX-License-Identifier: GPL-3.0-or-later
import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct PhotoPlanSheet: View {
    @EnvironmentObject var m: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var source: URL?
    @State private var sourceHash = ""
    @State private var original: NSImage?
    @State private var rectified: NSImage?
    @State private var points: [CGPoint] = []
    @State private var width = 10.0
    @State private var depth = 5.0
    @State private var status = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Button(m.t("photo_plan"), action: open)
                Text(m.t("photo_instructions")).foregroundStyle(.secondary)
                if let original { photo(original) }
                HStack {
                    TextField(m.t("photo_width"), value: $width, format: .number)
                    TextField(m.t("photo_depth"), value: $depth, format: .number)
                }
                if let rectified {
                    Image(nsImage: rectified).resizable().scaledToFit().frame(maxHeight: 160)
                }
                if !status.isEmpty { Text(status) }
                HStack {
                    Button(m.t("photo_reset")) { points.removeAll(); rectified = nil; status = "" }
                    Button(m.t("photo_preview")) { run(preview: true) }.disabled(points.count != 4)
                    Spacer()
                    Button(m.t("cancel")) { dismiss() }
                    Button(m.t("photo_apply")) { run(preview: false) }.disabled(points.count != 4)
                }
            }.padding(18)
        }.frame(width: 600, height: 610)
    }

    private func photo(_ image: NSImage) -> some View {
        let scale = min(550.0 / image.size.width, 280.0 / image.size.height)
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        return Image(nsImage: image).resizable().frame(width: size.width, height: size.height)
            .overlay(alignment: .topLeading) {
                ForEach(points.indices, id: \.self) { index in
                    Text("\(index + 1)").font(.caption.bold()).foregroundStyle(.white)
                        .frame(width: 22, height: 22).background(.teal, in: Circle())
                        .position(x: points[index].x * size.width, y: points[index].y * size.height)
                }
            }
            .gesture(SpatialTapGesture().onEnded { event in
                guard points.count < 4 else { return }
                points.append(CGPoint(x: min(max(event.location.x / size.width, 0), 1),
                                      y: min(max(event.location.y / size.height, 0), 1)))
            })
    }

    private func image(_ response: JSONDict) -> NSImage? {
        guard let png = response["png_base64"] as? String, let data = Data(base64Encoded: png) else { return nil }
        return NSImage(data: data)
    }

    private func open() {
        let picker = NSOpenPanel()
        picker.allowedContentTypes = [.png, .jpeg]
        guard picker.runModal() == .OK, let url = picker.url else { return }
        do {
            let result = try Core.invokeJSON("photo_source", ["source": url.path])
            source = url; sourceHash = result["source_hash"] as? String ?? ""
            original = image(result); rectified = nil; points.removeAll(); status = ""
        } catch { status = error.localizedDescription }
    }

    private func run(preview: Bool) {
        guard let source, let project = m.url else { return }
        let control: JSONDict = ["corners": points.map { [Double($0.x), Double($0.y)] }, "width_m": width, "depth_m": depth]
        let options: JSONDict = ["source_hash": sourceHash, "control": control]
        do {
            let result = try Core.invokeJSON("photo_layout", ["file": project.path, "source": source.path, "preview": preview, "options": options])
            if preview {
                rectified = image(result)
                let count = ((result["roof"] as? JSONDict)?["modules"] as? [JSONDict])?.count ?? 0
                status = "\(count) \(m.t("modules_fit"))"
            } else { m.reload(); dismiss() }
        } catch { status = error.localizedDescription }
    }
}
