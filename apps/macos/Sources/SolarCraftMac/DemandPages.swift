// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct InputSpec: Identifiable {
    let id: String
    let label: String
    let preset: Double
    let whole: Bool

    init(_ id: String, _ label: String, _ preset: Double, whole: Bool = false) {
        self.id = id
        self.label = label
        self.preset = preset
        self.whole = whole
    }

    func field(_ values: Binding<[String: Double]>) -> Binding<Double> {
        Binding(get: { values.wrappedValue[id] ?? preset }, set: { values.wrappedValue[id] = $0 })
    }

    func parsed(_ values: [String: Double]) -> Any? {
        let v = values[id] ?? preset
        guard v.isFinite else { return nil }
        if whole {
            guard v >= 0, v <= Double(Int32.max), v == v.rounded() else { return nil }
            return Int(v)
        }
        return v
    }
}

func payload(_ specs: [InputSpec], _ values: [String: Double]) -> JSONDict? {
    var result: JSONDict = [:]
    for spec in specs {
        guard let value = spec.parsed(values) else { return nil }
        result[spec.id] = value
    }
    return result
}

struct DemandEditorPage: View {
    let section: String
    @EnvironmentObject var m: AppModel
    @State private var enabled = false
    @State private var strategy = "immediate"
    @State private var fields = [String: Double]()
    private var isEv: Bool { section == "ev" }
    private var required: Bool { m.systemType.hasPrefix(isEv ? "grid_ev" : "grid_thermal") }
    private var specs: [InputSpec] {
        if isEv {
            return [.init("daily_kwh", "ev_daily", 12), .init("charger_kw", "ev_charger", 7.2),
                .init("efficiency", "ev_efficiency", 0.9), .init("arrival_hour", "ev_arrival", 18, whole: true),
                .init("departure_hour", "ev_departure", 7, whole: true)]
        }
        return [.init("setpoint_c", "heat_setpoint", 20), .init("heat_loss_kw_per_k", "heat_loss", 0.18),
            .init("thermal_kw", "heat_capacity", 6), .init("cutoff_c", "heat_cutoff", -15),
            .init("cop_at_0c", "heat_cop0", 2.5), .init("cop_at_10c", "heat_cop10", 3.8),
            .init("resistance_backup_kw", "heat_backup", 3)]
    }

    var body: some View {
        Form {
            if isEv { FleetControls() }
            if !isEv || m.section("fleet").isEmpty {
                Toggle(m.t(isEv ? "ev_page" : "heat_page"), isOn: $enabled).disabled(required)
                if enabled {
                    ForEach(specs) { spec in NumField(label: m.t(spec.label), value: spec.field($fields)) }
                    if isEv {
                        Picker(m.t("ev_strategy"), selection: $strategy) {
                            Text(m.t("ev_immediate")).tag("immediate")
                            Text(m.t("ev_solar")).tag("solar_preferred")
                        }
                        Text(m.t("ev_window")).foregroundStyle(.secondary)
                    }
                }
                Button(m.t("apply")) {
                    guard enabled else { m.setSection(section, NSNull()); return }
                    guard var value = payload(specs, fields) else { m.error = m.t("issues"); return }
                    if isEv { value["strategy"] = strategy }
                    m.setSection(section, value)
                }.buttonStyle(.borderedProminent)
                if let result = m.summary?[section] as? JSONDict {
                    let electricity = isEv ? num(result, "electrical_kwh") : num(result, "pump_electric_kwh") + num(result, "backup_kwh")
                    let missing = num(result, isEv ? "unscheduled_kwh" : "unmet_heat_kwh")
                    LabeledContent(m.t("annual_kwh")) { Text(fmt(electricity, 1)) }
                    LabeledContent(m.t("island_unserved")) { Text("\(fmt(missing, 1)) kWh") }
                }
            }
            if !isEv { ThermalStoreControls() }
        }
        .onAppear {
            let current = m.section(section)
            enabled = required || !current.isEmpty
            fields = Dictionary(uniqueKeysWithValues: specs.map { ($0.id, num(current, $0.id, $0.preset)) })
            strategy = current["strategy"] as? String ?? "immediate"
        }
    }
}

