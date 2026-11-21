// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

private struct RecordEdit: Identifiable {
    let id = UUID()
    let index: Int?
    let record: JSONDict
}

private func numberInputs(_ specs: [InputSpec], _ value: JSONDict) -> [String: Double] {
    Dictionary(uniqueKeysWithValues: specs.map { ($0.id, num(value, $0.id, $0.preset)) })
}

struct FleetControls: View {
    @EnvironmentObject var m: AppModel
    @State private var edit: RecordEdit?
    @State private var charge = 7.2
    @State private var discharge = 0.0
    @State private var export = false
    @State private var exportFields: [String: Double] = [:]
    @State private var exportDays = Set(0..<7)
    private let exportSpecs: [InputSpec] = [
        .init("limit_kw", "v2g_limit", 2), .init("start_hour", "v2g_start", 18, whole: true), .init("end_hour", "v2g_end", 22, whole: true),
    ]
    private var groups: [JSONDict] { dicts(m.section("fleet")["groups"]) }

    var body: some View {
        Section(m.t("fleet")) {
            Text(m.t("fleet_hint")).foregroundStyle(.secondary)
            NumField(label: m.t("fleet_charge"), value: $charge)
            NumField(label: m.t("fleet_discharge"), value: $discharge)
            DisclosureGroup(m.t("v2g")) {
                Toggle(m.t("v2g"), isOn: $export)
                if export {
                    Text(m.t("v2g_hint")).foregroundStyle(.secondary)
                    ForEach(exportSpecs) { field in NumField(label:m.t(field.label), value:field.field($exportFields)) }
                    LabeledContent(m.t("v2g_days")) {
                        HStack {
                            ForEach(0..<7) { i in
                                Toggle(Calendar.current.shortWeekdaySymbols[(i + 1) % 7], isOn: Binding(
                                    get: { exportDays.contains(i) }, set: { on in
                                        if on { exportDays.insert(i) } else { exportDays.remove(i) }
                                    }
                                )).toggleStyle(.checkbox)
                            }
                        }
                    }
                }
            }
            ForEach(groups.indices, id: \.self) { i in
                HStack {
                    Text(str(groups[i], "name"))
                    Spacer()
                    Button(m.t("advanced_edit")) { edit = RecordEdit(index: i, record: groups[i]) }
                    Button(m.t("remove")) { var next = groups; next.remove(at: i); save(next) }
                }
            }
            HStack {
                Button(m.t("advanced_add")) { edit = RecordEdit(index: nil, record: [:]) }
                if !groups.isEmpty { Button(m.t("apply")) { save(groups) }.buttonStyle(.borderedProminent) }
            }
            if let result = m.summary?["fleet"] as? JSONDict {
                if export { LabeledContent(m.t("v2g")) { Text("\(fmt(num(result,"v2g_kwh"),1)) kWh") } }
                SimpleTable(header: [m.t("fleet_group"), "SOC (%)", "V2H + V2G (kWh)", m.t("fleet_low_departures")],
                    rows: dicts(result["groups"]).map { row in
                        let range = fmt(num(row, "min_soc_pct"), 1) + "–" + fmt(num(row, "max_soc_pct"), 1)
                        return [str(row, "name"), range, fmt(num(row, "discharged_kwh"), 1), fmt(num(row, "departures_below_target"))]
                    })
            }
        }
        .onAppear {
            let fleet = m.section("fleet")
            charge = num(fleet, "site_charge_kw", 7.2)
            discharge = num(fleet, "site_discharge_kw")
            let rule = fleet["v2g"] as? JSONDict
            export = rule != nil
            exportFields = numberInputs(exportSpecs, rule ?? [:])
            let days = rule?["weekdays"] as? [Int] ?? []
            exportDays = Set(days.isEmpty ? Array(0..<7) : days)
        }
        .sheet(item: $edit) { draft in
            FleetGroupSheet(record: draft.record) { value in
                var next = groups
                if let index = draft.index { next[index] = value } else { next.append(value) }
                save(next)
            }.environmentObject(m)
        }
    }

    private func save(_ groups: [JSONDict]) {
        if groups.isEmpty { m.setSection("fleet", NSNull()); return }
        guard charge.isFinite, charge > 0, discharge.isFinite, discharge >= 0 else { m.error = m.t("fleet_charge"); return }
        var config: JSONDict = ["groups": groups, "site_charge_kw": charge, "site_discharge_kw": discharge]
        if export {
            guard var rule = payload(exportSpecs, exportFields), num(rule,"limit_kw") > 0,
                num(rule,"start_hour") < 24, num(rule,"end_hour") < 24, discharge > 0 else { m.error = m.t("v2g_hint"); return }
            rule["weekdays"] = exportDays.sorted()
            config["v2g"] = rule
        }
        m.setSection("fleet", config)
    }
}

