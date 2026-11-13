// SPDX-License-Identifier: GPL-3.0-or-later
import Foundation

/// A project value with its provenance (user-entered or calculated).
struct Field<T: Decodable>: Decodable {
    let value: T
    let origin: String
    var isUser: Bool { origin == "user" }
}

struct ProjectDoc: Decodable {
    struct Customer: Decodable { let name: String; let address: String }
    struct Site: Decodable { let latitude: Field<Double>; let longitude: Field<Double>; let elevation: Field<Double>; let albedo: Field<Double> }
    struct ArrayDoc: Decodable {
        let id: String
        let tilt: Field<Double>
        let azimuth: Field<Double>
        let target_kwp: Field<Double>
        let module_id: Field<String>
        let modules_per_string: Field<Int>
        let strings: Field<Int>
        let inverter_id: Field<String>
        let inverter_count: Field<Int>
        let roof: RoofDoc?
    }
    struct RoofDoc: Decodable {
        struct Face: Decodable { let outline: [[Double]]; let obstacles: [[[Double]]] }
        struct Params: Decodable { let setback: Double; let obstacle_clearance: Double }
        struct ModuleRect: Decodable { let x0: Double; let y0: Double; let x1: Double; let y1: Double }
        let face: Face
        let params: Params
        let modules: [ModuleRect]
    }
    struct Simulation: Decodable { let mounting: Field<String> }
    struct WeatherRef: Decodable { let provider: String; let summary: String }
    struct Module: Decodable { let id: String; let manufacturer: String; let model: String }
    struct Inverter: Decodable { let id: String; let manufacturer: String; let model: String }
    struct Library: Decodable { let modules: [Module]; let inverters: [Inverter] }

    let name: Field<String>
    let customer: Customer
    let mode: String
    let site: Site
    let arrays: [ArrayDoc]
    let simulation: Simulation
    let weather: WeatherRef?
    let library: Library
}

struct Issue: Decodable, Identifiable {
    let code: String
    let severity: String
    let pointer: String
    let message: String
    var id: String { code + pointer }
}
struct IssuesResult: Decodable { let issues: [Issue] }
struct SetResult: Decodable { let project: ProjectDoc; let issues: [Issue] }

struct Annual: Decodable { let dc_kwp: Double; let ac_kw: Double; let ac_kwh: Double; let specific_yield_kwh_kwp: Double; let performance_ratio: Double; let ghi_kwh_m2: Double; let poa_kwh_m2: Double }
struct Step: Decodable, Identifiable { let key: String; let label: String; let value: Double; let delta: Double; let delta_pct: Double; var id: String { key } }
struct Month: Decodable, Identifiable { let month: Int; let ac_kwh: Double; var id: Int { month } }
struct Explain: Decodable { let annual: Annual; let energy_kwh: [Step]; let monthly: [Month] }
struct WeatherSummary: Decodable { let weather_ref: ProjectDoc.WeatherRef }
struct ReportOutput: Decodable { let path: String; let bytes: Int }

struct RoofResult: Decodable { let modules_fit: Int? }
