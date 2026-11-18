// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System;
using System.Globalization;
using System.Linq;
using System.Text.Json.Nodes;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private StackPanel OptionalForm(Panel owner, string label, JsonNode? current, out CheckBox enabled)
    {
        var container = new StackPanel { Spacing = 8 };
        var toggle = new CheckBox { Content = T("advanced_enable"), IsChecked = current is JsonObject };
        enabled = toggle;
        container.Children.Add(toggle);
        var fields = new StackPanel { Spacing = 8 };
        container.Children.Add(fields);
        void Refresh() => fields.Visibility = toggle.IsChecked == true ? Visibility.Visible : Visibility.Collapsed;
        toggle.Checked += (_, _) => Refresh();
        toggle.Unchecked += (_, _) => Refresh();
        Refresh();
        owner.Children.Add(new Expander { Header = T(label), Content = container, IsExpanded = current is JsonObject });
        return fields;
    }

    private Func<JsonObject?> FaultEditor(Panel owner, JsonNode? current)
    {
        var fields = OptionalForm(owner, "fault_study", current, out var enabled);
        fields.Children.Add(Note(T("fault_source_hint")));
        Func<JsonObject> Source(string key)
        {
            var source = current?[key];
            fields.Children.Add(H(T("fault_" + key)));
            var path = Combo(T("fault_path"), new[] { "line_neutral", "line_earth", "line_line", "three_phase" }
                .Select(p => (p, T("fault_" + p))).ToList(), S(source,"path"));
            fields.Children.Add(path);
            var numbers = MakeFields(fields, source, [
                ("r_mohm", "fault_r", double.NaN, false), ("x_mohm", "fault_x", 0, false),
                ("voltage_factor", "fault_voltage_factor", key == "strong" ? 1 : 0.95, false),
            ]);
            return () =>
            {
                var value = ReadFields(numbers);
                value["path"] = Sel(path);
                return value;
            };
        }
        var strong = Source("strong");
        var weak = Source("weak");
        var conductor = MakeFields(fields, current, [
            ("remote_temperature_c","fault_temp",70,false), ("path_x_mohm_per_m","fault_path_x",0,false),
        ]);
        var area = Num(T("fault_return_area"), N(current,"return_area_mm2",double.NaN), 295);
        var material = Combo(T("fault_return_metal"), [("", T("cable_material")), ("copper", T("cable_copper")), ("aluminium", T("cable_aluminium"))], S(current,"return_metal"));
        fields.Children.Add(area);
        fields.Children.Add(material);
        return () =>
        {
            if (enabled.IsChecked != true) return null;
            var result = ReadFields(conductor);
            result["strong"] = strong();
            result["weak"] = weak();
            if (double.IsFinite(area.Value)) result["return_area_mm2"] = area.Value;
            if (Sel(material).Length > 0) result["return_metal"] = Sel(material);
            return result;
        };
    }

    private Func<JsonObject?> CoordinationEditor(Panel owner, JsonNode? current)
    {
        var fields = OptionalForm(owner, "fault_coordination", current, out var enabled);
        var settings = MakeFields(fields, current, [
            ("minimum_ratio", "fault_margin", 1.15, false), ("curve_limit_ka", "fault_curve_limit", double.NaN, false),
            ("maximum_clear_seconds", "fault_clear_limit", double.NaN, false),
        ]);
        Func<JsonObject> Curve(string key)
        {
            var saved = current?[key];
            fields.Children.Add(H(T("fault_" + key)));
            var kind = Combo(T("fault_device"), [("breaker",T("fault_breaker")),("fuse",T("fault_fuse"))], S(saved,"kind"));
            fields.Children.Add(kind);
            var model = new TextBox { Header = T("model"), Text = S(saved,"model") };
            var reference = new TextBox { Header = T("fault_reference"), Text = S(saved,"reference") };
            var points = new TextBox { Header = T("fault_curve_points"), TextWrapping = TextWrapping.Wrap, AcceptsReturn = true,
                Text = string.Join("; ", Arr(saved?["points"]).Select(point => string.Join(",",
                    Arr(point).Select(v => v!.GetValue<double>().ToString("G", CultureInfo.InvariantCulture))))) };
            fields.Children.Add(model); fields.Children.Add(reference); fields.Children.Add(points);
            return () =>
            {
                var entries = new JsonArray();
                foreach (var line in points.Text.Split([';', '\n', '\r'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
                {
                    var pair = line.Split(',', StringSplitOptions.TrimEntries);
                    if (pair.Length != 2) throw new ArgumentException(T("fault_curve_points"));
                    entries.Add(new JsonArray(double.Parse(pair[0], CultureInfo.InvariantCulture), double.Parse(pair[1], CultureInfo.InvariantCulture)));
                }
                return new JsonObject { ["kind"] = Sel(kind), ["model"] = model.Text.Trim(), ["reference"] = reference.Text.Trim(), ["points"] = entries };
            };
        }
        var upstream = Curve("upstream");
        var downstream = Curve("downstream");
        var vendor = current?["manufacturer"];
        var vendorPanel = new StackPanel { Spacing = 8 };
        fields.Children.Add(new Expander { Header = T("fault_vendor"), Content = vendorPanel, IsExpanded = vendor is JsonObject });
        var vendorReference = new TextBox { Header = T("fault_reference"), Text = S(vendor,"reference") };
        vendorPanel.Children.Add(vendorReference);
        var ratings = MakeFields(vendorPanel, vendor, [
            ("maximum_ka","fault_vendor_ka",double.NaN,false), ("voltage_v","cable_voltage",double.NaN,false),
        ]);
        return () =>
        {
            if (enabled.IsChecked != true) return null;
            var value = ReadFields(settings);
            value["upstream"] = upstream();
            value["downstream"] = downstream();
            if (vendorReference.Text.Trim().Length > 0 || ratings.Any(x => double.IsFinite(x.field.Value)))
            {
                var spec = ReadFields(ratings);
                spec["reference"] = vendorReference.Text.Trim();
                value["manufacturer"] = spec;
            }
            return value;
        };
    }

    private Func<JsonObject?> LocalCableEditor(Panel owner, JsonNode? current)
    {
        var panel = new StackPanel { Spacing = 8 };
        owner.Children.Add(new Expander { Header = T("local_cable"), Content = panel, IsExpanded = current is JsonObject });
        var options = new[] { "", "retie_inverter_ac", "retie_service", "retie_overhead_service", "retie_feeder_branch", "authority" };
        var rule = Combo(T("code_rule_set"), options.Select(key => (key, T(key.Length == 0 ? "none" : "local_" + key))).ToList(), S(current,"rule"));
        panel.Children.Add(rule);
        var review = new StackPanel { Spacing = 8 };
        panel.Children.Add(review);
        var rest = Num(T("local_other"), N(current,"other_segments_pct",double.NaN), 295);
        var reviewed = Num(T("local_reviewed"), N(current,"reviewed_segment_pct",double.NaN), 295);
        review.Children.Add(rest); review.Children.Add(reviewed);
        var custom = new StackPanel { Spacing = 8 };
        review.Children.Add(custom);
        var limit = Num(T("local_limit"), N(current,"authority_limit_pct",double.NaN), 295);
        var reference = new TextBox { Header = T("fault_reference"), Text = S(current,"authority_reference") };
        custom.Children.Add(limit); custom.Children.Add(reference);
        var aerial = new StackPanel { Spacing = 8 };
        review.Children.Add(aerial);
        var awg = Num(T("local_awg"), N(current,"wire_awg",double.NaN), 295);
        aerial.Children.Add(awg);
        ComboBox Evidence(string field, string header)
        {
            var choices = Combo(T(header), [("", "—"), ("yes", T("yes")), ("no", T("no"))],
                current?[field] is JsonValue value ? (value.GetValue<bool>() ? "yes" : "no") : "");
            aerial.Children.Add(choices);
            return choices;
        }
        var certified = Evidence("aerial_cable_certified","local_aerial");
        var grade = Evidence("aa8000","local_grade");
        void Refresh()
        {
            var selected = Sel(rule);
            review.Visibility = selected.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            custom.Visibility = selected == "authority" ? Visibility.Visible : Visibility.Collapsed;
            aerial.Visibility = selected == "retie_overhead_service" ? Visibility.Visible : Visibility.Collapsed;
        }
        rule.SelectionChanged += (_, _) => Refresh();
        Refresh();
        return () =>
        {
            var selected = Sel(rule);
            if (selected.Length == 0) return null;
            var value = new JsonObject { ["rule"] = selected };
            if (double.IsFinite(rest.Value)) value["other_segments_pct"] = rest.Value;
            if (double.IsFinite(reviewed.Value)) value["reviewed_segment_pct"] = reviewed.Value;
            if (selected == "authority")
            {
                if (double.IsFinite(limit.Value)) value["authority_limit_pct"] = limit.Value;
                value["authority_reference"] = reference.Text.Trim();
            }
            if (selected == "retie_overhead_service")
            {
                if (double.IsFinite(awg.Value))
                {
                    if (awg.Value != Math.Truncate(awg.Value) || awg.Value < -3 || awg.Value > 40) throw new ArgumentException(T("local_awg"));
                    value["wire_awg"] = (int)awg.Value;
                }
                if (Sel(certified).Length > 0) value["aerial_cable_certified"] = Sel(certified) == "yes";
                if (Sel(grade).Length > 0) value["aa8000"] = Sel(grade) == "yes";
            }
            return value;
        };
    }
}
