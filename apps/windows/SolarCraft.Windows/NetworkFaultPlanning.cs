// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private static readonly JsonSerializerOptions StudyJson = new() { WriteIndented = true };

    private void BuildNetworkFaultPlanner(Panel panel, string file)
    {
        var body = new StackPanel { Spacing = 8 };
        panel.Children.Add(new Expander { Name = "NetworkFaultSection", Header = T("network_faults"),
            Content = body, HorizontalAlignment = HorizontalAlignment.Stretch });
        if (!Try(() => { var state = Api.Invoke("network_faults", new { file }); Render(state); })) return;

        void Render(JsonNode? state)
        {
            body.Children.Clear();
            body.Children.Add(Note(T("network_faults_hint")));
            var edit = new Button { Content = T("network_fault_edit") };
            edit.Click += async (_, _) =>
            {
                if (await EditNetworkFaults(file, state) is JsonNode saved && _file == file) Render(saved);
            };
            body.Children.Add(edit);
            RenderNetworkFaults(body, state);
        }
    }

    private async Task<JsonNode?> EditNetworkFaults(string file, JsonNode? state)
    {
        JsonNode? saved = null;
        var draft = new TextBox {
            Header = T("network_fault_json"), AcceptsReturn = true, TextWrapping = TextWrapping.NoWrap,
            Height = 255, MinWidth = 430, Text = state?["studies"]?.ToJsonString(StudyJson) ?? "[]",
        };
        var results = new StackPanel { Spacing = 6 };
        var fields = new StackPanel { Spacing = 10 };
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap };
        fields.Children.Add(Note(T("network_faults_hint")));
        fields.Children.Add(draft);
        fields.Children.Add(Row(
            Btn(T("network_fault_example"), () => { draft.Text = state?["example"]?.ToJsonString(StudyJson) ?? "[]"; results.Children.Clear(); }),
            Btn(T("network_fault_preview"), () => Run(false))));
        fields.Children.Add(error);
        fields.Children.Add(results);
        var dialog = new ContentDialog {
            Title = T("network_faults"), XamlRoot = Content.XamlRoot, PrimaryButtonText = T("network_fault_save"),
            CloseButtonText = T("cancel"), Content = new ScrollViewer { Content = fields, MaxHeight = 680 },
        };
        dialog.PrimaryButtonClick += (_, eventArgs) => eventArgs.Cancel = !Run(true);
        await dialog.ShowAsync();
        return saved;

        bool Run(bool save)
        {
            results.Children.Clear();
            try
            {
                if (_file != file) return false;
                var studies = JsonNode.Parse(draft.Text) as JsonArray
                    ?? throw new FormatException(T("network_fault_json"));
                var state = Api.Invoke("network_faults", new { file, studies, mode = save ? "save" : "preview" });
                error.Text = "";
                if (save) saved = state;
                else RenderNetworkFaults(results, state);
                return true;
            }
            catch (Exception ex) { error.Text = ex.Message; return false; }
        }
    }

    private void RenderNetworkFaults(Panel panel, JsonNode? state)
    {
        string Format(JsonNode? row, string key) => row?[key] is null ? "—" : F(N(row,key),3);
        string Status(JsonNode? row) => T("code_" + S(row, "status"));
        foreach (var study in Arr(state?["results"]))
        {
            panel.Children.Add(H($"{S(study,"name")} · {Status(study)}"));
            foreach (var fault in Arr(study?["faults"]))
            {
                var details = new StackPanel { Spacing = 6 };
                foreach (var source in Arr(fault?["outside_source_envelope"]))
                    details.Children.Add(Note($"{T("network_fault_outside")} {source?.GetValue<string>()}"));
                foreach (var source in Arr(fault?["source_paths_without_device"]))
                    details.Children.Add(Note($"{T("network_fault_no_device")} {source?.GetValue<string>()}"));
                if (fault?["staged_clearing_required"]?.GetValue<bool>() == true)
                    details.Children.Add(Note(T("network_fault_staged")));
                foreach (var source in Arr(fault?["sources"]))
                    details.Children.Add(Note($"{T("network_fault_source")} {S(source,"name")} · {T("network_fault_min_max")} {Format(source,"minimum_state_ka")} / {Format(source,"maximum_state_ka")}"));
                foreach (var d in Arr(fault?["devices"]))
                {
                    var checks = new StackPanel { Spacing = 4 };
                    checks.Children.Add(Note($"{T("network_fault_max")} {Format(d,"maximum_ka")}  ·  {T("network_fault_peak")} {Format(d,"peak_ka")}"));
                    checks.Children.Add(Table([T("code_protection"), T("network_fault_status"), T("code_actual"), T("code_threshold")],
                        Arr(d?["checks"]).Select(c => (IList<string>)new[] { T("check_" + S(c,"id")), Status(c), Format(c,"actual"), Format(c,"limit") })));
                    details.Children.Add(new Expander { Header = $"{S(d,"link")} · {S(d,"model")} · {Status(d)}", Content = checks });
                }
                foreach (var pair in Arr(fault?["coordination"]))
                    details.Children.Add(Note($"{S(pair,"backup_link")} → {S(pair,"primary_link")} · {Status(pair)} · {T("network_fault_ratio")} {Format(pair,"time_ratio")}"));
                panel.Children.Add(new Expander { Header = $"{S(fault,"bus")} · {Status(fault)} · {T("network_fault_min_max")} {Format(fault,"minimum_ka")} / {Format(fault,"maximum_ka")}",
                    Content = details });
            }
        }
    }
}
