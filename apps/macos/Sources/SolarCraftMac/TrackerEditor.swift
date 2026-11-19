// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct TrackerEditor: View {
    @EnvironmentObject var m: AppModel
    let pointer: String
    let settings: JSONDict
    let facing: Double
    @State private var draft = Parameters()

    private struct Parameters: Equatable {
        var kind = "fixed"
        var axis = 180.0
        var axisTilt = 0.0
        var rotation = 60.0
        var center = 180.0
        var travel = 360.0
        var maxTilt = 90.0
        var backtracking = false
        var gcr = 0.4
    }

    private var saved: Parameters {
        Parameters(kind: settings["kind"] as? String ?? "fixed",
                   axis: num(settings, "axis_azimuth_deg", facing), axisTilt: num(settings, "axis_tilt_deg"),
                   rotation: num(settings, "max_rotation_deg", 60), center: num(settings, "center_azimuth_deg", facing),
                   travel: num(settings, "azimuth_travel_deg", 360), maxTilt: num(settings, "max_tilt_deg", 90),
                   backtracking: settings["backtracking_gcr"] is NSNumber, gcr: num(settings, "backtracking_gcr", 0.4))
    }

    var body: some View {
        Picker(m.t("tracker_mode"), selection: $draft.kind) {
            Text(m.t("tracker_fixed")).tag("fixed")
            Text(m.t("tracker_single")).tag("single_axis")
            Text(m.t("tracker_vertical")).tag("vertical_axis")
            Text(m.t("tracker_dual")).tag("dual_axis")
        }
        if draft.kind == "single_axis" {
            NumField(label: m.t("tracker_axis_azimuth"), value: $draft.axis)
            NumField(label: m.t("tracker_axis_tilt"), value: $draft.axisTilt)
            NumField(label: m.t("tracker_rotation"), value: $draft.rotation)
            Toggle(m.t("tracker_backtrack"), isOn: $draft.backtracking)
            if draft.backtracking {
                NumField(label: m.t("tracker_gcr"), value: $draft.gcr)
            }
        }
        if draft.kind == "vertical_axis" || draft.kind == "dual_axis" {
            NumField(label: m.t("tracker_center"), value: $draft.center)
            NumField(label: m.t("tracker_travel"), value: $draft.travel)
        }
        if draft.kind == "dual_axis" {
            NumField(label: m.t("tracker_max_tilt"), value: $draft.maxTilt)
        }
        Button(m.t("tracker_apply"), action: apply)
            .onAppear { draft = saved }
            .onChange(of: saved) { draft = saved }
    }

    private func apply() {
        var value: JSONDict = ["kind": draft.kind]
        switch draft.kind {
        case "single_axis":
            value["axis_azimuth_deg"] = draft.axis
            value["axis_tilt_deg"] = draft.axisTilt
            value["max_rotation_deg"] = draft.rotation
            if draft.backtracking { value["backtracking_gcr"] = draft.gcr }
        case "vertical_axis", "dual_axis":
            value["center_azimuth_deg"] = draft.center
            value["azimuth_travel_deg"] = draft.travel
            if draft.kind == "dual_axis" { value["max_tilt_deg"] = draft.maxTilt }
        default:
            m.set(pointer, NSNull())
            return
        }
        m.set(pointer, value)
    }
}