struct IslandControlsPage: View {
    @EnvironmentObject var m: AppModel
    @State private var plan = [String: Double]()
    @State private var costs = [String: Double]()
    @State private var currency = "EUR"
    @State private var charge = false
    private let settings: [InputSpec] = [
        .init("minimum_generator_pct", "island_min", 25), .init("minimum_run_hours", "island_run", 1, whole: true),
        .init("start_soc_pct", "island_start", 25), .init("stop_soc_pct", "island_stop", 75),
        .init("fuel_price", "island_fuel_price", 0), .init("fuel_escalation_pct", "island_fuel_rise", 0),
        .init("generator_investment", "island_gen_cost", 0),
    ]
    private let economics: [InputSpec] = [
        .init("horizon_years", "horizon", 20, whole: true), .init("discount_rate_pct", "discount_rate", 4),
        .init("cost_fixed", "cost_fixed", 1500), .init("cost_per_kwp", "cost_per_kwp", 1100),
        .init("cost_per_battery_kwh", "cost_per_kwh", 450), .init("om_pct_of_investment", "om", 1),
        .init("om_escalation_pct", "island_maintenance_escalation", 2),
    ]
    private let replacements: [InputSpec] = [
        .init("inverter_replacement_year", "island_inverter_year", 13, whole: true),
        .init("inverter_replacement_cost", "island_inverter_cost", 0),
        .init("battery_replacement_year", "island_battery_year", 12, whole: true),
        .init("battery_replacement_cost", "island_battery_cost", 0), .init("subsidy", "island_subsidy", 0),
    ]
    var body: some View {
        Form {
            IslandClusterControls()
            if m.systemType == "off_grid_generator" && dicts(m.raw["island_clusters"]).isEmpty {
                Section(m.t("generator")) {
                    Toggle(m.t("island_charge"), isOn: $charge)
                    ForEach(settings) { s in NumField(label: m.t(s.label), value: s.field($plan)) }
                }
            }
            Section(m.t("economics")) {
                Text(m.t("island_cost_hint")).foregroundStyle(.secondary)
                TextField(m.t("currency"), text: $currency)
                ForEach(economics) { s in NumField(label: m.t(s.label), value: s.field($costs)) }
                DisclosureGroup(m.t("island_replacements")) {
                    ForEach(replacements) { s in NumField(label: m.t(s.label), value: s.field($costs)) }
                }
                Button(m.t("apply")) {
                    guard var data = payload(settings, plan), let expense = payload(economics + replacements, costs) else { m.error = m.t("issues"); return }
                    data["charge_battery"] = charge
                    m.saveIsland(plan: data, costs: expense, currency: currency)
                }.buttonStyle(.borderedProminent)
                if let finance = m.summary?["island_finance"] as? JSONDict {
                    LabeledContent(m.t("island_lifetime")) { Text("\(fmt(num(finance, "discounted_cost"), 2)) \(currency)") }
                    if finance["cost_per_served_kwh"] is NSNumber {
                        LabeledContent(m.t("island_unit_cost")) { Text("\(fmt(num(finance, "cost_per_served_kwh"), 3)) \(currency)/kWh") }
                    }
                    SimpleTable(header: [m.t("year"), m.t("island_fuel_cost"), m.t("island_year_cost")],
                        rows: dicts(finance["years"]).map { y in [fmt(num(y, "year")), fmt(num(y, "fuel_cost"), 2), fmt(num(y, "discounted_cost"), 2)] })
                }
            }
        }
        .onAppear {
            let input = m.section("island_plan")
            let existing = m.section("economics")
            plan = Dictionary(uniqueKeysWithValues: settings.map { ($0.id, num(input, $0.id, $0.preset)) })
            costs = Dictionary(uniqueKeysWithValues: (economics + replacements).map { ($0.id, num(existing, $0.id, $0.preset)) })
            charge = input["charge_battery"] as? Bool ?? false
            currency = m.section("tariff")["currency"] as? String ?? "EUR"
        }
    }
}
