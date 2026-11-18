// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private async Task<JsonObject?> AdvancedDialog(string title, Panel fields, Func<JsonObject> draft)
    {
        var status = Note("");
        fields.Children.Add(status);
        var dialog = new ContentDialog { Title = title, XamlRoot = Content.XamlRoot,
            Content = new ScrollViewer { Content = fields, MaxHeight = 570 },
            PrimaryButtonText = T("apply"), CloseButtonText = T("cancel") };
        JsonObject? result = null;
        dialog.PrimaryButtonClick += (_, args) =>
        {
            try { result = draft(); }
            catch (Exception error) { args.Cancel = true; status.Text = error.Message; }
        };
        return await dialog.ShowAsync() == ContentDialogResult.Primary ? result : null;
    }

    private ComboBox AdvancedList(Panel panel, JsonArray rows, string title, Func<JsonNode?, Task<JsonObject?>> edit, Action<JsonArray> save)
    {
        var file = _file;
        var select = Combo(title, rows.Select((r, i) => (i.ToString(), S(r, "name"))).ToList(), "0");
        if (rows.Count > 0) panel.Children.Add(select);
        var add = new Button { Content = T("advanced_add") };
        var change = new Button { Content = T("advanced_edit"), IsEnabled = rows.Count > 0 };
        var remove = Btn(T("remove"), () =>
        {
            if (select.SelectedIndex < 0) return;
            var next = (JsonArray)rows.DeepClone();
            next.RemoveAt(select.SelectedIndex);
            save(next);
        });
        remove.IsEnabled = rows.Count > 0;
        panel.Children.Add(Row(add, change, remove));

        async Task Change(int index)
        {
            var entered = await edit(index < 0 ? null : rows[index]);
            if (entered is null || file != _file) return;
            var next = (JsonArray)rows.DeepClone();
            if (index < 0) next.Add(entered); else next[index] = entered;
            save(next);
        }
        add.Click += async (_, _) => await Change(-1);
        change.Click += async (_, _) => { if (select.SelectedIndex >= 0) await Change(select.SelectedIndex); };
        return select;
    }

    private void BuildFleet()
    {
        var fleet = _project?["fleet"];
        var groups = Arr(fleet?["groups"]);
        var panel = new StackPanel { Spacing = 9 };
        panel.Children.Add(Note(T("fleet_hint")));
        var limits = MakeFields(panel, fleet, [
            ("site_charge_kw", "fleet_charge", 7.2, false), ("site_discharge_kw", "fleet_discharge", 0, false),
        ]);
        var v2g = fleet?["v2g"];
        var schedule = new StackPanel { Spacing = 8 };
        var export = new CheckBox { Content = T("v2g"), IsChecked = v2g is JsonObject };
        schedule.Children.Add(export);
        var exportDetails = new StackPanel { Spacing = 8 };
        schedule.Children.Add(exportDetails);
        exportDetails.Children.Add(Note(T("v2g_hint")));
        var hours = MakeFields(exportDetails, v2g, [
            ("limit_kw", "v2g_limit", 2, false), ("start_hour", "v2g_start", 18, true), ("end_hour", "v2g_end", 22, true),
        ]);
        var days = Weekdays(v2g?["weekdays"], true);
        exportDetails.Children.Add(Note(T("v2g_days")));
        exportDetails.Children.Add(Row(days.Take(4).Cast<UIElement>().ToArray()));
        exportDetails.Children.Add(Row(days.Skip(4).Cast<UIElement>().ToArray()));
        void ShowExport() => exportDetails.Visibility = export.IsChecked == true ? Visibility.Visible : Visibility.Collapsed;
        export.Checked += (_, _) => ShowExport();
        export.Unchecked += (_, _) => ShowExport();
        ShowExport();
        panel.Children.Add(new Expander { Header = T("v2g"), Content = schedule, IsExpanded = v2g is JsonObject });
        void Save(JsonArray rows)
        {
            if (rows.Count == 0) { SetSection("fleet", null); return; }
            if (!Try(() => {
                var config = ReadFields(limits);
                if (N(config, "site_charge_kw") <= 0 || N(config, "site_discharge_kw") < 0) throw new ArgumentException(T("fleet_charge"));
                config["groups"] = rows;
                if (export.IsChecked == true)
                {
                    var rule = ReadFields(hours);
                    if (N(rule,"limit_kw") <= 0 || N(rule,"start_hour") > 23 || N(rule,"end_hour") > 23 || N(config,"site_discharge_kw") <= 0)
                        throw new ArgumentException(T("v2g_hint"));
                    rule["weekdays"] = SelectedDays(days);
                    config["v2g"] = rule;
                }
                Api.Invoke("set_section", new { file = _file, section = "fleet", value = config });
            })) return;
            Reload();
        }
        AdvancedList(panel, groups, T("fleet_group"), EditFleetGroup, Save);
        if (groups.Count > 0) panel.Children.Add(Btn(T("apply"), () => Save((JsonArray)groups.DeepClone()), true));
        if (_summary?["fleet"] is JsonNode results)
        {
            if (v2g is JsonObject) panel.Children.Add(Note($"{T("v2g")} · {F(N(results,"v2g_kwh"),1)} kWh"));
            panel.Children.Add(Table([T("fleet_group"), T("fleet_vehicles"), "SOC (%)", "V2H + V2G (kWh)", T("fleet_low_departures"), T("fleet_travel_unmet")],
                Arr(results["groups"]).Select(g => (IList<string>)new[] { S(g,"name"), F(N(g,"vehicles")),
                    $"{F(N(g,"min_soc_pct"),1)}–{F(N(g,"max_soc_pct"),1)}", F(N(g,"discharged_kwh"),1),
                    F(N(g,"departures_below_target")), F(N(g,"unmet_travel_kwh"),1) })));
        }
        EvPage.Children.Insert(0, new Expander { Header = T("fleet"), Content = panel, IsExpanded = groups.Count > 0 });
    }

    private Task<JsonObject?> EditFleetGroup(JsonNode? current)
    {
        var panel = new StackPanel { Spacing = 8 };
        var name = new TextBox { Header = T("fleet_group"), Text = S(current, "name") };
        panel.Children.Add(name);
        panel.Children.Add(Note(T("fleet_per_vehicle")));
        var fields = MakeFields(panel, current, [
            ("vehicles", "fleet_vehicles", 1, true), ("battery_kwh", "fleet_battery", 60, false),
            ("initial_soc_pct", "initial_soc", 60, false), ("departure_soc_pct", "fleet_departure", 60, false),
            ("trip_kwh", "fleet_trip", 12, false), ("arrival_hour", "ev_arrival", 18, true),
            ("departure_hour", "ev_departure", 7, true), ("charge_kw", "ev_charger", 7.2, false),
            ("charge_efficiency", "ev_efficiency", 0.9, false), ("discharge_kw", "fleet_v2h", 0, false),
            ("discharge_efficiency", "fleet_v2h_eff", 0.9, false),
        ]);
        var days = Weekdays(current?["weekdays"], false);
        panel.Children.Add(H(T("fleet_arrival_days")));
        panel.Children.Add(Row(days.Take(4).Cast<UIElement>().ToArray()));
        panel.Children.Add(Row(days.Skip(4).Cast<UIElement>().ToArray()));
        var strategy = Combo(T("ev_strategy"), [("immediate", T("ev_immediate")), ("solar_preferred", T("ev_solar"))], S(current, "strategy"));
        panel.Children.Add(strategy);
        return AdvancedDialog(T("fleet_group"), panel, () =>
        {
            var group = ReadFields(fields);
            group["name"] = name.Text.Trim();
            group["weekdays"] = SelectedDays(days);
            group["strategy"] = Sel(strategy);
            if (name.Text.Trim().Length == 0 || Arr(group["weekdays"]).Count == 0 || N(group, "arrival_hour") > 23
                || N(group, "departure_hour") > 23 || N(group, "vehicles") < 1 || N(group,"trip_kwh") > N(group,"battery_kwh"))
                throw new ArgumentException(T("fleet_per_vehicle"));
            return group;
        });
    }

    private static CheckBox[] Weekdays(JsonNode? value, bool allByDefault)
    {
        var selected = Arr(value).Select(v => v!.GetValue<int>()).ToHashSet();
        return Enumerable.Range(0, 7).Select(i => new CheckBox {
            Content = System.Globalization.CultureInfo.CurrentUICulture.DateTimeFormat.GetAbbreviatedDayName((DayOfWeek)((i + 1) % 7)),
            IsChecked = selected.Count == 0 ? allByDefault || i < 5 : selected.Contains(i), MinWidth = 65,
        }).ToArray();
    }

    private static JsonArray SelectedDays(CheckBox[] days) =>
        new(days.Select((box, i) => (box, i)).Where(day => day.box.IsChecked == true)
            .Select(day => (JsonNode?)JsonValue.Create(day.i)).ToArray());

    private void BuildHeatStore()
    {
        var panel = new StackPanel { Spacing = 9 };
        panel.Children.Add(Note(T("tank_hint")));
        var current = _project?["heat_store"];
        var fields = MakeFields(panel, current, [
            ("capacity_kwh", "tank_capacity", 8, false), ("initial_kwh", "tank_initial", 0, false),
            ("charge_kw", "tank_charge", 3, false), ("discharge_kw", "tank_discharge", 3, false),
            ("standing_loss_pct_per_day", "tank_standing", 2, false), ("daily_hot_water_kwh", "tank_dhw", 0, false),
            ("hot_water_hour", "tank_dhw_hour", 19, true),
        ]);
        var layers = current?["layers"];
        var layerPanel = new StackPanel { Spacing = 7 };
        var layered = new CheckBox { Content = T("tank_layers"), IsChecked = layers is JsonObject };
        layerPanel.Children.Add(layered);
        var layerDetails = new StackPanel { Spacing = 7 };
        layerPanel.Children.Add(layerDetails);
        var layerFields = MakeFields(layerDetails, layers, [
            ("top_fraction", "tank_top_fraction", 0.4, false), ("ambient_c", "tank_ambient_c", 20, false),
            ("target_c", "tank_target_c", 65, false), ("space_supply_c", "tank_space_c", 35, false),
            ("hot_water_supply_c", "tank_dhw_c", 50, false), ("conductance_w_per_k", "tank_transfer", 20, false),
        ]);
        void ShowLayers() => layerDetails.Visibility = layered.IsChecked == true ? Visibility.Visible : Visibility.Collapsed;
        layered.Checked += (_, _) => ShowLayers();
        layered.Unchecked += (_, _) => ShowLayers();
        ShowLayers();
        panel.Children.Add(new Expander { Header = T("tank_layers"), Content = layerPanel, IsExpanded = layers is JsonObject });
        var save = Btn(T("apply"), () =>
        {
            if (!Try(() => {
                var value = ReadFields(fields);
                if (layered.IsChecked == true) value["layers"] = ReadFields(layerFields);
                Api.Invoke("set_section", new { file = _file, section = "heat_store", value });
            })) return;
            Reload();
        }, true);
        save.IsEnabled = _project?["heating"] is JsonObject;
        var remove = Btn(T("remove"), () => SetSection("heat_store", null));
        remove.IsEnabled = current is JsonObject;
        panel.Children.Add(Row(save, remove));
        if (_summary?["heat_store"] is JsonNode result && result["end_top_c"] is not null)
            panel.Children.Add(Note($"{T("tank_top")} {F(N(result,"end_top_c"),1)} · {T("tank_bottom")} {F(N(result,"end_bottom_c"),1)}"));
        var store = new Expander { Header = T("tank"), Content = panel, IsExpanded = current is JsonObject };
        if (current is JsonObject) HeatingPage.Children.Insert(0, store);
        else HeatingPage.Children.Add(store);
    }

    private void BuildIslandClusters()
    {
        var rows = Arr(_project?["island_clusters"]);
        var panel = new StackPanel { Spacing = 9 };
        panel.Children.Add(Note(T("clusters_hint")));
        panel.Children.Add(Note(T("island_load_hint")));
        var selection = AdvancedList(panel, rows, T("clusters"), EditIslandCluster, next => SetSection("island_clusters", next));
        if (rows.Count > 0) panel.Children.Add(Btn(T("island_import_load"), async () =>
        {
            if (selection.SelectedIndex < 0) return;
            var island = S(rows[selection.SelectedIndex], "name");
            var file = _file;
            var picker = new Windows.Storage.Pickers.FileOpenPicker();
            picker.FileTypeFilter.Add(".csv");
            picker.FileTypeFilter.Add(".txt");
            InitPicker(picker);
            var selected = await picker.PickSingleFileAsync();
            if (selected is not null && file == _file && Try(() => Api.Invoke("import_load", new { file, source = selected.Path, island }))) Reload();
        }));
        var results = Arr(_summary?["clusters"]);
        if (results.Count > 0)
            panel.Children.Add(Table([T("name"), "kWh", T("island_unserved"), T("generator")],
                results.Select(c => (IList<string>)new[] { S(c,"name"), F(N(c?["balance"],"load_served_kwh"),1),
                    F(N(c?["balance"],"unserved_kwh"),1), F(N(c?["balance"],"generator_kwh"),1) })));
        IslandPage.Children.Add(new Expander { Header = T("clusters"), Content = panel, IsExpanded = rows.Count > 0 });
    }

    private Task<JsonObject?> EditIslandCluster(JsonNode? current)
    {
        var panel = new StackPanel { Spacing = 8 };
        var name = new TextBox { Header = T("name"), Text = S(current,"name") };
        panel.Children.Add(name);
        var load = Num(T("cluster_share"), N(current,"load_share_pct", 100), 280);
        panel.Children.Add(load);
        var selectedLoad = current?["load"];
        var source = selectedLoad?["profile"];
        var choices = new List<(string, string)> { ("shared", T("island_load_shared")) };
        choices.AddRange(new[] { "flat","residential","office","retail" }.Select(key => (key, T("profile_" + key))));
        if (S(source,"kind") == "series") choices.Add(("series", S(source,"name")));
        var profiles = Combo(T("island_load_source"), choices, S(source,"kind") == "synthetic" ? S(source,"profile") : S(source,"kind"));
        panel.Children.Add(profiles);
        var annual = Num(T("annual_kwh"), N(selectedLoad,"annual_kwh",4000), 280);
        panel.Children.Add(annual);
        void ShowLoad()
        {
            load.IsEnabled = Sel(profiles) == "shared";
            annual.IsEnabled = Sel(profiles) is not ("shared" or "series");
        }
        profiles.SelectionChanged += (_, _) => ShowLoad();
        ShowLoad();
        var lib = _project?["library"];
        var storage = current?["storage"] ?? _project?["storage"];
        var units = Arr(Api.Invoke("stringing", new { file = _file })?["inverters"]);
        var checkedUnits = Arr(current?["inverter_units"]).Select(v => v!.GetValue<string>()).ToHashSet();
        panel.Children.Add(H(T("cluster_pv")));
        var assigned = units.Select(r => new CheckBox { Content = S(r?["unit"],"name"), Tag = S(r?["unit"],"id"),
            IsChecked = checkedUnits.Contains(S(r?["unit"],"id")) }).ToArray();
        foreach (var unit in assigned) panel.Children.Add(unit);
        List<(string, string)> Equipment(string family) => Arr(lib?[family])
            .Select(r => (S(r,"id"), $"{S(r,"manufacturer")} {S(r,"model")}")).ToList();
        var battery = Combo(T("battery"), Equipment("batteries"), S(storage,"battery_id"));
        panel.Children.Add(battery);
        var converters = Equipment("battery_inverters").Select(x => ("ac:" + x.Item1, x.Item2)).ToList();
        converters.Add(("integrated", T("integrated")));
        converters.AddRange(units.Where(u => u?["has_battery_port"]?.GetValue<bool>() == true)
            .Select(u => ("dc:" + S(u?["unit"],"id"), S(u?["unit"],"name"))));
        var coupling = storage?["coupling"];
        var converterKey = S(coupling,"kind") switch {
            "dc_hybrid" => "dc:" + S(coupling,"unit_id"), "integrated" => "integrated",
            _ => "ac:" + S(coupling,"battery_inverter_id"),
        };
        var converter = Combo(T("coupling"), converters, converterKey);
        panel.Children.Add(converter);
        var fields = MakeFields(panel, storage, [
            ("units", "cluster_battery_units", 1, true), ("soc_min_pct", "cluster_soc_min", 10, false),
            ("soc_max_pct", "cluster_soc_max", 95, false), ("initial_soc_pct", "initial_soc", 50, false),
        ]);
        List<(string, string)> gens = [("", T("none"))];
        if (SystemType == "off_grid_generator") gens.AddRange(Equipment("generators"));
        var gen = Combo(T("generator"), gens, S(storage,"generator_id"));
        var generator = new StackPanel { Spacing = 8 };
        panel.Children.Add(gen);
        panel.Children.Add(generator);
        var plan = current?["controls"] ?? _project?["island_plan"];
        var charge = new CheckBox { Content = T("island_charge"), IsChecked = plan?["charge_battery"]?.GetValue<bool>() == true };
        generator.Children.Add(charge);
        var controls = MakeFields(generator, plan, [
            ("minimum_generator_pct","island_min",25,false), ("minimum_run_hours","island_run",1,true),
            ("start_soc_pct","island_start",25,false), ("stop_soc_pct","island_stop",75,false),
            ("fuel_price","island_fuel_price",0,false), ("fuel_escalation_pct","island_fuel_rise",0,false),
            ("generator_investment","island_gen_cost",0,false),
        ]);
        void ShowGenerator() => generator.Visibility = Sel(gen).Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        gen.SelectionChanged += (_, _) => ShowGenerator();
        ShowGenerator();
        return AdvancedDialog(T("clusters"), panel, () =>
        {
            if (name.Text.Trim().Length == 0 || !(load.Value >= 0 && load.Value <= 100) || Sel(battery).Length == 0)
                throw new ArgumentException(T("clusters_hint"));
            var baseStorage = storage?.DeepClone() as JsonObject ?? new JsonObject();
            foreach (var field in ReadFields(fields)) baseStorage[field.Key] = field.Value?.DeepClone();
            if (N(baseStorage,"units") < 1) throw new ArgumentException(T("battery"));
            baseStorage["battery_id"] = Sel(battery);
            var key = Sel(converter);
            baseStorage["coupling"] = key switch {
                "integrated" => new JsonObject { ["kind"] = "integrated" },
                var v when v.StartsWith("dc:") => new JsonObject { ["kind"] = "dc_hybrid", ["unit_id"] = v[3..] },
                var v when v.StartsWith("ac:") => new JsonObject { ["kind"] = "ac_coupled", ["battery_inverter_id"] = v[3..] },
                _ => throw new ArgumentException(T("coupling")),
            };
            baseStorage["model"] = storage?["model"]?.DeepClone() ?? JsonValue.Create("simple");
            baseStorage["backup_reserve_pct"] ??= 0;
            baseStorage["grid_services_pct"] = 0;
            baseStorage["charge_priority"] ??= new JsonArray("self_consumption", "backup", "grid_services");
            baseStorage["grid_charge_backup"] = false;
            baseStorage["grid_service"] = new JsonObject { ["kind"] = "none" };
            baseStorage["outages"] = new JsonArray();
            baseStorage["generator_id"] = Sel(gen).Length > 0 ? JsonValue.Create(Sel(gen)) : null;
            var value = Sel(gen).Length > 0 ? ReadFields(controls) : null;
            if (value is not null) value["charge_battery"] = charge.IsChecked == true;
            var loadProfile = Sel(profiles) switch {
                "series" => selectedLoad?.DeepClone(), "shared" => null,
                var profileKind when annual.Value > 0 && double.IsFinite(annual.Value) => new JsonObject {
                    ["profile"] = new JsonObject { ["kind"] = "synthetic", ["profile"] = profileKind },
                    ["annual_kwh"] = annual.Value, ["critical_fraction"] = 1,
                },
                _ => throw new ArgumentException(T("annual_kwh")),
            };
            return new JsonObject { ["name"] = name.Text.Trim(), ["load_share_pct"] = load.Value, ["load"] = loadProfile, ["storage"] = baseStorage,
                ["inverter_units"] = new JsonArray(assigned.Where(c => c.IsChecked == true).Select(c => (JsonNode?)JsonValue.Create((string)c.Tag)).ToArray()),
                ["controls"] = value };
        });
    }
}
