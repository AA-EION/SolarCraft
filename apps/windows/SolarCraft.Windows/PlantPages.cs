// SPDX-License-Identifier: GPL-3.0-or-later
// Plant workflow pages (grid, electrical design and equipment, storage and loads,
// simulation inspection, economics and variants, report contents). Every edit
// goes through the core's validated JSON API, the same contract as CLI and MCP.
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Shapes;
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Windows.Graphics.Imaging;
using Windows.Storage.Pickers;

using IOPath = System.IO.Path;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private JsonNode? _summary, _stringing, _cash;
    private JsonArray? _dayRows, _compareRows, _previewSections, _libraryItems;

    // ---- small helpers ----
    private static double N(JsonNode? n, string k, double d = 0) { try { return n?[k]?.GetValue<double>() ?? d; } catch { return d; } }
    private static string S(JsonNode? n, string k) => n?[k] is JsonValue v && v.TryGetValue<string>(out var s) ? s : "";
    private static string F(double v, int digits = 0) => v.ToString("N" + digits, CultureInfo.CurrentCulture);
    private static JsonArray Arr(JsonNode? n) => n as JsonArray ?? new JsonArray();
    private static TextBlock H(string text) => new() { Text = text, Style = (Style)Application.Current.Resources["SubtitleTextBlockStyle"], Margin = new Thickness(0, 10, 0, 0) };
    private static TextBlock Note(string text) => new() { Text = text, Opacity = 0.7, TextWrapping = TextWrapping.Wrap };
    private static NumberBox Num(string header, double value, double width = 220) => new() { Header = header, Value = value, Width = width, HorizontalAlignment = HorizontalAlignment.Left, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact };
    private static ComboBox Combo(string header, IList<(string tag, string label)> items, string selected)
    {
        var c = new ComboBox { Header = header, MinWidth = 260 };
        foreach (var (tag, label) in items) c.Items.Add(new ComboBoxItem { Content = label, Tag = tag });
        c.SelectedIndex = Math.Max(0, items.ToList().FindIndex(i => i.tag == selected));
        return c;
    }
    private static string Sel(ComboBox c) => (c.SelectedItem as ComboBoxItem)?.Tag as string ?? "";
    private static StackPanel Row(params UIElement[] items) { var p = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 }; foreach (var i in items) p.Children.Add(i); return p; }
    private Button Btn(string text, Action onClick, bool accent = false)
    {
        var b = new Button { Content = text };
        if (accent) b.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
        b.Click += (_, _) => onClick();
        return b;
    }

    /// <summary>A compact table; the first column is left-aligned.</summary>
    private static Grid Table(IList<string> header, IEnumerable<IList<string>> rows, double fontSize = 13)
    {
        var g = new Grid { ColumnSpacing = 14, RowSpacing = 2 };
        var all = rows.ToList();
        int cols = Math.Max(header.Count, all.Count > 0 ? all.Max(r => r.Count) : 1);
        for (int c = 0; c < cols; c++) g.ColumnDefinitions.Add(new ColumnDefinition { Width = c == 0 ? new GridLength(1, GridUnitType.Star) : GridLength.Auto });
        int row = 0;
        void Add(IList<string> cells, bool head)
        {
            g.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            for (int c = 0; c < cells.Count; c++)
            {
                var t = new TextBlock { Text = cells[c], FontSize = fontSize, HorizontalAlignment = c == 0 ? HorizontalAlignment.Left : HorizontalAlignment.Right, Opacity = head ? 0.7 : 1, FontWeight = head ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal };
                Grid.SetRow(t, row); Grid.SetColumn(t, c); g.Children.Add(t);
            }
            row++;
        }
        if (header.Count > 0) Add(header, true);
        foreach (var r in all) Add(r, false);
        return g;
    }

    private void IssuesInto(Panel p, Func<string, bool> filter)
    {
        try
        {
            foreach (var i in Arr(Api.Invoke("validate", new { file = _file })?["issues"]))
            {
                if (!filter(S(i, "pointer")) && !filter(S(i, "code"))) continue;
                var err = S(i, "severity") == "error";
                p.Children.Add(new TextBlock { Text = (err ? "✖ " : "⚠ ") + S(i, "message"), TextWrapping = TextWrapping.Wrap, Foreground = new SolidColorBrush(err ? Colors.Firebrick : Colors.DarkOrange) });
            }
        }
        catch { }
    }

    private void SetSection(string name, JsonNode? value)
    {
        if (!Try(() =>
        {
            var r = Api.Invoke("set_section", new JsonObject { ["file"] = _file, ["section"] = name, ["value"] = value })!;
            var errs = Arr(r["issues"]).Where(i => S(i, "severity") == "error").Select(i => S(i, "message")).ToList();
            if (errs.Count > 0) ShowError(string.Join("\n", errs)); else ShowInfo(T("apply") + " ✓");
        })) return;
        Reload();
    }

    // ---- page construction (rebuilt on every reload) ----
    private void BuildPlantPages()
    {
        if (_file is null || _project is null) return;
        try { _summary = Api.Invoke("results_summary", new { file = _file }); } catch { _summary = null; }
        try { _stringing = Api.Invoke("stringing", new { file = _file }); } catch { _stringing = null; }
        try { _cash = Api.Invoke("cash_flow", new { file = _file }); } catch { _cash = null; }
        BuildWorkflowPanel();
        BuildGridPanel();
        BuildElectrical();
        BuildLoads();
        BuildDemandAndIsland();
        BuildStorage();
        BuildEngineering();
        BuildSimulationPanels();
        BuildPvsolComparison();
        BuildEconomics();
        BuildPreview();
        BuildScene();
    }

    private void BuildPvsolComparison()
    {
        PvsolComparisonPanel.Children.Clear();
        var data = Api.Invoke("compare_pvsol", new { file = _file });
        if (data?["reference"] is not JsonObject reference) return;
        PvsolComparisonPanel.Children.Add(H(T("pvsol_reference")));
        PvsolComparisonPanel.Children.Add(Note($"{S(reference,"source_name")} · {T("pvsol_format")} {N(reference,"format")}"));
        var details = new StackPanel { Spacing = 6 };
        PvsolComparisonPanel.Children.Add(new Expander { Header = T("pvsol_scope"), Content = details });
        foreach (var warning in Arr(reference["warnings"]))
            details.Children.Add(Note(warning?.GetValue<string>() ?? ""));
        if (data["comparison"] is not JsonObject comparison)
        {
            var channels = reference["channels"] as JsonObject;
            if (channels is null || channels.Count == 0)
                PvsolComparisonPanel.Children.Add(Note(T("pvsol_no_saved_results")));
            else
            {
                foreach (var (key, channel) in channels)
                    if (channel?["annual_kwh"] is not null)
                        PvsolComparisonPanel.Children.Add(Note($"{T(key == "pv_dc" ? "pvsol_dc" : "pvsol_ac")} · {F(N(channel, "annual_kwh"), 2)} kWh"));
                PvsolComparisonPanel.Children.Add(Note(T("pvsol_run_to_compare")));
            }
            return;
        }
        if (comparison.Count == 0) PvsolComparisonPanel.Children.Add(Note(T("pvsol_no_saved_results")));
        if (data["imported_input_hash_matches"]?.GetValue<bool>() == false)
            PvsolComparisonPanel.Children.Add(Note(T("pvsol_edited_design")));
        string Cell(JsonNode? row, string key, int digits = 2) =>
            row?[key] is null ? "—" : F(N(row, key), digits);
        foreach (var (key, row) in comparison)
        {
            var title = T(key == "pv_dc" ? "pvsol_dc" : "pvsol_ac");
            var track = new StackPanel { Spacing = 6 };
            var months = Arr(row?["monthly"]);
            if (months.Count > 0)
                track.Children.Add(Table([T("pvsol_month"), "PV*SOL (kWh)", "SolarCraft (kWh)", "Δ (kWh)"],
                    months.Select(m => (IList<string>)new[] { Cell(m,"month",0), Cell(m,"pvsol_kwh"),
                        Cell(m,"solarcraft_kwh"), Cell(m,"delta_kwh") })));
            if (row?["hourly"] is JsonObject hourly)
                track.Children.Add(Note($"{T("pvsol_hourly_error")} {Cell(hourly,"mae_kwh",3)} kWh · {S(hourly,"source")}"));
            PvsolComparisonPanel.Children.Add(new Expander { Header = $"{title} · PV*SOL {Cell(row,"pvsol_kwh")} → SolarCraft {Cell(row,"solarcraft_kwh")} kWh · Δ {Cell(row,"delta_pct")}%", Content = track });
        }
    }

    private void BuildEngineering()
    {
        CablesPage.Children.Clear();
        PlansPage.Children.Clear();
        CablesPage.Children.Add(Note(T("workflow_cable_hint")));
        var losses = _project?["losses"];
        var dc = Num(T("workflow_dc_loss"), Api.Num(losses?["dc_wiring_pct"]));
        var ac = Num(T("workflow_ac_loss"), Api.Num(losses?["ac_wiring_pct"]));
        foreach (var box in new[] { dc, ac }) { box.Minimum = 0; box.Maximum = 99.999; }
        CablesPage.Children.Add(Row(dc, ac));
        CablesPage.Children.Add(Btn(T("apply"), () =>
        {
            if (Try(() => Api.Invoke("set_fields", new { file = _file, edits = new[] {
                new { pointer = "/losses/dc_wiring_pct", value = (object)dc.Value },
                new { pointer = "/losses/ac_wiring_pct", value = (object)ac.Value },
                new { pointer = "/hourly_cables", value = (object)new JsonObject() },
            } }))) Reload();
        }, true));
        BuildCablePlanner(CablesPage);
        var schedule = Api.Invoke("design_schedule", new { file = _file });
        var routes = Arr(schedule?["routes"]);
        if (routes.Count > 0)
        {
            CablesPage.Children.Add(Table([T("photo_array"), T("workflow_string"), T("module"), T("workflow_jumper"), T("workflow_home_run")],
                routes.Select(r => (IList<string>)new[] { S(r, "field"), S(r, "id"), F(N(r, "modules"), 0), F(N(r, "jumper_m"), 1),
                    r?["home_run_m"] is null ? "–" : F(N(r, "home_run_m"), 1) }).ToList()));
        }
        else CablesPage.Children.Add(Note(T("workflow_no_routes")));
        PlansPage.Children.Add(H(T("workflow_parts")));
        PlansPage.Children.Add(Table([T("family"), T("model"), T("workflow_quantity")], Arr(schedule?["equipment"]).Select(row =>
            (IList<string>)new[] { T(S(row, "kind")), S(row, "model"), F(N(row, "quantity"), 0) }).ToList()));
        var planLinks = Row();
        if (Steps.Contains(NavRoof)) planLinks.Children.Add(Btn(T("roof"), () => Nav.SelectedItem = NavRoof));
        if (Steps.Contains(NavScene)) planLinks.Children.Add(Btn(T("scene3d"), () => Nav.SelectedItem = NavScene));
        planLinks.Children.Add(Btn(T("workflow_presentation"), () => Nav.SelectedItem = NavReport));
        PlansPage.Children.Add(planLinks);
        foreach (var array in Arr(_project?["arrays"]).Where(array => array?["roof"] is not null))
        {
            var canvas = new Canvas { Width = 560, Height = 245, HorizontalAlignment = HorizontalAlignment.Left };
            var count = DrawRoof(canvas, array?["roof"]);
            PlansPage.Children.Add(H(Api.Str(array?["name"])));
            PlansPage.Children.Add(canvas);
            PlansPage.Children.Add(Note($"{count} {T("modules_fit")}"));
        }
    }

    private void BuildGridPanel()
    {
        GridPanel.Children.Clear();
        var g = _project!["grid"];
        GridPanel.Children.Add(H(T(Islanded ? "island_bus" : "grid")));
        var feed = Combo(T("feed_in"), new[] { ("surplus", T("surplus")), ("full", T("full_feed_in")) }, S(g, "feed_in"));
        var limit = Num(T("export_limit"), g?["export_limit_kw"] is null ? double.NaN : N(g, "export_limit_kw"));
        var phases = Combo(T("phases"), new[] { ("1", "1"), ("3", "3") }, ((int)N(g, "phases", 1)).ToString(CultureInfo.InvariantCulture));
        var volt = Num(T("voltage"), N(g, "voltage_v", 230));
        if (SystemType == "custom") GridPanel.Children.Add(feed);
        if (!Islanded) GridPanel.Children.Add(limit);
        GridPanel.Children.Add(phases); GridPanel.Children.Add(volt);
        GridPanel.Children.Add(Btn(T("apply"), () => SetSection("grid", new JsonObject
        {
            ["feed_in"] = Sel(feed), ["export_limit_kw"] = double.IsNaN(limit.Value) ? null : (JsonNode)limit.Value, ["phases"] = int.Parse(Sel(phases), CultureInfo.InvariantCulture), ["voltage_v"] = volt.Value,
        })));
    }

    // Stringing is edited as text per MPPT: "array-1: 7x2 + array-2: 6x1" (modules × strings).
    private void BuildElectrical()
    {
        var p = ElectricalPage; p.Children.Clear();
        p.Children.Add(H(T("stringing")));
        var manual = new ToggleSwitch { Header = T("manual_stringing"), IsOn = _stringing?["manual"]?.GetValue<bool>() == true };
        p.Children.Add(manual);
        p.Children.Add(Note("MPPT: array-1: 7x2 + array-2: 6x1  (" + T("modules_per_string_short") + " x " + T("strings_short") + ")"));
        var inverters = Arr(_project!["library"]?["inverters"]);
        var invItems = inverters.Select(i => (S(i, "id"), $"{S(i, "manufacturer")} {S(i, "model")}")).ToList();
        var editors = new List<(TextBox name, ComboBox inv, List<(TextBox text, JsonArray? archived)> mppts, JsonNode unit)>();
        foreach (var entry in Arr(_stringing?["inverters"]))
        {
            var u = entry!["unit"];
            var name = new TextBox { Header = T("inverter"), Text = S(u, "name"), Width = 320, HorizontalAlignment = HorizontalAlignment.Left };
            var inv = Combo("", invItems, S(u, "inverter_id"));
            var mpptBoxes = new List<(TextBox, JsonArray?)>();
            var count = Math.Max(Arr(u?["mppts"]).Count, (int)N(inverters.FirstOrDefault(i => S(i, "id") == S(u, "inverter_id")), "mppt_count", 1));
            var panel = new StackPanel { Spacing = 4 };
            panel.Children.Add(Row(name, inv));
            for (int m = 0; m < count; m++)
            {
                var groups = m < Arr(u?["mppts"]).Count ? Arr(u!["mppts"]![m]) : new JsonArray();
                var advanced = groups.Any(g => g?["series_chain"] is not null || g?["optimizer"] is not null);
                var text = groups.Select(g =>
                {
                    var dimensions = $"{S(g, "array_id")}: {N(g, "modules_per_string")}x{N(g, "strings")}";
                    if (g?["series_chain"] is JsonNode chain) dimensions += $" · {T("series_chain")} {chain.GetValue<int>() + 1}";
                    if (g?["optimizer"] is JsonNode optimizer) dimensions += $" · {S(optimizer, "model")}";
                    return dimensions;
                });
                var tb = new TextBox { Header = $"MPPT {m + 1}", Width = 520, HorizontalAlignment = HorizontalAlignment.Left,
                    Text = string.Join(" + ", text), IsReadOnly = advanced, TextWrapping = TextWrapping.Wrap };
                mpptBoxes.Add((tb, advanced ? groups : null)); panel.Children.Add(tb);
            }
            p.Children.Add(panel);
            editors.Add((name, inv, mpptBoxes, u!));
        }
        p.Children.Add(Btn(T("apply"), () =>
        {
            if (!manual.IsOn) { Try(() => Api.Invoke("set_stringing", new { file = _file, manual = false })); Reload(); return; }
            var units = new JsonArray();
            foreach (var (name, inv, mppts, unit) in editors)
            {
                var mp = new JsonArray();
                foreach (var (tb, archived) in mppts)
                {
                    if (archived is not null) { mp.Add(archived.DeepClone()); continue; }
                    var groups = new JsonArray();
                    foreach (var part in tb.Text.Split('+', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
                    {
                        var kv = part.Split(':', StringSplitOptions.TrimEntries);
                        var dims = (kv.Length > 1 ? kv[1] : kv[0]).Split('x', StringSplitOptions.TrimEntries);
                        if (dims.Length != 2 || !int.TryParse(dims[0], out var mods) || !int.TryParse(dims[1], out var strs)) { ShowError($"MPPT: '{part}'"); return; }
                        groups.Add(new JsonObject { ["array_id"] = kv.Length > 1 ? kv[0] : "array-1", ["modules_per_string"] = mods, ["strings"] = strs });
                    }
                    mp.Add(groups);
                }
                units.Add(new JsonObject { ["id"] = S(unit, "id"), ["name"] = name.Text, ["inverter_id"] = Sel(inv),
                    ["mppts"] = mp, ["interconnected"] = unit["interconnected"]?.DeepClone() ?? new JsonArray() });
            }
            Try(() => Api.Invoke("set_stringing", new JsonObject { ["file"] = _file, ["manual"] = true, ["inverters"] = units }));
            Reload();
        }, true));
        foreach (var i in Arr(_stringing?["issues"]))
        {
            var err = S(i, "severity") == "error";
            p.Children.Add(new TextBlock { Text = (err ? "✖ " : "⚠ ") + S(i, "message"), TextWrapping = TextWrapping.Wrap, Foreground = new SolidColorBrush(err ? Colors.Firebrick : Colors.DarkOrange) });
        }

        p.Children.Add(H(T("equipment")));
        var lib = _project!["library"];
        var rows = new List<IList<string>>();
        foreach (var kind in new[] { "modules", "inverters", "batteries", "battery_inverters" })
            foreach (var r in Arr(lib?[kind])) rows.Add(new[] { $"{S(r, "manufacturer")} {S(r, "model")}", kind switch { "batteries" => "battery", "battery_inverters" => "battery inverter", _ => kind.TrimEnd('s') }, S(r, "provenance") });
        p.Children.Add(Table(new[] { "", "", "" }, rows));
        p.Children.Add(Row(Btn(T("new_module"), () => _ = NewEquipment("module")), Btn(T("new_inverter"), () => _ = NewEquipment("inverter")), Btn(T("new_battery"), () => _ = NewEquipment("battery"))));

        p.Children.Add(H(T("catalog")));
        var fam = Combo("", new[] { "module", "pvinverter", "batterysystem", "batteryinverter", "generator", "demandtariff", "netmeteringtariff", "feedintariff" }.Select(x => (x, x)).ToList(), "module");
        var cq = new TextBox { Width = 260 };
        var hits = new StackPanel { Spacing = 4 };
        void CatSearch()
        {
            hits.Children.Clear();
            JsonArray found;
            try { found = Arr(Api.Invoke("catalog_search", new { family = Sel(fam), query = cq.Text, limit = 50 })?["hits"]); } catch (Exception ex) { ShowError(ex.Message); return; }
            foreach (var h in found)
            {
                var f = S(h, "family"); var id = S(h, "id");
                var info = new TextBlock { Text = $"{S(h, "name")} — {S(h, "company")} {S(h, "country")} · v{N(h, "version")} · {N(h, "versions")} {T("revisions")} · {h?["summary"]}", Width = 640, TextWrapping = TextWrapping.Wrap };
                var act = f.EndsWith("tariff")
                    ? Btn(f == "feedintariff" ? T("use_as_export") : T("use_as_import"), () => { if (Try(() => { var side = f == "feedintariff" ? "export" : "import"; var r = Api.Invoke("catalog_apply_tariff", new JsonObject { ["file"] = _file, [side] = new JsonObject { ["family"] = f, ["id"] = id } }); ShowInfo(string.Join("; ", Arr(r?["applied"]?[0]?["notes"]).Select(x => x?.ToString()))); })) Reload(); })
                    : Btn(T("use_in_project"), () => { if (Try(() => Api.Invoke("catalog_pin", new { file = _file, family = f, id }))) Reload(); });
                hits.Children.Add(Row(info, act));
            }
        }
        p.Children.Add(Row(fam, cq, Btn(T("search"), CatSearch), Btn(T("import_catalog"), async () =>
        {
            var picker = new FileOpenPicker(); picker.FileTypeFilter.Add(".sqlite"); picker.FileTypeFilter.Add("*"); InitPicker(picker);
            var f = await picker.PickSingleFileAsync();
            if (f is not null) { var r = await RunLong("catalog_import", new { source = f.Path }); if (r) ShowInfo(T("catalog") + " ✓"); }
        })));
        p.Children.Add(hits);

        p.Children.Add(H(T("my_library")));
        var query = new TextBox { Width = 260 };
        var list = new StackPanel { Spacing = 4 };
        void Fill()
        {
            list.Children.Clear();
            try { _libraryItems = Arr(Api.Invoke("library_list", new { query = query.Text })?["items"]); } catch { _libraryItems = new JsonArray(); }
            foreach (var it in _libraryItems)
            {
                var rec = it!["record"];
                var kind = S(it, "kind"); var id = S(it, "id");
                list.Children.Add(Row(new TextBlock { Text = $"{kind} · {S(rec, "manufacturer")} {S(rec, "model")} · rev {N(it, "rev")}", Width = 480, VerticalAlignment = VerticalAlignment.Center },
                    Btn(T("use_in_project"), () => { if (Try(() => Api.Invoke("library_pin", new { file = _file, kind, id }))) Reload(); })));
            }
        }
        p.Children.Add(Row(query, Btn("🔍", Fill)));
        p.Children.Add(list);
        Fill();
    }

    private async Task NewEquipment(string kind)
    {
        (string key, string label, double def)[] fields = kind switch
        {
            "module" => new[] { ("cells_in_series", "Cells in series", 72.0), ("length_m", "Length (m)", 1.76), ("width_m", "Width (m)", 1.13), ("v_oc", "Voc (V)", 49.6), ("i_sc", "Isc (A)", 11.6), ("v_mp", "Vmp (V)", 41.4), ("i_mp", "Imp (A)", 10.87), ("alpha_sc", "α Isc (A/K)", 0.0058), ("beta_voc", "β Voc (V/K)", -0.134), ("gamma_pmp", "γ Pmp (%/K)", -0.35) },
            "inverter" => new[] { ("pac_max_w", "Pac max (W)", 5000.0), ("eta_nominal", "Efficiency", 0.97), ("mppt_v_min", "MPPT V min", 120.0), ("mppt_v_max", "MPPT V max", 500.0), ("v_dc_max", "V DC max", 600.0), ("mppt_count", "MPPT inputs", 2.0), ("i_max_per_mppt", "I max per MPPT (A)", 16.0), ("night_w", "Night (W)", 2.0), ("port_v_min", "Battery port V min (0 = none)", 150.0), ("port_v_max", "Battery port V max", 500.0), ("port_p", "Battery port power (W)", 5000.0) },
            "generator" => new[] { ("rated_va", T("generator_rated"), 3000.0), ("max_va", T("generator_max"), 3000.0),
                ("fuel_rate", T("generator_fuel_rate"), 1.0), ("voltage_v", T("voltage"), N(_project?["grid"], "voltage_v", 230)),
                ("phases", T("phases"), N(_project?["grid"], "phases", 1)) },
            _ => new[] { ("capacity_kwh", "Capacity (kWh)", 10.0), ("nominal_voltage_v", "Nominal voltage (V)", 204.8), ("soc_min_pct", "SOC min (%)", 5.0), ("soc_max_pct", "SOC max (%)", 100.0), ("p_charge_max_kw", "Charge power (kW)", 5.0), ("p_discharge_max_kw", "Discharge power (kW)", 5.0), ("roundtrip_efficiency", "Round-trip efficiency", 0.95), ("self_discharge_pct_per_month", "Self-discharge (%/month)", 2.0), ("cycle_life", "Cycles to 80 %", 6000.0) },
        };
        var panel = new StackPanel { Spacing = 6 };
        var id = new TextBox { Header = "id", Text = $"user-{kind}-{DateTime.Now:HHmmss}" };
        var man = new TextBox { Header = "manufacturer", Text = "User" };
        var model = new TextBox { Header = "model", Text = kind };
        panel.Children.Add(id); panel.Children.Add(man); panel.Children.Add(model);
        var boxes = fields.Select(f => (f.key, box: Num(f.label, f.def, 260))).ToList();
        foreach (var (_, box) in boxes) panel.Children.Add(box);
        var fuelUnit = new TextBox { Header = T("generator_fuel_unit"), Text = "L" };
        if (kind == "generator") panel.Children.Add(fuelUnit);
        var save = new CheckBox { Content = T("save_to_library"), IsChecked = true };
        panel.Children.Add(save);
        var dlg = new ContentDialog { Title = T("new_" + kind), Content = new ScrollViewer { Content = panel, MaxHeight = 560 }, PrimaryButtonText = T("apply"), CloseButtonText = T("cancel"), XamlRoot = Content.XamlRoot };
        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;
        var v = boxes.ToDictionary(b => b.key, b => b.box.Value);
        var rec = new JsonObject { ["id"] = id.Text, ["manufacturer"] = man.Text, ["model"] = model.Text };
        switch (kind)
        {
            case "module":
                rec["technology"] = "c-Si";
                foreach (var (k, x) in v) rec[k] = k == "cells_in_series" ? (JsonNode)(int)x : x;
                break;
            case "inverter":
                foreach (var k in new[] { "pac_max_w", "eta_nominal", "mppt_v_min", "mppt_v_max", "v_dc_max", "i_max_per_mppt", "night_w" }) rec[k] = v[k];
                rec["mppt_count"] = (int)v["mppt_count"];
                if (v["port_v_min"] > 0) rec["battery_port"] = new JsonObject { ["v_min"] = v["port_v_min"], ["v_max"] = v["port_v_max"], ["p_charge_max_w"] = v["port_p"], ["p_discharge_max_w"] = v["port_p"], ["dc_dc_efficiency"] = 0.98 };
                break;
            case "generator":
                foreach (var key in new[] { "rated_va", "max_va", "voltage_v" }) rec[key] = v[key];
                rec["phases"] = (int)v["phases"];
                rec["fuel_curve"] = new JsonArray(new JsonArray(1.0, v["fuel_rate"]));
                rec["fuel_unit"] = fuelUnit.Text;
                break;
            default:
                rec["chemistry"] = "lfp";
                foreach (var (k, x) in v) rec[k] = x;
                break;
        }
        if (Try(() =>
        {
            Api.Invoke("add_equipment", new JsonObject { ["file"] = _file, ["kind"] = kind, ["record"] = rec.DeepClone() });
            if (save.IsChecked == true) Api.Invoke("library_save", new JsonObject { ["kind"] = kind, ["record"] = rec.DeepClone() });
        })) { ShowInfo($"{kind} {id.Text} ✓"); Reload(); }
    }

    private void BuildLoads()
    {
        var p = LoadsPage; p.Children.Clear();
        var l = _project!["loads"];
        var prof = l?["profile"];
        var kind = S(prof, "kind");
        p.Children.Add(H(T("loads")));
        var profile = Combo(T("load_profile"), new[] { "none", "residential", "office", "retail", "flat", "series" }.Select(x => (x, T("profile_" + x))).ToList(), kind == "synthetic" ? S(prof, "profile") : kind);
        var annual = Num(T("annual_kwh"), N(l, "annual_kwh", 4000));
        var critical = Num(T("critical_share"), N(l, "critical_fraction", 0.3));
        p.Children.Add(profile); p.Children.Add(annual);
        if (!Islanded) p.Children.Add(critical);
        if (kind == "series") p.Children.Add(Note(S(prof, "name")));
        p.Children.Add(Row(Btn(T("apply"), () =>
        {
            JsonNode pr = Sel(profile) switch { "none" => new JsonObject { ["kind"] = "none" }, "series" => prof?.DeepClone() ?? new JsonObject { ["kind"] = "none" }, var x => new JsonObject { ["kind"] = "synthetic", ["profile"] = x } };
            SetSection("loads", new JsonObject { ["profile"] = pr, ["annual_kwh"] = annual.Value, ["critical_fraction"] = critical.Value });
        }), Btn(T("import_load"), async () =>
        {
            var picker = new FileOpenPicker(); picker.FileTypeFilter.Add(".csv"); picker.FileTypeFilter.Add(".txt"); InitPicker(picker);
            var f = await picker.PickSingleFileAsync();
            if (f is not null && Try(() => Api.Invoke("import_load", new { file = _file, source = f.Path }))) Reload();
        })));

    }

    private void BuildStorage()
    {
        var p = StoragePage; p.Children.Clear();
        var s = _project!["storage"];
        var lib = _project!["library"];
        p.Children.Add(H(T("use_battery")));
        var required = SystemType is "grid_battery" or "grid_ev_battery" or "grid_thermal_battery" or "off_grid" or "off_grid_generator";
        var enabled = new ToggleSwitch { IsOn = required || s is JsonObject, IsEnabled = !required };
        p.Children.Add(enabled);
        if (Arr(lib?["batteries"]).Count == 0) p.Children.Add(Btn(T("add_generic_storage"), () => { if (Try(() => Api.Invoke("add_generic_storage_equipment", new { file = _file }))) Reload(); }));
        var battery = Combo(T("battery"), Arr(lib?["batteries"]).Select(b => (S(b, "id"), $"{S(b, "manufacturer")} {S(b, "model")}")).ToList(), S(s, "battery_id"));
        var units = Num(T("units"), N(s, "units", 1));
        var c = s?["coupling"];
        var coupling = Combo(T("coupling"), new[] { ("dc_hybrid", T("dc_hybrid")), ("ac_coupled", T("ac_coupled")), ("integrated", T("integrated")) }, S(c, "kind") is "" ? "dc_hybrid" : S(c, "kind"));
        var hybrids = Arr(_stringing?["inverters"]).Where(i => i?["has_battery_port"]?.GetValue<bool>() == true).Select(i => (S(i!["unit"], "id"), S(i!["unit"], "name"))).ToList();
        var unit = Combo(T("dc_hybrid"), hybrids, S(c, "unit_id"));
        var batInv = Combo(T("ac_coupled"), Arr(lib?["battery_inverters"]).Select(b => (S(b, "id"), $"{S(b, "manufacturer")} {S(b, "model")}")).ToList(), S(c, "battery_inverter_id"));
        var model = Combo(T("battery_model"), new[] { ("simple", T("model_simple")), ("detailed", T("model_detailed")) }, S(s, "model") is "" ? "simple" : S(s, "model"));
        var socMin = Num(T("soc_window") + " min", N(s, "soc_min_pct", 10)); var socMax = Num("max", N(s, "soc_max_pct", 100)); var init = Num(T("initial_soc"), N(s, "initial_soc_pct", 50));
        var backup = Num(T("backup_reserve"), N(s, "backup_reserve_pct", 20)); var services = Num(T("grid_services"), N(s, "grid_services_pct", 0));
        var gs = s?["grid_service"];
        var gsKind = Combo(T("grid_service"), new[] { ("none", T("gs_none")), ("peak_shaving", T("gs_peak")), ("export_window", T("gs_window")) }, S(gs, "kind") is "" ? "none" : S(gs, "kind"));
        var gsA = Num("kW / " + T("hour"), N(gs, "import_limit_kw", N(gs, "start_hour", 18))); var gsB = Num(T("hour") + " →", N(gs, "end_hour", 21)); var gsC = Num("kW", N(gs, "power_kw", 3));
        var gridCharge = new CheckBox { Content = T("grid_charge_backup"), IsChecked = s?["grid_charge_backup"]?.GetValue<bool>() == true };
        var outages = new TextBox { Header = T("outages") + " (" + T("day") + "," + T("hour") + "," + T("hours_n") + "; …)", Width = 520, HorizontalAlignment = HorizontalAlignment.Left,
            Text = string.Join("; ", Arr(s?["outages"]).Select(o => $"{(int)N(o, "start_hour") / 24 + 1},{(int)N(o, "start_hour") % 24},{N(o, "hours")}")) };
        foreach (var el in new UIElement[] { battery, units, coupling, unit, batInv, model, Row(socMin, socMax), init }) p.Children.Add(el);
        // Allocation bar drawn to scale.
        if (!Islanded) p.Children.Add(H(T("allocation")));
        var bar = new Grid { Height = 18, Width = 520, HorizontalAlignment = HorizontalAlignment.Left, CornerRadius = new CornerRadius(4) };
        void DrawBar()
        {
            bar.Children.Clear(); bar.ColumnDefinitions.Clear();
            var b = Math.Clamp(backup.Value, 0, 100); var g = Math.Clamp(services.Value, 0, 100 - b); var sc = 100 - b - g;
            var cols = new[] { (b, Colors.DarkOrange), (sc, Colors.Teal), (g, Colors.MediumPurple) };
            for (int i = 0; i < 3; i++)
            {
                bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(Math.Max(cols[i].Item1, 0.0001), GridUnitType.Star) });
                var r = new Rectangle { Fill = new SolidColorBrush(cols[i].Item2) }; Grid.SetColumn(r, i); bar.Children.Add(r);
            }
        }
        DrawBar();
        backup.ValueChanged += (_, _) => DrawBar(); services.ValueChanged += (_, _) => DrawBar();
        if (!Islanded)
        {
            p.Children.Add(bar);
            p.Children.Add(Note($"■ {T("backup_reserve")}   ■ {T("self_consumption")}   ■ {T("grid_services")}"));
        }
        foreach (var el in new UIElement[] { Row(backup, services), Note(T("allocation_note")), gsKind, Row(gsA, gsB, gsC), gridCharge, outages }) {
            if (!Islanded) p.Children.Add(el);
        }
        var genChoices = new List<(string, string)> { ("", T("gs_none")) };
        genChoices.AddRange(Arr(lib?["generators"]).Select(g => (S(g, "id"), $"{S(g, "manufacturer")} {S(g, "model")}")));
        var generator = Combo(T("generator"), genChoices, S(s, "generator_id"));
        if (SystemType != "off_grid")
        {
            p.Children.Add(generator);
            p.Children.Add(Btn(T("new_generator"), () => _ = NewEquipment("generator")));
        }
        p.Children.Add(Btn(T("apply"), () =>
        {
            if (!enabled.IsOn) { SetSection("storage", null); return; }
            var outs = new JsonArray();
            foreach (var part in outages.Text.Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
            {
                var x = part.Split(',', StringSplitOptions.TrimEntries).Select(t => int.TryParse(t, out var n) ? n : -1).ToArray();
                if (x.Length == 3 && x.All(n => n >= 0)) outs.Add(new JsonObject { ["start_hour"] = (x[0] - 1) * 24 + x[1], ["hours"] = x[2] });
            }
            JsonObject gsv = Sel(gsKind) switch
            {
                "peak_shaving" => new JsonObject { ["kind"] = "peak_shaving", ["import_limit_kw"] = gsA.Value },
                "export_window" => new JsonObject { ["kind"] = "export_window", ["start_hour"] = (int)gsA.Value, ["end_hour"] = (int)gsB.Value, ["power_kw"] = gsC.Value },
                _ => new JsonObject { ["kind"] = "none" },
            };
            JsonObject cp = Sel(coupling) switch {
                "ac_coupled" => new JsonObject { ["kind"] = "ac_coupled", ["battery_inverter_id"] = Sel(batInv) },
                "integrated" => new JsonObject { ["kind"] = "integrated" },
                _ => new JsonObject { ["kind"] = "dc_hybrid", ["unit_id"] = Sel(unit) },
            };
            SetSection("storage", new JsonObject
            {
                ["battery_id"] = Sel(battery), ["units"] = (int)units.Value, ["coupling"] = cp, ["model"] = Sel(model), ["soc_min_pct"] = socMin.Value, ["soc_max_pct"] = socMax.Value, ["initial_soc_pct"] = init.Value,
                ["backup_reserve_pct"] = backup.Value, ["grid_services_pct"] = services.Value, ["charge_priority"] = new JsonArray("backup", "self_consumption", "grid_services"),
                ["grid_charge_backup"] = gridCharge.IsChecked == true, ["grid_service"] = gsv, ["outages"] = outs,
                ["generator_id"] = Sel(generator) is "" ? null : Sel(generator),
            });
        }, true));
        IssuesInto(p, x => x.StartsWith("/storage") || x.StartsWith("/loads") || x.StartsWith("STORAGE") || x.StartsWith("BATTERY"));
    }

    private void BuildSimulationPanels()
    {
        var p = PlantResults; p.Children.Clear();
        if (_summary is null) return;
        var b = _summary["balance"];
        var kpis = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        var metrics = Islanded
            ? new[] { (T("island_results"), F(N(b, "load_served_kwh"), 1), "kWh"), (T("island_unserved"), F(N(b, "unserved_kwh"), 1), "kWh"), (T("generator"), F(N(b, "generator_kwh"), 1), "kWh") }
            : new[] { (T("self_consumption"), F(N(b, "self_consumption_rate") * 100, 1), "%"), (T("autarky"), F(N(b, "autarky") * 100, 1), "%"), (T("grid_import"), F(N(b, "import_kwh")), "kWh"), (T("grid_export"), F(N(b, "export_kwh")), "kWh") };
        foreach (var (label, value, unit) in metrics)
        {
            var sp = new StackPanel { Padding = new Thickness(12), CornerRadius = new CornerRadius(6), Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"], MinWidth = 160 };
            sp.Children.Add(new TextBlock { Text = label, Opacity = 0.7 });
            sp.Children.Add(new TextBlock { Text = $"{value} {unit}", Style = (Style)Application.Current.Resources["SubtitleTextBlockStyle"] });
            kpis.Children.Add(sp);
        }
        p.Children.Add(kpis);
        p.Children.Add(H(T("energy_balance")));
        p.Children.Add(Table(Array.Empty<string>(), new List<IList<string>>
        {
            new[] { "PV DC", F(N(b, "pv_dc_kwh")) + " kWh" }, new[] { T("grid_import"), F(N(b, "import_kwh")) + " kWh" }, new[] { T("loads"), F(N(b, "load_served_kwh")) + " kWh" },
            new[] { T("grid_export"), F(N(b, "export_kwh")) + " kWh" }, new[] { T("curtailed"), F(N(b, "curtailed_kwh")) + " kWh" }, new[] { T("clipped"), F(N(b, "clipped_kwh")) + " kWh" },
            new[] { T("batt_charge") + " / " + T("batt_discharge"), F(N(b, "batt_charge_kwh")) + " / " + F(N(b, "batt_discharge_kwh")) + " kWh" },
            new[] { "Δ max", N(b, "max_abs_residual_wh").ToString("0.0e0", CultureInfo.InvariantCulture) + " Wh" },
        }));
        p.Children.Add(H(T("stringing")));
        var rows = new List<IList<string>>();
        foreach (var u in Arr(_summary["electrical"]))
        {
            rows.Add(new[] { S(u, "name"), F(N(u, "stc_kw"), 2), F(N(u, "dc_kwh")), "", "", "" });
            foreach (var m in Arr(u?["mppts"])) rows.Add(new[] { $"  MPPT {N(m, "index")} · {S(m, "strings")}", F(N(m, "stc_kw"), 2), F(N(m, "dc_kwh")), F(N(m, "mismatch_kwh"), 1), F(N(m, "limit_loss_kwh"), 1), F(N(m, "limited_hours")) });
        }
        p.Children.Add(Table(new[] { T("inverter") + " / MPPT", "kWp", "DC kWh", "mismatch", "limits", "h" }, rows));

        p.Children.Add(H(T("hours_inspector")));
        var day = Num(T("day"), 172, 160); day.Minimum = 1; day.Maximum = 365;
        var host = new StackPanel { Spacing = 6 };
        void Show(JsonArray hours)
        {
            _dayRows = hours; host.Children.Clear();
            if (hours.Count == 0) return;
            // Lines: PV (gold), load (blue), import (red), battery net (teal).
            var canvas = new Canvas { Width = 720, Height = 170, Background = new SolidColorBrush(ColorHelper.FromArgb(10, 0, 0, 0)) };
            var max = Math.Max(1, hours.Max(h => Math.Max(Math.Max(N(h, "pv_ac_wh"), N(h, "load_wh")), Math.Max(N(h, "import_wh"), Math.Abs(N(h, "batt_discharge_wh") - N(h, "batt_charge_wh"))))));
            foreach (var (key, color) in new[] { ("pv_ac_wh", Colors.Goldenrod), ("load_wh", Colors.SteelBlue), ("import_wh", Colors.Firebrick), ("batt", Colors.Teal) })
            {
                var line = new Polyline { Stroke = new SolidColorBrush(color), StrokeThickness = 2 };
                for (int i = 0; i < hours.Count; i++)
                {
                    var v = key == "batt" ? N(hours[i], "batt_discharge_wh") - N(hours[i], "batt_charge_wh") : N(hours[i], key);
                    line.Points.Add(new Windows.Foundation.Point(10 + 700.0 * i / Math.Max(1, hours.Count - 1), 85 - 80 * v / max));
                }
                canvas.Children.Add(line);
            }
            host.Children.Add(canvas);
            host.Children.Add(Note("— PV   — " + T("loads") + "   — " + T("grid_import") + "   — " + T("battery")));
            host.Children.Add(Table(new[] { T("day"), T("hour"), "PV", T("loads"), "→bat", "bat→", T("grid_import"), T("grid_export"), T("soc"), T("backup_reserve") + " Wh" },
                hours.Select(h => (IList<string>)new[] { F(N(h, "day_of_year")), $"{(int)N(h, "hour"):00}:00" + (h?["outage"]?.GetValue<bool>() == true ? " ⚡" : ""), F(N(h, "pv_ac_wh")), F(N(h, "load_wh")), F(N(h, "batt_charge_wh")), F(N(h, "batt_discharge_wh")), F(N(h, "import_wh")), F(N(h, "export_wh")), F(N(h, "soc_pct"), 1), F(N(h, "backup_wh")) }).ToList(), 12));
            var details = new[] {
                ("ev_wh", T("ev_page")), ("heating_wh", T("heat_page")), ("dc_cable_wh", "DC cable (Wh)"),
                ("ac_cable_wh", "AC cable (Wh)"), ("generator_charge_wh", T("island_charge")),
                ("ev_to_load_wh", "V2H (Wh)"), ("ev_export_wh", "V2G (Wh)"), ("heat_store_kwh", T("tank") + " (kWh)"),
                ("tank_top_c", T("tank_top")), ("tank_bottom_c", T("tank_bottom")),
            }.Where(column => hours.Any(h => Math.Abs(N(h, column.Item1)) > 1e-7)).ToArray();
            if (details.Length > 0)
            {
                host.Children.Add(Table(new[] { T("day"), T("hour") }.Concat(details.Select(d => d.Item2)).ToList(),
                    hours.Select(h => (IList<string>)new[] { F(N(h, "day_of_year")), $"{(int)N(h, "hour"):00}:00" }
                        .Concat(details.Select(d => F(N(h, d.Item1), 2))).ToList()), 12));
            }
            var series = new List<(string label, string field, int column)>();
            void AddSeries(JsonArray entities, string title, string labelKey, string field, string unit)
            {
                for (var i = 0; i < entities.Count; i++)
                    series.Add(($"{title} · {S(entities[i], labelKey)} ({unit})", field, i));
            }
            AddSeries(Arr(_summary?["fleet"]?["groups"]), T("fleet"), "name", "fleet_soc_pct", "%");
            AddSeries(Arr(_summary?["clusters"]), T("clusters"), "name", "cluster_soc_pct", "%");
            AddSeries(Arr(_summary?["clusters"]), T("island_unserved"), "name", "cluster_unserved_wh", "Wh");
            AddSeries(Arr(_summary?["clusters"]), T("loads"), "name", "cluster_load_wh", "Wh");
            AddSeries(Arr(_summary?["routed_cables"]), T("cable_planner"), "label", "routed_cable_wh", "Wh");
            if (series.Count > 0)
            {
                var picker = Combo(T("hours_inspector"), series.Select((s, i) => (i.ToString(), s.label)).ToList(), "0");
                var selected = new StackPanel();
                host.Children.Add(picker);
                host.Children.Add(selected);
                void ShowSeries()
                {
                    if (picker.SelectedIndex < 0) return;
                    var c = series[picker.SelectedIndex];
                    selected.Children.Clear();
                    selected.Children.Add(Table([T("day"), T("hour"), c.label], hours.Select(h => (IList<string>)new[] {
                        F(N(h,"day_of_year")), $"{(int)N(h,"hour"):00}:00",
                        F(Arr(h?[c.field]).ElementAtOrDefault(c.column)?.GetValue<double>() ?? 0, 2),
                    }), 12));
                }
                picker.SelectionChanged += (_, _) => ShowSeries();
                ShowSeries();
            }
        }
        var buttons = new List<UIElement> { day, Btn(T("apply"), () => Try(() => Show(Arr(Api.Invoke("results_hours", new { file = _file, day_of_year = (int)day.Value })?["hours"])))) };
        foreach (var ch in new[] { "import", "export", "clipped", "curtailed", "unserved", "-soc" })
            buttons.Add(Btn(ch, () => Try(() => Show(Arr(Api.Invoke("results_hours", new { file = _file, top = ch, count = 24 })?["hours"])))));
        p.Children.Add(Row(buttons.ToArray()));
        p.Children.Add(host);
        if (_dayRows is not null) Show(_dayRows);
    }

    private void BuildEconomics()
    {
        var p = EconomicsPage; p.Children.Clear();
        var t = _project!["tariff"]; var e = _project!["economics"];
        p.Children.Add(H(T("tariff")));
        var currency = new TextBox { Header = T("currency"), Text = S(t, "currency"), Width = 120, HorizontalAlignment = HorizontalAlignment.Left };
        var imp = Num(T("import_price"), N(t, "import_price"));
        var ex = t?["export"];
        var exKind = Combo(T("grid_export"), new[] { ("fixed", T("export_price")), ("net_metering", T("net_metering")), ("none", T("gs_none")) }, S(ex, "kind") is "" ? "fixed" : S(ex, "kind"));
        var exPrice = Num(T("export_price"), S(ex, "kind") == "net_metering" ? N(ex, "surplus_price") : N(ex, "price"));
        var fixedM = Num(T("fixed_monthly"), N(t, "fixed_per_month")); var demand = Num(T("demand_charge"), N(t, "demand_charge_per_kw_month"));
        var escI = Num(T("escalation") + " ↓", N(t, "import_escalation_pct")); var escE = Num(T("escalation") + " ↑", N(t, "export_escalation_pct"));
        var tou = new TextBox { Header = T("tou_periods") + " (name,price,start,end; …)", Width = 520, HorizontalAlignment = HorizontalAlignment.Left,
            Text = string.Join("; ", Arr(t?["tou"]).Select(x => string.Format(CultureInfo.InvariantCulture, "{0},{1},{2},{3}", S(x, "name"), N(x, "price"), N(x, "start_hour"), N(x, "end_hour")))) };
        foreach (var el in new UIElement[] { currency, imp, Row(exKind, exPrice), Row(fixedM, demand), Row(escI, escE), tou }) p.Children.Add(el);
        p.Children.Add(Btn(T("apply"), () =>
        {
            var nt = t?.DeepClone() as JsonObject ?? new JsonObject { ["name"] = "Tariff" };
            nt["currency"] = currency.Text; nt["import_price"] = imp.Value; nt["fixed_per_month"] = fixedM.Value; nt["demand_charge_per_kw_month"] = demand.Value;
            nt["import_escalation_pct"] = escI.Value; nt["export_escalation_pct"] = escE.Value;
            nt["export"] = Sel(exKind) switch { "none" => new JsonObject { ["kind"] = "none" }, "net_metering" => new JsonObject { ["kind"] = "net_metering", ["surplus_price"] = exPrice.Value }, _ => new JsonObject { ["kind"] = "fixed", ["price"] = exPrice.Value } };
            var periods = new JsonArray();
            foreach (var part in tou.Text.Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
            {
                var x = part.Split(',', StringSplitOptions.TrimEntries);
                if (x.Length == 4 && double.TryParse(x[1], NumberStyles.Float, CultureInfo.InvariantCulture, out var pr) && int.TryParse(x[2], out var a) && int.TryParse(x[3], out var bb))
                    periods.Add(new JsonObject { ["name"] = x[0], ["price"] = pr, ["start_hour"] = a, ["end_hour"] = bb, ["weekdays"] = true, ["weekends"] = true, ["months"] = new JsonArray() });
            }
            nt["tou"] = periods;
            SetSection("tariff", nt);
        }, true));

        p.Children.Add(H(T("costs")));
        var fields = new[] { ("horizon_years", "horizon"), ("discount_rate_pct", "discount_rate"), ("cost_fixed", "cost_fixed"), ("cost_per_kwp", "cost_per_kwp"), ("cost_per_battery_kwh", "cost_per_kwh"), ("om_pct_of_investment", "om"), ("degradation_pct_per_year", "degradation") };
        var boxes = fields.Select(f => (f.Item1, box: Num(T(f.Item2), N(e, f.Item1)))).ToList();
        foreach (var (_, box) in boxes) p.Children.Add(box);
        var loan = e?["loan"];
        var loanOn = new ToggleSwitch { Header = T("loan"), IsOn = loan is JsonObject };
        var la = Num(T("loan"), N(loan, "amount")); var lr = Num("%", N(loan, "rate_pct", 5), 120); var ly = Num("a", N(loan, "years", 10), 120);
        p.Children.Add(loanOn); p.Children.Add(Row(la, lr, ly));
        p.Children.Add(Btn(T("apply"), () =>
        {
            var ne = e?.DeepClone() as JsonObject ?? new JsonObject();
            foreach (var (k, box) in boxes) ne[k] = k == "horizon_years" ? (JsonNode)(int)box.Value : box.Value;
            ne["loan"] = loanOn.IsOn ? new JsonObject { ["amount"] = la.Value, ["rate_pct"] = lr.Value, ["years"] = (int)ly.Value } : null;
            SetSection("economics", ne);
        }, true));

        if (_cash is JsonObject c && Arr(c["years"]).Count > 0)
        {
            p.Children.Add(H(T("cash_flow")));
            p.Children.Add(Note($"{T("npv")} {F(N(c, "npv"))} {S(c, "currency")} · {T("irr")} {(c["irr_pct"] is null ? "–" : F(N(c, "irr_pct"), 1) + " %")} · {T("payback")} {(c["payback_years"] is null ? "–" : F(N(c, "payback_years"), 1))} · LCOE {F(N(c, "lcoe_per_kwh"), 3)}"));
            var years = Arr(c["years"]);
            var max = Math.Max(1, years.Max(y => Math.Abs(N(y, "cumulative"))));
            var bars = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 3, Height = 140 };
            foreach (var y in years)
            {
                var v = N(y, "cumulative");
                var col = new Grid { Width = 22, Height = 140 };
                col.Children.Add(new Rectangle { Height = 65 * Math.Abs(v) / max, VerticalAlignment = v >= 0 ? VerticalAlignment.Top : VerticalAlignment.Bottom, Margin = v >= 0 ? new Thickness(0, 70 - 65 * Math.Abs(v) / max, 0, 0) : new Thickness(0, 0, 0, 70 - 65 * Math.Abs(v) / max), Fill = new SolidColorBrush(v >= 0 ? Colors.SeaGreen : Colors.IndianRed) });
                bars.Children.Add(col);
            }
            p.Children.Add(bars);
            p.Children.Add(Table(new[] { "", "savings", "export", "O&M", "repl.", T("loan"), T("cash_flow"), "Σ" },
                years.Select(y => (IList<string>)new[] { F(N(y, "year")), F(N(y, "savings")), F(N(y, "export_revenue")), F(N(y, "om")), F(N(y, "replacements")), F(N(y, "loan")), F(N(y, "cash_flow")), F(N(y, "cumulative")) }).ToList(), 12));
        }

        p.Children.Add(H(T("variants")));
        var vname = new TextBox { Width = 260, PlaceholderText = T("save_variant") };
        p.Children.Add(Row(vname, Btn(T("save_variant"), () => { if (Try(() => Api.Invoke("create_variant", new { file = _file, name = vname.Text.Length > 0 ? vname.Text : "Variant" }))) Reload(); }),
            Btn(T("compare"), async () => { var f = _file; try { _compareRows = Arr((await Task.Run(() => Api.Invoke("compare_variants", new { file = f })))?["variants"]); } catch (Exception ex2) { ShowError(ex2.Message); } BuildEconomics(); })));
        foreach (var v in Arr(_project!["variants"]))
        {
            var id = S(v, "id");
            p.Children.Add(Row(new TextBlock { Text = S(v, "name"), Width = 300, VerticalAlignment = VerticalAlignment.Center }, Btn(T("activate"), () => { if (Try(() => Api.Invoke("apply_variant", new { file = _file, id, keep_current = true }))) Reload(); })));
        }
        if (_compareRows is { Count: > 0 })
            p.Children.Add(Table(new[] { "", "kWp", "kWh bat", "AC kWh", "SC %", T("autarky") + " %", T("grid_import"), T("npv"), T("irr"), T("payback") },
                _compareRows.Select(r => (IList<string>)new[] { S(r, "name"), F(N(r, "dc_kwp"), 2), F(N(r, "battery_kwh"), 1), F(N(r, "ac_kwh")), F(N(r, "self_consumption_pct"), 1), F(N(r, "autarky_pct"), 1), F(N(r, "import_kwh")), F(N(r, "npv")),
                    r?["irr_pct"] is null ? "–" : F(N(r, "irr_pct"), 1), r?["payback_years"] is null ? "–" : F(N(r, "payback_years"), 1) }).ToList(), 12));
    }

    private readonly HashSet<string> _reportExclusions = new();

    private void BuildPreview()
    {
        var p = PreviewPanel; p.Children.Clear();
        ExportButton.IsEnabled = _summary is not null;
        if (_summary is null)
        {
            _previewSections = null;
            return;
        }
        var previewButton = new Button { Content = T("report_preview") };
        previewButton.Click += async (_, _) =>
        {
            var file = _file;
            var lang = (LangBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "en";
            var designer = DesignerBox.Text;
            var reportType = SelectedReportType();
            previewButton.IsEnabled = false;
            try
            {
                var result = await Task.Run(() => Api.Invoke("report_preview", new {
                    file, lang, designer, report_type = reportType, include_figures = true
                }));
                if (file != _file || lang != ((LangBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "en") || reportType != SelectedReportType() || _summary is null) return;
                _previewSections = Arr(result?["contents"]);
                BuildPreview();
            }
            catch (Exception error) { ShowError(error.Message); }
            finally { previewButton.IsEnabled = true; }
        };
        p.Children.Add(previewButton);
        if (_previewSections is null) return;
        p.Children.Add(H(T("report_sections")));
        var details = new StackPanel { Spacing = 8 };
        foreach (var section in _previewSections)
        {
            var id = S(section, "id");
            var label = S(section, "title");
            var content = new StackPanel { Spacing = 6, Visibility = _reportExclusions.Contains(id) ? Visibility.Collapsed : Visibility.Visible };
            var include = new CheckBox { Content = label, IsChecked = !_reportExclusions.Contains(id), MinHeight = 32 };
            include.Checked += (_, _) => { _reportExclusions.Remove(id); content.Visibility = Visibility.Visible; };
            include.Unchecked += (_, _) => { _reportExclusions.Add(id); content.Visibility = Visibility.Collapsed; };
            p.Children.Add(include);
            details.Children.Add(content);
            foreach (var block in Arr(section?["blocks"]))
            {
                switch (S(block, "kind"))
                {
                    case "table":
                        content.Children.Add(Table(Arr(block?["header"]).Select(h => h!.GetValue<string>()).ToList(),
                            Arr(block?["rows"]).Select(r => (IList<string>)Arr(r).Select(c => c!.GetValue<string>()).ToList()).ToList(), 12));
                        break;
                    case "text":
                        content.Children.Add(Note(S(block, "text")));
                        break;
                    case "figure":
                        var caption = S(block, "caption");
                        var data = S(block, "png_base64");
                        if (caption.Length > 0) content.Children.Add(Note(caption));
                        if (data.Length > 0)
                        {
                            var image = new Image { MaxWidth = 700, MaxHeight = 320, HorizontalAlignment = HorizontalAlignment.Left, Stretch = Stretch.Uniform };
                            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(image, caption.Length > 0 ? caption : label);
                            content.Children.Add(image);
                            _ = LoadPreviewFigure(image, data);
                        }
                        break;
                }
            }
        }
        p.Children.Add(details);
    }

    private async Task LoadPreviewFigure(Image image, string data)
    {
        try { image.Source = await PhotoBitmap(data); }
        catch (Exception error) { ShowError(error.Message); }
    }

    private string SelectedReportType() => (ReportTypeBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "customer_extended";

    private void OnReportTypeChanged(object sender, SelectionChangedEventArgs e)
    {
        _previewSections = null;
        _reportExclusions?.Clear();
        if (PreviewPanel is not null) BuildPreview();
    }

    private void OnReportLanguageChanged(object sender, SelectionChangedEventArgs e)
    {
        _previewSections = null;
        if (PreviewPanel is not null) BuildPreview();
    }

    // ---- --screenshots: every page of a simulated demo project ----
    private async Task<bool> CaptureAll(string dir)
    {
        var ok = true;
        try
        {
            Directory.CreateDirectory(dir);
            AppWindow.Resize(new Windows.Graphics.SizeInt32(1400, 1000));
            var file = SelfTest.BuildDemo(IOPath.Combine(dir, "project"));
            Open(file);
            _dayRows = Arr(Api.Invoke("results_hours", new { file, day_of_year = 172 })?["hours"]);
            _compareRows = Arr(Api.Invoke("compare_variants", new { file })?["variants"]);
            _previewSections = Arr(Api.Invoke("report_preview", new { file, lang = "en", report_type = SelectedReportType(), include_figures = true })?["contents"]);
            BuildPlantPages();
            async Task Capture(NavigationViewItem item, string filename, Action? prepare = null)
            {
                Nav.SelectedItem = item;
                await Task.Delay(420);
                if (prepare is not null) { prepare(); await Task.Delay(420); }
                Content.UpdateLayout();
                var rtb = new RenderTargetBitmap();
                await rtb.RenderAsync(Content);
                var pixels = await rtb.GetPixelsAsync();
                using var fs = File.Create(IOPath.Combine(dir, filename));
                var enc = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, fs.AsRandomAccessStream());
                enc.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, (uint)rtb.PixelWidth, (uint)rtb.PixelHeight, 96, 96, pixels.ToArray());
                await enc.FlushAsync();
            }

            var items = Steps;
            for (int i = 0; i < items.Length; i++) await Capture(items[i], $"{i + 1:00}-{items[i].Tag}.png");

            Api.Invoke("workflow_set", new { file, value = new { system = "grid_export", tools = "roof" } });
            Reload();
            if (Steps.Contains(NavLoads) || Steps.Contains(NavStorage) || Steps.Contains(NavScene) || _summary is not null)
                throw new InvalidOperationException("Full-export workflow navigation or result invalidation failed.");
            await Capture(NavSite, "16-full-export.png");
            Api.Invoke("workflow_set", new { file, value = new { system = "off_grid_generator", tools = "scene" } });
            Reload();
            if (!Steps.Contains(NavLoads) || !Steps.Contains(NavStorage) || Steps.Contains(NavRoof) || Steps.Contains(NavEconomics))
                throw new InvalidOperationException("Off-grid workflow navigation failed.");
            await Capture(NavSite, "17-off-grid.png");
            await Capture(NavIsland, "18-island-controls.png");
            Api.Invoke("workflow_set", new { file, value = new { system = "grid_ev_battery", tools = "both" } });
            Api.Invoke("set_section", new { file, section = "ev", value = new { daily_kwh = 12, charger_kw = 7.2, efficiency = 0.92,
                arrival_hour = 18, departure_hour = 7, strategy = "solar_preferred" } });
            Reload();
            if (!Steps.Contains(NavEv) || Steps.Contains(NavHeating) || !Steps.Contains(NavStorage))
                throw new InvalidOperationException("EV workflow navigation failed.");
            await Capture(NavEv, "19-ev-controls.png");
            Api.Invoke("workflow_set", new { file, value = new { system = "grid_thermal", tools = "both" } });
            Api.Invoke("set_section", new { file, section = "heating", value = new { setpoint_c = 20, heat_loss_kw_per_k = 0.18,
                thermal_kw = 6, cutoff_c = -15, cop_at_0c = 2.5, cop_at_10c = 3.8, resistance_backup_kw = 3 } });
            Reload();
            if (Steps.Contains(NavEv) || !Steps.Contains(NavHeating) || Steps.Contains(NavStorage))
                throw new InvalidOperationException("Thermal workflow navigation failed.");
            await Capture(NavHeating, "20-thermal-controls.png");
            Api.Invoke("workflow_set", new { file, value = new { system = "custom", tools = "both" } });

            _roofArray = (int)N(Api.Invoke("add_array", new { file, name = "West roof" }), "array");
            Api.Invoke("set_roof", new { file, array = _roofArray, outline = new[] {
                new[] { 0.0, 0.0 }, new[] { 10.0, 0.0 }, new[] { 10.0, 5.5 }, new[] { 5.0, 5.5 }, new[] { 5.0, 7.0 }, new[] { 0.0, 7.0 }
            } });
            Reload();
            if (PhotoArrayBox.SelectedIndex != _roofArray || RoofCanvas.Children.Count <= 1)
                throw new InvalidOperationException("Second roof did not load in the roof editor.");
            await Capture(NavRoof, "21-second-roof.png");

            var candidates = new[] {
                new { area_mm2 = 2.5, ampacity_a = 16 }, new { area_mm2 = 4.0, ampacity_a = 24 }, new { area_mm2 = 6.0, ampacity_a = 32 },
            };
            var appliedCables = Api.Invoke("cable_plan", new { file, mode = "apply_hourly", circuits = new[] {
                new { label = "West DC", kind = "dc_string", metal = "copper", length_m = 19, area_mm2 = 4, temperature_c = 60,
                    voltage_v = 390, current_a = 10, circuits = 2, target_drop_pct = 1.5, ampacity_a = 24, derating = 0.80, candidates },
                new { label = "Main AC", kind = "ac_single", metal = "copper", length_m = 15, area_mm2 = 6, temperature_c = 60,
                    voltage_v = 230, current_a = 20, circuits = 1, target_drop_pct = 1.5, ampacity_a = 32, derating = 0.85, candidates },
            } });
            if (N(appliedCables?["estimates"]?[0]?["suggested"], "area_mm2") != 2.5 ||
                N(appliedCables?["estimates"]?[1]?["suggested"], "area_mm2") != 4.0)
                throw new InvalidOperationException("Cable sizing did not return the expected cross sections.");
            Reload();
            await Capture(NavCables, "22-hourly-cable-estimates.png");

            Api.Invoke("set_section", new { file, section = "fleet", value = new { site_charge_kw = 7.2, site_discharge_kw = 2,
                v2g = new { limit_kw = 1.2, start_hour = 18, end_hour = 22, weekdays = new[] { 0, 1, 2, 3, 4 } },
                groups = new[] {
                    new { name = "Servicio", vehicles = 2, battery_kwh = 40, initial_soc_pct = 65, departure_soc_pct = 45,
                        trip_kwh = 12, weekdays = new[] { 0, 1, 2, 3, 4 }, arrival_hour = 9, departure_hour = 23,
                        charge_kw = 3.6, charge_efficiency = .92, discharge_kw = 1, discharge_efficiency = .9, strategy = "solar_preferred" },
                    new { name = "Visitas", vehicles = 1, battery_kwh = 55, initial_soc_pct = 55, departure_soc_pct = 40,
                        trip_kwh = 9, weekdays = new[] { 5, 6 }, arrival_hour = 8, departure_hour = 20,
                        charge_kw = 3.6, charge_efficiency = .92, discharge_kw = 1, discharge_efficiency = .9, strategy = "solar_preferred" }
                } } });
            Api.Invoke("set_section", new { file, section = "heat_store", value = new { capacity_kwh = 9, initial_kwh = 1,
                charge_kw = 3, discharge_kw = 3, standing_loss_pct_per_day = 2, daily_hot_water_kwh = 2, hot_water_hour = 21,
                layers = new { top_fraction = 0.4, ambient_c = 20, target_c = 65, space_supply_c = 34, hot_water_supply_c = 50, conductance_w_per_k = 20 },
            } });
            Api.Invoke("simulate", new { file });
            Reload();
            if (Arr(_summary?["fleet"]?["groups"]).Count != 2 || N(_summary?["fleet"], "v2g_kwh") <= 0 ||
                _summary?["heat_store"]?["end_top_c"] is null)
                throw new InvalidOperationException("Fleet or thermal store results did not load.");
            await Capture(NavEv, "23-ev-fleet.png");
            await Capture(NavHeating, "24-heat-store.png");

            var unitIds = Arr(Api.Invoke("stringing", new { file })?["inverters"]).Select(r => S(r?["unit"],"id")).ToArray();
            var baseStorage = _project?["storage"]?.DeepClone().AsObject() ?? throw new InvalidOperationException("Demo battery is missing.");
            var storageIds = Arr(Api.Invoke("add_generic_storage_equipment", new { file })?["ids"]);
            baseStorage["coupling"] = new JsonObject { ["kind"] = "ac_coupled", ["battery_inverter_id"] = storageIds[2]?.DeepClone() };
            Api.Invoke("workflow_set", new { file, value = new { system = "off_grid", tools = "both" } });
            Api.Invoke("set_section", new { file, section = "island_clusters", value = new[] {
                new { name = "Principal", inverter_units = unitIds, load_share_pct = 0, storage = baseStorage,
                    load = new { profile = new { kind = "synthetic", profile = "residential" }, annual_kwh = 2800, critical_fraction = 1 } },
                new { name = "Taller", inverter_units = Array.Empty<string>(), load_share_pct = 0, storage = baseStorage,
                    load = new { profile = new { kind = "synthetic", profile = "office" }, annual_kwh = 1600, critical_fraction = 1 } },
            } });
            var metered = IOPath.Combine(dir, "workshop-load.csv");
            File.WriteAllLines(metered, Enumerable.Range(0, 8760).Select(i => i % 24 == 10 ? "2" : "0"));
            Api.Invoke("import_load", new { file, source = metered, island = "Taller" });
            var linked = Api.Invoke("cable_plan", new { file, mode = "apply_routed", circuits = new[] {
                new { label = "Salida principal", kind = "ac_single", metal = "copper", length_m = 18,
                    area_mm2 = 10, temperature_c = 50, voltage_v = 230, current_a = 1, circuits = 1, target_drop_pct = 3,
                    ampacity_a = 55, derating = .85, sizing_factor = 1.25, protection_a = 40,
                    binding = new { kind = "inverter_ac", unit_id = unitIds[0] },
                    local_rule = new { rule = "retie_inverter_ac", other_segments_pct = 1.1 },
                    code_protection = new { standard = "iec60364", maximum_circuit_v = 250, device_rated_v = 275, device_dc_rated = false,
                        breaking_capacity_ka = 10, conventional_trip_a = 52, conductor_k = 115, device_let_through_a2s = 200000,
                        fault_study = new {
                            strong = new { path = "line_neutral", r_mohm = 38, x_mohm = 0, voltage_factor = 1.0 },
                            weak = new { path = "line_earth", r_mohm = 40, x_mohm = 0, voltage_factor = 0.95 },
                            remote_temperature_c = 70, path_x_mohm_per_m = 0.05, return_area_mm2 = 10,
                        },
                        coordination = new { minimum_ratio = 1.15, curve_limit_ka = 8, maximum_clear_seconds = 0.4,
                            upstream = new { kind = "fuse", model = "Fusible sintético", reference = "Curva sintética de prueba",
                                points = new[] { new[] { 1000.0,0.5 }, new[] { 3000.0,0.14 }, new[] { 8000.0,0.04 } } },
                            downstream = new { kind = "breaker", model = "Interruptor sintético", reference = "Curva sintética de prueba",
                                points = new[] { new[] { 1000.0,0.18 }, new[] { 3000.0,0.048 }, new[] { 8000.0,0.01 } } },
                        },
                    } },
            } });
            Api.Invoke("simulate", new { file });
            Reload();
            if (Arr(_summary?["clusters"]).Count != 2 || Arr(_summary?["routed_cables"]).Count != 1 ||
                S(linked?["estimates"]?[0]?["code_checks"], "status") != "pass" ||
                S(linked?["estimates"]?[0]?["local_checks"], "status") != "pass" ||
                N(linked?["estimates"]?[0]?["code_checks"]?["fault_duty"], "near_ka") <= 6 ||
                N(_summary?["clusters"]?[1]?["balance"], "load_kwh") != 730)
                throw new InvalidOperationException("Island profile or protection results did not load.");
            var draft = linked?["circuits"]?[0]?.DeepClone().AsObject() ??
                throw new InvalidOperationException("Linked circuit did not round-trip.");
            var editor = new StackPanel();
            var code = draft["code_protection"]!.AsObject();
            code["fault_study"] = FaultEditor(editor, code["fault_study"])();
            code["coordination"] = CoordinationEditor(editor, code["coordination"])();
            draft["local_rule"] = LocalCableEditor(editor, draft["local_rule"])();
            var roundTrip = Api.Invoke("cable_plan", new { file, circuits = new[] { draft } })?["estimates"]?[0];
            if (S(roundTrip?["code_checks"],"status") != "pass" || S(roundTrip?["local_checks"],"status") != "pass")
                throw new InvalidOperationException("Protection editor round-trip failed.");
            await Capture(NavIsland, "25-island-clusters.png");
            await Capture(NavCables, "26-routed-cables.png");
            var initial = Api.Invoke("network_faults", new { file });
            var savedStudy = Api.Invoke("network_faults", new { file, mode = "save", studies = initial?["example"]?.DeepClone() });
            if (S(savedStudy?["results"]?[0]?["faults"]?[0], "status") != "pass")
                throw new InvalidOperationException("The DC network study did not pass its synthetic duty checks.");
            Reload();
            await Capture(NavCables, "27-network-faults.png", () =>
            {
                var network = CablesPage.Children.OfType<Expander>().Single(x => x.Name == "NetworkFaultSection");
                network.IsExpanded = true;
                var contents = (StackPanel)network.Content;
                var fault = contents.Children.OfType<Expander>().Single();
                fault.IsExpanded = true;
                ((StackPanel)fault.Content).Children.OfType<Expander>().Single().IsExpanded = true;
                Content.UpdateLayout();
                network.StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 0.0 });
            });
            var savedMesh = Api.Invoke("network_faults", new { file, mode = "save", studies = initial?["mesh_example"]?.DeepClone() });
            var meshFault = savedMesh?["results"]?[0]?["faults"]?[0];
            if (S(meshFault, "status") != "pass" || Math.Abs(N(meshFault?["post_trip"]?["stages"]?[0], "remaining_maximum_ka") - 0.48) > 1e-9)
                throw new InvalidOperationException("Expected redistribution through the remaining ring path.");
            Reload();
            await Capture(NavCables, "28-meshed-protection.png", () =>
            {
                var network = CablesPage.Children.OfType<Expander>().Single(x => x.Name == "NetworkFaultSection");
                network.IsExpanded = true;
                var contents = (StackPanel)network.Content;
                var fault = contents.Children.OfType<Expander>().Single();
                fault.IsExpanded = true;
                Content.UpdateLayout();
                network.StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 0.0 });
            });
            if (Environment.GetEnvironmentVariable("SOLARCRAFT_PVPRJ_QA") is string replay && File.Exists(replay))
            {
                Open(replay);
                if (PvsolComparisonPanel.Children.OfType<Expander>().Count() < 2)
                    throw new InvalidOperationException("Expected the imported PV*SOL reference on the Results page.");
                await Capture(NavResults, "29-pvsol-comparison.png");
            }
        }
        catch (Exception ex) { ok = false; File.WriteAllText(IOPath.Combine(dir, "error.txt"), ex.ToString()); }
        Application.Current.Exit();
        return ok;
    }
}
