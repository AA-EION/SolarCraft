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
                    estimates.Select((r, i) => (IList<string>)new[] { S(circuits[i], "label"), $"{F(N(r, "drop_pct"), 2)} %",
                        F(N(r, "loss_w"), 1), F(N(r, "area_for_target_mm2"), 2),
                        r?["allowed_a"] is null ? "—" : $"{F(N(r, "allowed_a"), 1)} / {F(N(circuits[i], "current_a"), 1)}",
                        r?["suggested"] is null ? "—" : $"{F(N(r["suggested"], "area_mm2"), 1)} mm²" })));
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
            holder.Children.Add(Row(save, apply, hourly));
        }

        void Evaluate(JsonArray rows, string mode, int selected)
        {
            if (!Try(() => data = Api.Invoke("cable_plan", new { file, circuits = rows, mode }))) return;
            if (mode is "save" or "apply" or "apply_hourly") {
                ShowInfo(T(mode == "save" ? "cable_save" : mode == "apply_hourly" ? "cable_hourly" : "cable_apply"));
                Reload();
            }
            else Render(Math.Max(0, selected));
        }

        Render();
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
        var extra = fields[1].box;
        void UpdateExtra() => extra.Visibility = Sel(kind).StartsWith("dc_", StringComparison.Ordinal)
            ? Visibility.Visible : Visibility.Collapsed;
        kind.SelectionChanged += (_, _) => UpdateExtra();
        UpdateExtra();
        var form = new StackPanel { Spacing = 7, Children = { name, kind, metal } };
        foreach (var field in fields) form.Children.Add(field.box);
        var ampacity = Num(T("cable_ampacity"), N(original, "ampacity_a", double.NaN), 280);
        var derating = Num(T("cable_derating"), N(original, "derating", 1), 280);
        var candidates = new TextBox { Header = T("cable_candidates"), TextWrapping = TextWrapping.Wrap,
            Text = string.Join("; ", Arr(original?["candidates"]).Select(row =>
                N(row, "area_mm2").ToString(CultureInfo.InvariantCulture) + "," + N(row, "ampacity_a").ToString(CultureInfo.InvariantCulture))) };
        form.Children.Add(ampacity); form.Children.Add(derating); form.Children.Add(candidates);
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
                var sizes = new JsonArray();
                foreach (var entry in candidates.Text.Split(';', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries))
                {
                    var pair = entry.Split(',', StringSplitOptions.TrimEntries);
                    if (pair.Length != 2) throw new ArgumentException(T("cable_candidates"));
                    sizes.Add(new JsonObject { ["area_mm2"] = double.Parse(pair[0], CultureInfo.InvariantCulture),
                        ["ampacity_a"] = double.Parse(pair[1], CultureInfo.InvariantCulture) });
                }
                result["candidates"] = sizes;
                Api.Invoke("cable_plan", new { file = _file, circuits = new[] { result }, mode = "preview" });
            }
            catch (Exception error) { args.Cancel = true; status.Text = error.Message; }
        };
        return await dialog.ShowAsync() == ContentDialogResult.Primary ? result : null;
    }
}