private struct FleetGroupSheet: View {
    let record: JSONDict
    let save: (JSONDict) -> Void
    @EnvironmentObject var m: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var fields: [String: Double] = [:]
    @State private var days = Set<Int>()
    @State private var strategy = "solar_preferred"
    @State private var schedule = ""
    @State private var minimumKW = 0.0
    @State private var surplusFraction = 0.0
    private var weekly: JSONDict? { record["weekly_schedule"] as? JSONDict }
    private var surplusStart: JSONDict? { record["surplus_start"] as? JSONDict }
    private let specs: [InputSpec] = [
        .init("vehicles","fleet_vehicles",1,whole:true), .init("battery_kwh","fleet_battery",60),
        .init("initial_soc_pct","initial_soc",60), .init("departure_soc_pct","fleet_departure",60),
        .init("trip_kwh","fleet_trip",12), .init("arrival_hour","ev_arrival",18,whole:true), .init("departure_hour","ev_departure",7,whole:true),
        .init("charge_kw","ev_charger",7.2), .init("charge_efficiency","ev_efficiency",0.9),
        .init("discharge_kw","fleet_v2h",0), .init("discharge_efficiency","fleet_v2h_eff",0.9),
    ]

    var body: some View {
        Form {
            TextField(m.t("fleet_group"), text: $name)
            Text(m.t("fleet_per_vehicle")).foregroundStyle(.secondary)
            ForEach(specs) { s in
                if weekly == nil || (s.id != "arrival_hour" && s.id != "departure_hour") {
                    NumField(label:m.t(s.label), value:s.field($fields))
                }
            }
            if let weekly {
                Text(m.t(str(weekly, "origin") == "year_start" ? "fleet_weekly_year" : "fleet_weekly_monday"))
                    .foregroundStyle(.secondary)
                LabeledContent(m.t("fleet_weekly")) {
                    TextEditor(text: $schedule).font(.system(.body, design: .monospaced)).frame(minHeight: 135)
                }
            } else {
                LabeledContent(m.t("fleet_arrival_days")) {
                    HStack {
                        ForEach(0..<7) { i in
                            Toggle(Calendar.current.shortWeekdaySymbols[(i + 1) % 7], isOn: Binding(
                                get: { days.contains(i) }, set: { if $0 { days.insert(i) } else { days.remove(i) } }
                            )).toggleStyle(.checkbox)
                        }
                    }
                }
            }
            if surplusStart != nil {
                NumField(label: m.t("fleet_surplus_min_kw"), value: $minimumKW)
                NumField(label: m.t("fleet_surplus_fraction"), value: $surplusFraction)
            }
            Picker(m.t("ev_strategy"), selection: $strategy) {
                Text(m.t("ev_solar")).tag("solar_preferred")
                Text(m.t("ev_immediate")).tag("immediate")
            }
            HStack {
                Button(m.t("cancel")) { dismiss() }
                Button(m.t("apply")) {
                    guard var value = payload(specs, fields) else { m.error = m.t("issues"); return }
                    value["name"] = name.trimmingCharacters(in: .whitespacesAndNewlines)
                    value["weekdays"] = days.sorted()
                    value["strategy"] = strategy
                    if var weekly {
                        let bits = schedule.filter { !$0.isWhitespace }
                        guard bits.count == 168, bits.allSatisfy({ $0 == "0" || $0 == "1" }),
                              bits.contains("1"), num(value, "trip_kwh") == 0 || bits.contains("0") else {
                            m.error = m.t("fleet_weekly"); return
                        }
                        weekly["slots"] = bits.map { $0 == "1" }
                        value["weekly_schedule"] = weekly
                    }
                    if surplusStart != nil {
                        guard minimumKW >= 0, minimumKW <= num(value, "charge_kw"),
                              (0...1).contains(surplusFraction) else {
                            m.error = m.t("fleet_surplus_fraction"); return
                        }
                        value["surplus_start"] = ["minimum_kw": minimumKW, "fraction": surplusFraction]
                    }
                    save(value)
                    dismiss()
                }.buttonStyle(.borderedProminent).disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (weekly == nil && days.isEmpty))
            }
        }.padding(16).frame(width: 595, height: 560)
        .onAppear {
            name = str(record, "name")
            fields = numberInputs(specs, record)
            days = Set(record["weekdays"] as? [Int] ?? Array(0..<5))
            strategy = record["strategy"] as? String ?? "solar_preferred"
            if let slots = weekly?["slots"] as? [Bool] {
                schedule = (0..<7).map { day in slots.dropFirst(day * 24).prefix(24).map { $0 ? "1" : "0" }.joined() }.joined(separator: "\n")
            }
            minimumKW = num(surplusStart ?? [:], "minimum_kw")
            surplusFraction = num(surplusStart ?? [:], "fraction")
        }
    }
}

