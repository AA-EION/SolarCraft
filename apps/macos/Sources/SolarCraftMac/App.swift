// SPDX-License-Identifier: GPL-3.0-or-later
import Charts
import CSolarCraft
import SwiftUI

@main
enum Entry {
    static func main() {
        if CommandLine.arguments.contains("--selftest") { exit(SelfTest.run()) }
        SolarCraftApp.main()
    }
}

struct SolarCraftApp: App {
    var body: some Scene {
        WindowGroup("SolarCraft") { QuickDesignView().frame(minWidth: 720, minHeight: 480) }
    }
}

@MainActor
final class QuickDesignModel: ObservableObject {
    @Published var latitude = 4.711, longitude = -74.072, elevation = 2640.0
    @Published var tilt = 10.0, azimuth = 180.0, modules = 10.0
    @Published var progress = 0.0
    @Published var running = false
    @Published var summary: ScAnnualSummary?
    @Published var monthly: [(Int, Double)] = []
    @Published var error: String?
    private var job: Core.Job?

    func run() {
        let input = ScQuickInput(latitude: latitude, longitude: longitude, elevation_m: elevation, tilt_deg: tilt, azimuth_deg: azimuth, modules: UInt32(modules), year: 2026)
        let job = Core.Job(onProgress: { f in Task { @MainActor in self.progress = f } })
        self.job = job; running = true; error = nil
        Task.detached {
            let result = Result { try Core.quickAnnual(input, job: job) }
            await MainActor.run {
                self.running = false
                switch result {
                case .success(let r):
                    self.summary = r.summary
                    let days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
                    var h = 0
                    self.monthly = days.enumerated().map { (m, d) in
                        let kwh = r.hourly[h..<min(h + d * 24, r.hourly.count)].reduce(0, +) / 1000; h += d * 24; return (m + 1, kwh)
                    }
                case .failure(let e): self.error = "\(e)"
                }
            }
        }
    }
    func cancel() { job?.cancel() }
}

struct QuickDesignView: View {
    @StateObject private var m = QuickDesignModel()
    var body: some View {
        HSplitView {
            Form {
                Section("Site") {
                    TextField("Latitude", value: $m.latitude, format: .number)
                    TextField("Longitude", value: $m.longitude, format: .number)
                    TextField("Elevation (m)", value: $m.elevation, format: .number)
                }
                Section("Array") {
                    TextField("Tilt (°)", value: $m.tilt, format: .number)
                    TextField("Azimuth (° from north)", value: $m.azimuth, format: .number)
                    Stepper("Modules: \(Int(m.modules))", value: $m.modules, in: 1...60)
                }
                HStack {
                    Button("Simulate", action: m.run).disabled(m.running).keyboardShortcut(.return)
                    if m.running { ProgressView(value: m.progress).frame(width: 120); Button("Cancel", action: m.cancel) }
                }
                Text("Clear-sky spike — not a yield estimate.").font(.footnote).foregroundStyle(.secondary)
            }
            .padding().frame(minWidth: 260)
            VStack(alignment: .leading) {
                if let s = m.summary {
                    Text(String(format: "%.0f kWh/yr · %.0f kWh/kWp · %.2f kWp", s.annual_ac_kwh, s.specific_yield_kwh_per_kwp, s.dc_kwp)).font(.title3)
                    Chart(m.monthly, id: \.0) { BarMark(x: .value("Month", $0.0), y: .value("kWh", $0.1)) }
                        .accessibilityLabel("Monthly AC energy")
                }
                if let e = m.error { Text(e).foregroundStyle(.red) }
                Spacer()
                Text("Core \(Core.version)").font(.caption).foregroundStyle(.secondary)
            }.padding()
        }
    }
}
