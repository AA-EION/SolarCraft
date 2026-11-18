// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private void BuildDemandAndIsland()
    {
        var ev = _project?["ev"];
        var evStrategy = Combo(T("ev_strategy"), new[] { ("immediate", T("ev_immediate")), ("solar_preferred", T("ev_solar")) }, S(ev, "strategy"));
        if (_project?["fleet"] is JsonObject) EvPage.Children.Clear();
        else DemandEditor(EvPage, "ev", SystemType is "grid_ev" or "grid_ev_battery", [
            ("daily_kwh", "ev_daily", 12, false), ("charger_kw", "ev_charger", 7.2, false), ("efficiency", "ev_efficiency", 0.9, false),
            ("arrival_hour", "ev_arrival", 18, true), ("departure_hour", "ev_departure", 7, true),
        ], evStrategy, T("ev_window"), values => values["strategy"] = Sel(evStrategy));
        DemandEditor(HeatingPage, "heating", SystemType is "grid_thermal" or "grid_thermal_battery", [
            ("setpoint_c", "heat_setpoint", 20, false), ("heat_loss_kw_per_k", "heat_loss", 0.18, false),
            ("thermal_kw", "heat_capacity", 6, false), ("cutoff_c", "heat_cutoff", -15, false),
            ("cop_at_0c", "heat_cop0", 2.5, false), ("cop_at_10c", "heat_cop10", 3.8, false),
            ("resistance_backup_kw", "heat_backup", 3, false),
        ], null, "", null);
        BuildFleet();
        BuildHeatStore();
        BuildIslandPage();
    }

    private void DemandEditor(Panel panel, string section, bool required, (string key, string label, double fallback, bool whole)[] specs,
        UIElement? choice, string hint, Action<JsonObject>? extra)
    {
        panel.Children.Clear();
        var data = _project?[section];
        var use = new ToggleSwitch { Header = T(section == "ev" ? "ev_page" : "heat_page"), IsOn = required || data is JsonObject, IsEnabled = !required };
        panel.Children.Add(use);
        var fields = new StackPanel { Spacing = 8 };
        var editors = MakeFields(fields, data, specs);
        if (choice is not null) fields.Children.Add(choice);
        if (hint.Length > 0) fields.Children.Add(Note(hint));
        panel.Children.Add(fields);
        void ShowFields() => fields.Visibility = use.IsOn ? Visibility.Visible : Visibility.Collapsed;
        ShowFields();
        use.Toggled += (_, _) => ShowFields();
        panel.Children.Add(Btn(T("apply"), () =>
        {
            if (!use.IsOn) { SetSection(section, null); return; }
            if (!Try(() =>
            {
                var value = ReadFields(editors);
                extra?.Invoke(value);
                Api.Invoke("set_section", new { file = _file, section, value });
            })) return;
            Reload();
        }, true));
        var result = _summary?[section];
        if (result is null) return;
        var electricity = section == "ev" ? N(result, "electrical_kwh") : N(result, "pump_electric_kwh") + N(result, "backup_kwh");
        var unmet = N(result, section == "ev" ? "unscheduled_kwh" : "unmet_heat_kwh");
        panel.Children.Add(Table([T("workflow_results"), "kWh"], new IList<string>[] {
            new[] { T("annual_kwh"), F(electricity, 1) }, new[] { T("island_unserved"), F(unmet, 1) },
        }));
    }

    private (string key, NumberBox field, bool whole)[] MakeFields(Panel panel, JsonNode? current, (string key, string label, double fallback, bool whole)[] specs)
    {
        var fields = specs.Select(s => (s.key, field: Num(T(s.label), N(current, s.key, s.fallback), 295), s.whole)).ToArray();
        foreach (var entry in fields) panel.Children.Add(entry.field);
        return fields;
    }

    private JsonObject ReadFields(IEnumerable<(string key, NumberBox field, bool whole)> fields)
    {
        var value = new JsonObject();
        foreach (var (key, field, whole) in fields)
        {
            if (!double.IsFinite(field.Value) || (whole && (field.Value != Math.Truncate(field.Value) || field.Value < 0 || field.Value > int.MaxValue)))
                throw new ArgumentException(field.Header?.ToString());
            value[key] = whole ? JsonValue.Create((int)field.Value) : JsonValue.Create(field.Value);
        }
        return value;
    }

    private void BuildIslandPage()
    {
        var panel = IslandPage;
        panel.Children.Clear();
        if (!Islanded) return;
        BuildIslandClusters();
        panel.Children.Add(Note(T("island_cost_hint")));
        var plan = _project?["island_plan"];
        var generator = new StackPanel { Spacing = 7, Visibility = SystemType == "off_grid_generator" && Arr(_project?["island_clusters"]).Count == 0 ? Visibility.Visible : Visibility.Collapsed };
        panel.Children.Add(generator);
        var controls = MakeFields(generator, plan, [
            ("minimum_generator_pct", "island_min", 25, false), ("minimum_run_hours", "island_run", 1, true),
            ("start_soc_pct", "island_start", 25, false), ("stop_soc_pct", "island_stop", 75, false),
            ("fuel_price", "island_fuel_price", 0, false), ("fuel_escalation_pct", "island_fuel_rise", 0, false),
            ("generator_investment", "island_gen_cost", 0, false),
        ]);
        var allowCharge = new CheckBox { Content = T("island_charge"), IsChecked = plan?["charge_battery"]?.GetValue<bool>() == true };
        generator.Children.Insert(2, allowCharge);
        var economics = _project?["economics"];
        var costs = MakeFields(panel, economics, [
            ("horizon_years", "horizon", 20, true), ("discount_rate_pct", "discount_rate", 4, false),
            ("cost_fixed", "cost_fixed", 1500, false), ("cost_per_kwp", "cost_per_kwp", 1100, false),
            ("cost_per_battery_kwh", "cost_per_kwh", 450, false), ("om_pct_of_investment", "om", 1, false),
            ("om_escalation_pct", "island_maintenance_escalation", 2, false),
        ]);
        var replacements = new StackPanel { Spacing = 8 };
        var optionalCosts = MakeFields(replacements, economics, [
            ("inverter_replacement_year", "island_inverter_year", 13, true), ("inverter_replacement_cost", "island_inverter_cost", 0, false),
            ("battery_replacement_year", "island_battery_year", 12, true), ("battery_replacement_cost", "island_battery_cost", 0, false),
            ("subsidy", "island_subsidy", 0, false),
        ]);
        panel.Children.Add(new Expander { Header = T("island_replacements"), Content = replacements });
        var currency = new TextBox { Header = T("currency"), Text = S(_project?["tariff"], "currency"), Width = 160, HorizontalAlignment = HorizontalAlignment.Left };
        panel.Children.Add(currency);
        panel.Children.Add(Btn(T("apply"), () =>
        {
            if (!Try(() =>
            {
                var settings = ReadFields(controls);
                settings["charge_battery"] = allowCharge.IsChecked == true;
                var edits = new JsonArray(
                    new JsonObject { ["pointer"] = "/island_plan", ["value"] = settings },
                    new JsonObject { ["pointer"] = "/tariff/currency", ["value"] = currency.Text });
                foreach (var field in ReadFields(costs.Concat(optionalCosts)))
                    edits.Add(new JsonObject { ["pointer"] = "/economics/" + field.Key, ["value"] = field.Value?.DeepClone() });
                Api.Invoke("set_fields", new { file = _file, edits });
            })) return;
            Reload();
        }, true));
        var r = _summary?["island_finance"];
        if (r is null) return;
        var money = S(r, "currency");
        panel.Children.Add(H(T("workflow_results")));
        panel.Children.Add(Note($"{T("island_lifetime")} {F(N(r, "discounted_cost"), 2)} {money}"));
        if (r["cost_per_served_kwh"] is not null)
            panel.Children.Add(Note($"{T("island_unit_cost")} {F(N(r, "cost_per_served_kwh"), 3)} {money}/kWh"));
        panel.Children.Add(Table([T("year"), T("island_fuel_cost"), T("island_year_cost")], Arr(r["years"])
            .Select(y => (IList<string>)new[] { F(N(y, "year")), F(N(y, "fuel_cost"), 2), F(N(y, "discounted_cost"), 2) })));
    }
}