struct ThermalStoreControls: View {
    @EnvironmentObject var m: AppModel
    @State private var fields: [String: Double] = [:]
    @State private var layered = false
    @State private var layers: [String:Double] = [:]
    private let layerSpecs: [InputSpec] = [
        .init("top_fraction","tank_top_fraction",0.4), .init("ambient_c","tank_ambient_c",20),
        .init("target_c","tank_target_c",65), .init("space_supply_c","tank_space_c",35),
        .init("hot_water_supply_c","tank_dhw_c",50), .init("conductance_w_per_k","tank_transfer",20),
    ]
    private let specs: [InputSpec] = [
        .init("capacity_kwh","tank_capacity",8), .init("initial_kwh","tank_initial",0),
        .init("charge_kw","tank_charge",3), .init("discharge_kw","tank_discharge",3),
        .init("standing_loss_pct_per_day","tank_standing",2), .init("daily_hot_water_kwh","tank_dhw",0),
        .init("hot_water_hour","tank_dhw_hour",19,whole:true),
    ]
    var body: some View {
        let controls = m.section("heat_store")["controls"] as? JSONDict
        DisclosureGroup(m.t("tank")) {
            Text(m.t("tank_hint")).foregroundStyle(.secondary)
            ForEach(specs) { s in NumField(label:m.t(s.label), value:s.field($fields)) }
            if controls == nil {
                DisclosureGroup(m.t("tank_layers")) {
                    Toggle(m.t("tank_layers"), isOn: $layered)
                    if layered {
                        ForEach(layerSpecs) { field in NumField(label:m.t(field.label), value:field.field($layers)) }
                    }
                }
            }
            HStack {
                Button(m.t("apply")) {
                    guard var value = payload(specs, fields) else { m.error = m.t("issues"); return }
                    if let controls {
                        value["controls"] = controls
                    } else if layered {
                        guard let spec = payload(layerSpecs, layers) else { m.error = m.t("issues"); return }
                        value["layers"] = spec
                    }
                    m.setSection("heat_store", value)
                }.buttonStyle(.borderedProminent).disabled(m.section("heating").isEmpty)
                Button(m.t("remove")) { m.setSection("heat_store", NSNull()) }.disabled(m.section("heat_store").isEmpty)
            }
            if let result = m.summary?["heat_store"] as? JSONDict {
                if result["end_top_c"] is NSNumber {
                    LabeledContent(m.t("tank_top")) { Text(fmt(num(result,"end_top_c"),1)) }
                    LabeledContent(m.t("tank_bottom")) { Text(fmt(num(result,"end_bottom_c"),1)) }
                }
            }
        }.onAppear {
            let config = m.section("heat_store")
            fields = numberInputs(specs, config)
            let spec = config["layers"] as? JSONDict
            layered = spec != nil
            layers = numberInputs(layerSpecs, spec ?? [:])
        }
    }
}

struct IslandClusterControls: View {
    @EnvironmentObject var m: AppModel
    @State private var edit: RecordEdit?
    private var rows: [JSONDict] { dicts(m.raw["island_clusters"]) }

    var body: some View {
        Section(m.t("clusters")) {
            Text(m.t("clusters_hint")).foregroundStyle(.secondary)
            Text(m.t("island_load_hint")).foregroundStyle(.secondary)
            ForEach(rows.indices, id: \.self) { i in
                HStack {
                    Text(str(rows[i], "name"))
                    Spacer()
                    Button(m.t("advanced_edit")) { edit = RecordEdit(index: i, record: rows[i]) }
                    Button(m.t("import_load")) { m.importLoad(island: str(rows[i],"name")) }
                    Button(m.t("remove")) { var next = rows; next.remove(at: i); m.setSection("island_clusters", next) }
                }
            }
            Button(m.t("advanced_add")) { edit = RecordEdit(index: nil, record: [:]) }
            let results = dicts(m.summary?["clusters"])
            if !results.isEmpty {
                SimpleTable(header: [m.t("name"), "kWh", m.t("island_unserved"), m.t("generator")], rows: results.map { row in
                    let balance = row["balance"] as? JSONDict ?? [:]
                    return [str(row, "name"), fmt(num(balance,"load_served_kwh"),1), fmt(num(balance,"unserved_kwh"),1), fmt(num(balance,"generator_kwh"),1)]
                })
            }
        }
        .sheet(item: $edit) { draft in
            IslandClusterSheet(record: draft.record) { value in
                var next = rows
                if let index = draft.index { next[index] = value } else { next.append(value) }
                m.setSection("island_clusters", next)
            }.environmentObject(m)
        }
    }
}

