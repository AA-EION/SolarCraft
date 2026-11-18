// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading.Tasks;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private string SystemType => _project?["workflow"]?["system"]?.GetValue<string>() ?? "custom";
    private bool Islanded => SystemType is "off_grid" or "off_grid_generator";

    private void BuildWorkflowPanel()
    {
        var p = WorkflowPanel; p.Children.Clear();
        var settings = _project?["workflow"];
        var system = Combo(T("workflow_type"),
            new[] { "custom", "grid_export", "grid_consumption", "grid_battery", "grid_ev", "grid_ev_battery", "grid_thermal", "grid_thermal_battery", "off_grid", "off_grid_generator" }
                .Select(key => (key, T(key))).ToList(), SystemType);
        var tools = Combo(T("workflow_tools"),
            new[] { "both", "roof", "scene" }.Select(key => (key, T("tools_" + key))).ToList(), S(settings, "tools"));
        p.Children.Add(system);
        p.Children.Add(tools);
        p.Children.Add(Note(T("workflow_type_hint")));
        p.Children.Add(Btn(T("apply"), () =>
        {
            if (Try(() => Api.Invoke("workflow_set", new { file = _file, value = new { system = Sel(system), tools = Sel(tools) } })))
                Reload();
        }, true));
    }

    private void BuildCablePlanner(Panel panel)
    {
        panel.Children.Add(H(T("cable_planner")));
        panel.Children.Add(Note(T("cable_planner_hint")));
        panel.Children.Add(Note(T("cable_hourly_hint")));
        var file = _file;
        var data = Api.Invoke("cable_plan", new { file });
        var holder = new StackPanel { Spacing = 8 };
        panel.Children.Add(holder);

        void Render(int selection = 0)
        {
            var circuits = Arr(data?["circuits"]);
            holder.Children.Clear();
            var picker = Combo(T("cable_planner"), circuits.Select((c, i) => (i.ToString(), S(c, "label"))).ToList(), selection.ToString());
            if (circuits.Count > 0) holder.Children.Add(picker);
            var add = new Button { Content = T("cable_add") };
            var edit = new Button { Content = T("cable_edit"), IsEnabled = circuits.Count > 0 };
            var remove = new Button { Content = T("remove"), IsEnabled = circuits.Count > 0 };
            var import = Btn(T("cable_import"), () => Evaluate((JsonArray)circuits.DeepClone(), "import_routes", picker.SelectedIndex));
            holder.Children.Add(Row(add, edit, remove, import));

            async Task Change(int index)
            {
                var existing = index < 0 ? null : circuits[index];
                var draft = await EditCable(existing);
                if (draft is null || _file != file) return;
                var rows = (JsonArray)circuits.DeepClone();
                if (index < 0) { rows.Add(draft); index = rows.Count - 1; }
                else rows[index] = draft;
                Evaluate(rows, "preview", index);
            }
            add.Click += async (_, _) => await Change(-1);
            edit.Click += async (_, _) => { if (picker.SelectedIndex >= 0) await Change(picker.SelectedIndex); };
            remove.Click += (_, _) =>
            {
                var index = picker.SelectedIndex;
                if (index < 0) return;
                var values = (JsonArray)circuits.DeepClone();
                values.RemoveAt(index);
                Evaluate(values, "preview", Math.Clamp(index, 0, Math.Max(0, values.Count - 1)));
            };

            var estimates = Arr(data?["estimates"]);
            if (estimates.Count > 0)
            {
                holder.Children.Add(Table([T("cable_planner"), T("cable_drop"), "W", T("cable_area_target"), T("cable_current_check"), T("cable_suggested")],
                    estimates.Select((r, i) => (IList<string>)(r is null ? new[] { S(circuits[i], "label"), "—", "—", "—", "—", "—" } : new[] {
                        S(circuits[i], "label"), $"{F(N(r, "drop_pct"), 2)} %", F(N(r, "loss_w"), 1), F(N(r, "area_for_target_mm2"), 2),
                        r["allowed_a"] is null ? "—" : $"{F(N(r, "allowed_a"), 1)} / {F(N(circuits[i], "design_current_a", N(circuits[i], "current_a")) * N(circuits[i], "sizing_factor", 1), 1)}",
                        r["suggested"] is null ? "—" : $"{F(N(r["suggested"], "area_mm2"), 1)} mm²" }))));
                foreach (var error in Arr(data?["row_errors"]).Where(e => e is not null)) holder.Children.Add(Note(error!.GetValue<string>()));
                var protection = new StackPanel { Spacing = 6 };
                holder.Children.Add(protection);
                void ShowProtection()
                {
                    protection.Children.Clear();
                    var index = picker.SelectedIndex;
                    if (index < 0) return;
                    var estimate = estimates.ElementAtOrDefault(index);
                    string Format(JsonNode? row, string key) => row?[key] is null ? "—" : F(N(row,key),3);
                    void Display(string title, JsonNode assessment)
                    {
                        var state = T("code_" + S(assessment,"status"));
                        var details = new StackPanel { Spacing = 6 };
                        if (assessment["fault_duty"] is JsonNode duty)
                            protection.Children.Add(Note($"{T("fault_near_ka")} {Format(duty,"near_ka")} · {T("fault_remote_ka")} {Format(duty,"remote_ka")}"));
                        if (assessment["coordination"] is JsonNode pair)
                            details.Children.Add(Note($"{S(pair,"upstream_model")} → {S(pair,"downstream_model")} · {T("fault_clearing")} {Format(pair,"remote_clear_seconds")}"));
                        details.Children.Add(Table([T("code_protection"), T("workflow_results"), T("code_actual"), T("code_threshold")],
                            Arr(assessment["checks"]).Select(c => (IList<string>)new[] {
                                T("check_" + S(c,"id")), T("code_" + S(c,"status")), Format(c,"actual"), Format(c,"limit"),
                            })));
                        protection.Children.Add(new Expander { Header = $"{title} · {state}", Content = details });
                    }
                    if (estimate?["code_checks"] is JsonNode code)
                        Display(S(code,"standard") == "nec2023_pv" ? "NEC 2023 PV DC" : "IEC 60364-4-43", code);
                    if (estimate?["local_checks"] is JsonNode local)
                        Display(S(local,"reference").Length > 0 ? S(local,"reference") : T("local_cable"), local);
                }
                picker.SelectionChanged += (_, _) => ShowProtection();
                ShowProtection();
                var useSize = Btn(T("cable_use_size"), () =>
                {
                    int index = picker.SelectedIndex;
                    if (index < 0 || estimates[index]?["suggested"] is not JsonNode suggestion) return;
                    var updated = (JsonArray)circuits.DeepClone();
                    updated[index]!["area_mm2"] = suggestion["area_mm2"]?.DeepClone();
                    updated[index]!["ampacity_a"] = suggestion["ampacity_a"]?.DeepClone();
                    Evaluate(updated, "preview", index);
                });
                void EnableSize() => useSize.IsEnabled = picker.SelectedIndex >= 0 && estimates[picker.SelectedIndex]?["suggested"] is not null;
                picker.SelectionChanged += (_, _) => EnableSize();
                EnableSize();
                holder.Children.Add(useSize);
                string Loss(string key) => data?[key] is null ? "—" : F(N(data, key), 3);
                holder.Children.Add(Note($"{T("workflow_dc_loss")} {Loss("dc_loss_pct")}     {T("workflow_ac_loss")} {Loss("ac_loss_pct")}"));
            }
            var save = Btn(T("cable_save"), () => Evaluate((JsonArray)circuits.DeepClone(), "save", 0));
            var apply = Btn(T("cable_apply"), () => Evaluate((JsonArray)circuits.DeepClone(), "apply", 0), true);
            apply.IsEnabled = circuits.Count > 0;
            var hourly = Btn(T("cable_hourly"), () => Evaluate((JsonArray)circuits.DeepClone(), "apply_hourly", 0), true);
            hourly.IsEnabled = apply.IsEnabled;
            var active = data?["hourly"];
            if (active?["dc_pct"] is not null || active?["ac_pct"] is not null) holder.Children.Add(Note(T("cable_hourly") + " ✓"));
            var routed = Btn(T("cable_apply_routed"), () => Evaluate((JsonArray)circuits.DeepClone(), "apply_routed", 0), true);
            routed.IsEnabled = circuits.Any(c => c?["binding"] is not null);
            holder.Children.Add(Row(save, apply, hourly, routed));
            var losses = Arr(_summary?["routed_cables"]);
            if (losses.Count > 0) holder.Children.Add(Table([T("cable_planner"), T("cable_routed_loss")],
                losses.Select(c => (IList<string>)new[] { S(c,"label"), F(N(c,"loss_kwh"),2) })));
        }

        void Evaluate(JsonArray rows, string mode, int selected)
        {
            if (!Try(() => data = Api.Invoke("cable_plan", new { file, circuits = rows, mode }))) return;
            if (mode is "save" or "apply" or "apply_hourly" or "apply_routed") {
                ShowInfo(T(mode switch { "save" => "cable_save", "apply_hourly" => "cable_hourly", "apply_routed" => "cable_apply_routed", _ => "cable_apply" }));
                Reload();
            }
            else Render(Math.Max(0, selected));
        }

        Render();
        if (file is not null) BuildNetworkFaultPlanner(panel, file);
    }

    private async Task<JsonObject?> EditCable(JsonNode? original)
    {
        var name = new TextBox { Header = T("cable_planner"), Text = original is null ? T("cable_dc_string") : S(original, "label") };
        var kind = Combo(T("cable_kind"), new[] { "dc_string", "dc_main", "ac_single", "ac_three" }
            .Select(key => (key, T("cable_" + key))).ToList(), S(original, "kind"));
        var metal = Combo(T("cable_material"), new[] { "copper", "aluminium" }
            .Select(key => (key, T("cable_" + key))).ToList(), S(original, "metal"));
        var fields = new[] {
            ("length_m", "cable_length", 20.0), ("extra_m", "cable_extra", 0.0), ("area_mm2", "cable_area", 4.0),
            ("temperature_c", "cable_temp", 60.0), ("voltage_v", "cable_voltage", 400.0),
            ("current_a", "cable_current", 10.0), ("circuits", "cable_count", 1.0), ("target_drop_pct", "cable_target", 1.5)
        }.Select(f => (key: f.Item1, box: Num(T(f.Item2), N(original, f.Item1, f.Item3), 250))).ToList();
        var form = new StackPanel { Spacing = 7, Children = { name, kind, metal } };
        var unitRows = Arr(Api.Invoke("stringing", new { file = _file })?["inverters"]);
        var links = new Dictionary<string, JsonObject>();
        var linkHolder = new StackPanel();
        form.Children.Add(linkHolder);
        ComboBox link = new();
        string Key(JsonNode? binding) => binding is null ? "" :
            S(binding,"kind") == "mppt" ? $"dc:{S(binding,"unit_id")}:{N(binding,"input")}" : $"ac:{S(binding,"unit_id")}";
        void BuildLinks()
        {
            var dc = Sel(kind).StartsWith("dc_", StringComparison.Ordinal);
            fields[1].box.Visibility = dc ? Visibility.Visible : Visibility.Collapsed;
            var choices = new List<(string, string)> { ("", T("cable_unbound")) };
            links.Clear();
            foreach (var row in unitRows)
            {
                var unit = row?["unit"];
                var id = S(unit,"id");
                var title = S(unit,"name");
                if (dc)
                {
                    var mppts = Arr(unit?["mppts"]);
                    for (var i = 0; i < mppts.Count; i++)
                    {
                        var binding = new JsonObject { ["kind"] = "mppt", ["unit_id"] = id, ["input"] = i };
                        var key = Key(binding);
                        choices.Add((key, $"{title} / MPPT {i + 1}"));
                        links[key] = binding;
                    }
                }
                else
                {
                    var binding = new JsonObject { ["kind"] = "inverter_ac", ["unit_id"] = id };
                    var key = Key(binding);
                    choices.Add((key, title));
                    links[key] = binding;
                }
            }
            linkHolder.Children.Clear();
            link = Combo(T("cable_link"), choices, Key(original?["binding"]));
            linkHolder.Children.Add(link);
            void UpdateFields()
            {
                fields[4].box.IsEnabled = !(dc && links.ContainsKey(Sel(link)));
                fields[5].box.IsEnabled = !links.ContainsKey(Sel(link));
            }
            link.SelectionChanged += (_, _) => UpdateFields();
            UpdateFields();
        }
        kind.SelectionChanged += (_, _) => BuildLinks();
        BuildLinks();
        foreach (var field in fields) form.Children.Add(field.box);
        var ampacity = Num(T("cable_ampacity"), N(original, "ampacity_a", double.NaN), 280);
        var derating = Num(T("cable_derating"), N(original, "derating", 1), 280);
        var candidates = new TextBox { Header = T("cable_candidates"), TextWrapping = TextWrapping.Wrap,
            Text = string.Join("; ", Arr(original?["candidates"]).Select(row =>
                N(row, "area_mm2").ToString(CultureInfo.InvariantCulture) + "," + N(row, "ampacity_a").ToString(CultureInfo.InvariantCulture))) };
        var factor = Num(T("cable_sizing_factor"), N(original, "sizing_factor", 1), 280);
        var protection = Num(T("cable_protection"), N(original, "protection_a", double.NaN), 280);
        form.Children.Add(ampacity); form.Children.Add(derating); form.Children.Add(factor);
        form.Children.Add(protection); form.Children.Add(candidates);
        var codePanel = new StackPanel { Spacing = 7 };
        var code = original?["code_protection"];
        var rules = Combo(T("code_rule_set"), [
            ("", T("code_manual")), ("nec2023_pv", "NEC 2023 PV DC"), ("iec60364", "IEC 60364-4-43"),
        ], S(code, "standard"));
        codePanel.Children.Add(rules);
        var codeDetails = new StackPanel { Spacing = 7 };
        codePanel.Children.Add(codeDetails);
        var ratingType = Combo(T("code_current_type"), [("", "—"), ("dc","DC"), ("ac","AC")],
            code?["device_dc_rated"] is JsonValue typed ? (typed.GetValue<bool>() ? "dc" : "ac") : "");
        codeDetails.Children.Add(ratingType);
        var codeFields = new[] {
            ("pv_isc_a", "nec2023_pv"), ("termination_ampacity_a", ""), ("equipment_max_device_a", "nec2023_pv"),
            ("maximum_circuit_v", ""), ("device_rated_v", ""), ("prospective_fault_ka", ""), ("breaking_capacity_ka", ""),
            ("conventional_trip_a", "iec60364"), ("conductor_k", "iec60364"), ("device_let_through_a2s", "iec60364"),
        }.Select(field => (key: field.Item1, standard: field.Item2, input: Num(T("code_" + field.Item1),
            N(code,field.Item1,double.NaN), 295))).ToArray();
        foreach (var field in codeFields) codeDetails.Children.Add(field.input);
        var studyPanel = new StackPanel { Spacing = 7 };
        codeDetails.Children.Add(studyPanel);
        var fault = FaultEditor(studyPanel, code?["fault_study"]);
        var coordination = CoordinationEditor(studyPanel, code?["coordination"]);
        var local = LocalCableEditor(form, original?["local_rule"]);
        void ShowCode()
        {
            var selected = Sel(rules);
            codeDetails.Visibility = selected.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            studyPanel.Visibility = selected == "iec60364" ? Visibility.Visible : Visibility.Collapsed;
            foreach (var entry in codeFields)
                entry.input.Visibility = entry.standard.Length == 0 || entry.standard == selected ? Visibility.Visible : Visibility.Collapsed;
        }
        rules.SelectionChanged += (_, _) => ShowCode();
        ShowCode();
        form.Children.Add(new Expander { Header = T("code_protection"), Content = codePanel, IsExpanded = code is JsonObject });
        var dialog = new ContentDialog { Title = T(original is null ? "cable_add" : "cable_edit"), XamlRoot = Content.XamlRoot,
            Content = new ScrollViewer { Content = form, MaxHeight = 535 }, PrimaryButtonText = T("cable_preview"), CloseButtonText = T("cancel") };
        var status = Note("");
        form.Children.Add(status);
        JsonObject? result = null;
        dialog.PrimaryButtonClick += (_, args) =>
        {
            if (fields.Any(f => f.box.Visibility == Visibility.Visible && !double.IsFinite(f.box.Value))
                || fields[6].box.Value != Math.Truncate(fields[6].box.Value))
            {
                args.Cancel = true; status.Text = T("issues"); return;
            }
            result = new JsonObject { ["label"] = name.Text, ["kind"] = Sel(kind), ["metal"] = Sel(metal),
                ["source_route"] = original?["source_route"]?.DeepClone() };
            foreach (var (key, box) in fields)
                result[key] = key == "circuits" ? JsonValue.Create((int)box.Value) :
                    JsonValue.Create(box.Visibility == Visibility.Visible ? box.Value : 0.0);
            try
            {
                result["ampacity_a"] = double.IsNaN(ampacity.Value) ? null : JsonValue.Create(ampacity.Value);
                result["derating"] = derating.Value;
                result["sizing_factor"] = factor.Value;
                result["protection_a"] = double.IsNaN(protection.Value) ? null : JsonValue.Create(protection.Value);
                if (links.TryGetValue(Sel(link), out var binding)) result["binding"] = binding.DeepClone();
                if (Sel(rules).Length > 0)
                {
                    var protectionSpec = new JsonObject { ["standard"] = Sel(rules) };
                    if (Sel(ratingType).Length > 0) protectionSpec["device_dc_rated"] = Sel(ratingType) == "dc";
                    foreach (var item in codeFields)
                        if (item.input.Visibility == Visibility.Visible && double.IsFinite(item.input.Value))
                            protectionSpec[item.key] = item.input.Value;
                    if (Sel(rules) == "iec60364")
                    {
                        protectionSpec["fault_study"] = fault();
                        protectionSpec["coordination"] = coordination();
                    }
                    result["code_protection"] = protectionSpec;
                }
                result["local_rule"] = local();
                var sizes = new JsonArray();
                foreach (var entry in candidates.Text.Split(';', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries))
                {
                    var pair = entry.Split(',', StringSplitOptions.TrimEntries);
                    if (pair.Length != 2) throw new ArgumentException(T("cable_candidates"));
                    sizes.Add(new JsonObject { ["area_mm2"] = double.Parse(pair[0], CultureInfo.InvariantCulture),
                        ["ampacity_a"] = double.Parse(pair[1], CultureInfo.InvariantCulture) });
                }
                result["candidates"] = sizes;
                var preview = Api.Invoke("cable_plan", new { file = _file, circuits = new[] { result }, mode = "preview" });
                if (Arr(preview?["row_errors"]).FirstOrDefault() is JsonNode error) throw new ArgumentException(error.GetValue<string>());
            }
            catch (Exception error) { args.Cancel = true; status.Text = error.Message; }
        };
        return await dialog.ShowAsync() == ContentDialogResult.Primary ? result : null;
    }
}
