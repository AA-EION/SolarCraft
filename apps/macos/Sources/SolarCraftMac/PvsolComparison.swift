// SPDX-License-Identifier: GPL-3.0-or-later
import SwiftUI

struct PvsolComparison: View {
    @EnvironmentObject var m: AppModel

    var body: some View {
        if let data = m.pvsolComparison, let reference = data["reference"] as? JSONDict {
            GroupBox(m.t("pvsol_reference")) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(str(reference,"source_name") + " · " + m.t("pvsol_format") + " " + number(reference, "format", 0))
                    DisclosureGroup(m.t("pvsol_scope")) {
                        let warnings = reference["warnings"] as? [String] ?? []
                        ForEach(warnings, id: \.self) { Text($0).foregroundStyle(.secondary) }
                    }
                    if let comparison = data["comparison"] as? JSONDict {
                        if comparison.isEmpty { Text(m.t("pvsol_no_saved_results")) }
                        if data["imported_input_hash_matches"] as? Bool == false {
                            Text(m.t("pvsol_edited_design")).foregroundStyle(.secondary)
                        }
                        ForEach(["pv_dc","inverter_ac"], id: \.self) { key in
                            if let channel = comparison[key] as? JSONDict { track(key, channel) }
                        }
                    } else { Text(m.t("pvsol_run_to_compare")) }
                }
            }
        }
    }

    private func number(_ row: JSONDict, _ key: String, _ digits: Int = 2) -> String {
        (row[key] as? NSNumber).map { fmt($0.doubleValue, digits) } ?? "–"
    }

    private func track(_ key: String, _ row: JSONDict) -> some View {
        let name = m.t(key == "pv_dc" ? "pvsol_dc" : "pvsol_ac")
        return DisclosureGroup(name + " · PV*SOL " + number(row,"pvsol_kwh") + " → SolarCraft " + number(row,"solarcraft_kwh")
            + " kWh · Δ " + number(row,"delta_pct") + "%") {
            let months = dicts(row["monthly"])
            if !months.isEmpty {
                SimpleTable(header: [m.t("pvsol_month"), "PV*SOL (kWh)", "SolarCraft (kWh)", "Δ (kWh)"],
                    rows: months.map { [number($0,"month",0), number($0,"pvsol_kwh"), number($0,"solarcraft_kwh"), number($0,"delta_kwh")] })
            }
            if let hourly = row["hourly"] as? JSONDict {
                Text(m.t("pvsol_hourly_error") + " " + number(hourly,"mae_kwh",3) + " kWh · " + str(hourly,"source"))
            }
        }
    }
}