private struct IslandClusterSheet: View {
    let record: JSONDict
    let save: (JSONDict) -> Void
    @EnvironmentObject var m: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var assigned = Set<String>()
    @State private var share = 100.0
    @State private var source = "shared"
    @State private var annual = 4000.0
    @State private var battery = ""
    @State private var converter = ""
    @State private var generator = ""
    @State private var charging = false
    @State private var fields: [String: Double] = [:]
    private let storage: [InputSpec] = [
        .init("units","cluster_battery_units",1,whole:true), .init("soc_min_pct","cluster_soc_min",10),
        .init("soc_max_pct","cluster_soc_max",95), .init("initial_soc_pct","initial_soc",50)
    ]
    private let controls: [InputSpec] = [
        .init("minimum_generator_pct","island_min",25), .init("minimum_run_hours","island_run",1,whole:true),
        .init("start_soc_pct","island_start",25), .init("stop_soc_pct","island_stop",75),
        .init("fuel_price","island_fuel_price",0), .init("fuel_escalation_pct","island_fuel_rise",0), .init("generator_investment","island_gen_cost",0)
    ]
    private var units: [JSONDict] { dicts(m.stringingDoc?["inverters"]).compactMap { $0["unit"] as? JSONDict } }
    private var equipment: JSONDict { m.section("library") }
    private var previous: JSONDict { record["storage"] as? JSONDict ?? m.section("storage") }

    var body: some View {
        Form {
            TextField(m.t("name"), text: $name)
            Picker(m.t("island_load_source"), selection: $source) {
                Text(m.t("island_load_shared")).tag("shared")
                ForEach(["flat","residential","office","retail"], id: \.self) { kind in Text(m.t("profile_" + kind)).tag(kind) }
                let prior = record["load"] as? JSONDict ?? [:]
                if let profile = prior["profile"] as? JSONDict, str(profile,"kind") == "series" {
                    Text(str(profile,"name")).tag("series")
                }
            }
            if source == "shared" {
                NumField(label: m.t("cluster_share"), value: $share)
            } else if source != "series" {
                NumField(label: m.t("annual_kwh"), value: $annual)
            }
            Section(m.t("cluster_pv")) {
                ForEach(units.indices, id: \.self) { i in
                    let id = str(units[i], "id")
                    Toggle(str(units[i], "name"), isOn: Binding(
                        get: { assigned.contains(id) }, set: { if $0 { assigned.insert(id) } else { assigned.remove(id) } }
                    ))
                }
            }
            Picker(m.t("battery"), selection: $battery) {
                Text(m.t("none")).tag("")
                ForEach(dicts(equipment["batteries"]), id: \.description) { row in Text(str(row,"model")).tag(str(row,"id")) }
            }
            Picker(m.t("coupling"), selection: $converter) {
                Text(m.t("integrated")).tag("integrated")
                ForEach(dicts(equipment["battery_inverters"]), id: \.description) { row in Text(str(row,"model")).tag("ac:" + str(row,"id")) }
                ForEach(units.indices, id: \.self) { i in Text(str(units[i], "name")).tag("dc:" + str(units[i], "id")) }
            }
            ForEach(storage) { s in NumField(label:m.t(s.label), value:s.field($fields)) }
            if m.systemType == "off_grid_generator" {
                Picker(m.t("generator"), selection: $generator) {
                    Text(m.t("none")).tag("")
                    ForEach(dicts(equipment["generators"]), id: \.description) { row in Text(str(row,"model")).tag(str(row,"id")) }
                }
                if !generator.isEmpty {
                    Toggle(m.t("island_charge"), isOn: $charging)
                    ForEach(controls) { s in NumField(label:m.t(s.label), value:s.field($fields)) }
                }
            }
            HStack {
                Button(m.t("cancel")) { dismiss() }
                Button(m.t("apply")) { apply() }.buttonStyle(.borderedProminent)
                    .disabled(name.trimmingCharacters(in:.whitespacesAndNewlines).isEmpty || battery.isEmpty || !(0...100).contains(share))
            }
        }.padding(16).frame(width: 540, height: 610)
        .onAppear {
            name = str(record, "name")
            assigned = Set(record["inverter_units"] as? [String] ?? [])
            share = num(record, "load_share_pct", 100)
            let load = record["load"] as? JSONDict ?? [:]
            annual = num(load,"annual_kwh",4000)
            let profile = load["profile"] as? JSONDict ?? [:]
            source = str(profile,"kind") == "synthetic" ? str(profile,"profile") : (str(profile,"kind") == "series" ? "series" : "shared")
            battery = str(previous, "battery_id")
            let c = previous["coupling"] as? JSONDict ?? [:]
            switch str(c,"kind") {
            case "dc_hybrid": converter = "dc:" + str(c,"unit_id")
            case "ac_coupled": converter = "ac:" + str(c,"battery_inverter_id")
            default: converter = "integrated"
            }
            generator = str(previous, "generator_id")
            let plan = record["controls"] as? JSONDict ?? m.section("island_plan")
            fields = numberInputs(storage, previous).merging(numberInputs(controls, plan)) { current, _ in current }
            charging = plan["charge_battery"] as? Bool ?? false
        }
    }

    private func apply() {
        guard let edited = payload(storage, fields), var plan = payload(controls, fields) else { m.error = m.t("issues"); return }
        var spec = previous.merging(edited) { _, updated in updated }
        spec["battery_id"] = battery
        if converter == "integrated" { spec["coupling"] = ["kind":"integrated"] }
        else if converter.hasPrefix("dc:") { spec["coupling"] = ["kind":"dc_hybrid","unit_id":String(converter.dropFirst(3))] }
        else { spec["coupling"] = ["kind":"ac_coupled","battery_inverter_id":String(converter.dropFirst(3))] }
        spec["model"] = previous["model"] ?? "simple"
        spec["backup_reserve_pct"] = spec["backup_reserve_pct"] ?? 0
        spec["grid_services_pct"] = 0
        spec["charge_priority"] = spec["charge_priority"] ?? ["self_consumption","backup","grid_services"]
        spec["grid_charge_backup"] = false
        spec["grid_service"] = ["kind":"none"]
        spec["outages"] = [JSONDict]()
        if m.systemType == "off_grid_generator", !generator.isEmpty { spec["generator_id"] = generator }
        else { spec.removeValue(forKey: "generator_id") }
        plan["charge_battery"] = charging
        var draft: JSONDict = ["name":name.trimmingCharacters(in:.whitespacesAndNewlines), "load_share_pct":share,
            "storage":spec, "inverter_units":assigned.sorted()]
        if m.systemType == "off_grid_generator", !generator.isEmpty { draft["controls"] = plan }
        if source == "series" { draft["load"] = record["load"] }
        else if source != "shared" {
            guard annual.isFinite, annual > 0 else { m.error = m.t("annual_kwh"); return }
            draft["load"] = ["profile":["kind":"synthetic","profile":source], "annual_kwh":annual,"critical_fraction":1] as JSONDict
        }
        save(draft)
        dismiss()
    }
}

struct AdvancedHourSeries: View {
    @EnvironmentObject var m: AppModel
    @State private var selected = 0

    private var channels: [(label: String, field: String, index: Int)] {
        func series(_ rows: [JSONDict], _ title: String, _ label: String, _ field: String, _ unit: String) -> [(String, String, Int)] {
            rows.enumerated().map { (i, row) in ("\(title) · \(str(row,label)) (\(unit))", field, i) }
        }
        let fleet = m.summary?["fleet"] as? JSONDict ?? [:]
        let clusters = dicts(m.summary?["clusters"])
        return series(dicts(fleet["groups"]), m.t("fleet"), "name", "fleet_soc_pct", "%")
             + series(clusters, m.t("clusters"), "name", "cluster_soc_pct", "%")
             + series(clusters, m.t("island_unserved"), "name", "cluster_unserved_wh", "Wh")
             + series(clusters, m.t("loads"), "name", "cluster_load_wh", "Wh")
             + series(dicts(m.summary?["routed_cables"]), m.t("cable_planner"), "label", "routed_cable_wh", "Wh")
    }

    var body: some View {
        let series = channels
        if !series.isEmpty {
            Picker(m.t("hours_inspector"), selection: $selected) {
                ForEach(series.indices, id: \.self) { i in Text(series[i].label).tag(i) }
            }
            let column = series[min(selected, series.count - 1)]
            SimpleTable(header: [m.t("day"), m.t("hour"), column.label], rows: m.dayRows.map { row in
                let values = row[column.field] as? [NSNumber] ?? []
                let value = values.indices.contains(column.index) ? values[column.index].doubleValue : 0
                return [fmt(num(row,"day_of_year")), String(format:"%02d:00",int(row,"hour")), fmt(value, 2)]
            }).font(.caption)
        }
    }
}
