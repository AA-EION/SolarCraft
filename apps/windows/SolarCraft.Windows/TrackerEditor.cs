// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml.Controls;
using System.Linq;
using System.Text.Json.Nodes;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private void BuildTrackingEditor(JsonNode array)
    {
        TrackingPanel.Children.Clear();
        var settings = array["tracking"];
        string kind = settings is null ? "fixed" : S(settings, "kind");
        var mode = Combo(T("tracker_mode"), new[] {
            ("fixed", T("tracker_fixed")), ("single_axis", T("tracker_single")),
            ("vertical_axis", T("tracker_vertical")), ("dual_axis", T("tracker_dual"))
        }, kind);
        var paramsPanel = new StackPanel { Spacing = 8 };
        var axisAzimuth = Num(T("tracker_axis_azimuth"), N(settings, "axis_azimuth_deg", Api.Num(array["azimuth"])));
        var axisTilt = Num(T("tracker_axis_tilt"), N(settings, "axis_tilt_deg"));
        var rotation = Num(T("tracker_rotation"), N(settings, "max_rotation_deg", 60));
        var center = Num(T("tracker_center"), N(settings, "center_azimuth_deg", Api.Num(array["azimuth"])));
        var travel = Num(T("tracker_travel"), N(settings, "azimuth_travel_deg", 360));
        var tilt = Num(T("tracker_max_tilt"), N(settings, "max_tilt_deg", 90));
        var backtrack = new CheckBox { Content = T("tracker_backtrack"), IsChecked = settings?["backtracking_gcr"] is not null };
        var gcr = Num(T("tracker_gcr"), N(settings, "backtracking_gcr", 0.4));
        var axisRow = Row(axisAzimuth, axisTilt);
        var compass = Row(center, travel);
        void ShowParameters()
        {
            paramsPanel.Children.Clear();
            switch (Sel(mode))
            {
                case "single_axis":
                    paramsPanel.Children.Add(axisRow);
                    paramsPanel.Children.Add(rotation);
                    paramsPanel.Children.Add(backtrack);
                    paramsPanel.Children.Add(gcr);
                    gcr.IsEnabled = backtrack.IsChecked == true;
                    break;
                case "dual_axis":
                    paramsPanel.Children.Add(tilt);
                    goto case "vertical_axis";
                case "vertical_axis":
                    paramsPanel.Children.Add(compass);
                    break;
            }
        }
        mode.SelectionChanged += (_, _) => ShowParameters();
        backtrack.Checked += (_, _) => gcr.IsEnabled = true;
        backtrack.Unchecked += (_, _) => gcr.IsEnabled = false;
        TrackingPanel.Children.Add(mode);
        TrackingPanel.Children.Add(paramsPanel);
        TrackingPanel.Children.Add(Btn(T("tracker_apply"), () =>
        {
            string selected = Sel(mode);
            JsonObject? update = selected switch
            {
                "single_axis" => new() {
                    ["kind"] = selected, ["axis_azimuth_deg"] = axisAzimuth.Value,
                    ["axis_tilt_deg"] = axisTilt.Value, ["max_rotation_deg"] = rotation.Value,
                    ["backtracking_gcr"] = backtrack.IsChecked == true ? JsonValue.Create(gcr.Value) : null,
                },
                "vertical_axis" => new() {
                    ["kind"] = selected, ["center_azimuth_deg"] = center.Value, ["azimuth_travel_deg"] = travel.Value,
                },
                "dual_axis" => new() {
                    ["kind"] = selected, ["center_azimuth_deg"] = center.Value, ["azimuth_travel_deg"] = travel.Value,
                    ["max_tilt_deg"] = tilt.Value,
                },
                _ => null,
            };
            Set($"/arrays/{_designArray}/tracking", update);
        }));
        ShowParameters();
        AddBifacialEditor(array);
    }

    private void AddBifacialEditor(JsonNode array)
    {
        var module = Arr(_project?["library"]?["modules"]).FirstOrDefault(
            item => S(item, "id") == S(array["module_id"], "value"));
        if (N(module, "bifaciality") <= 0) return;

        var rows = array["bifacial_rows"];
        var width = Num(T("bifacial_width"), N(rows, "slant_width_m", N(module, "length_m", 1.7)));
        var pitch = Num(T("bifacial_pitch"), N(rows, "row_pitch_m", 3.0));
        var height = Num(T("bifacial_height"), N(rows, "lower_edge_m", 1.0));
        var shade = Num(T("bifacial_structure"), 100 * N(rows, "mounting_shade", 0.1));
        width.Minimum = pitch.Minimum = 0.01;
        height.Minimum = shade.Minimum = 0;
        shade.Maximum = 100;
        var force = new CheckBox { Content = T("bifacial_force"), IsChecked = rows?["force_geometry"]?.GetValue<bool>() == true };
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(Row(width, pitch));
        panel.Children.Add(Row(height, shade));
        panel.Children.Add(force);
        var apply = Btn(T("bifacial_apply"), () => Set($"/arrays/{_designArray}/bifacial_rows", new
        {
            slant_width_m = width.Value, row_pitch_m = pitch.Value, lower_edge_m = height.Value,
            mounting_shade = shade.Value / 100.0, force_geometry = force.IsChecked == true,
        }));
        panel.Children.Add(rows is null ? apply : Row(apply,
            Btn(T("bifacial_clear"), () => Set($"/arrays/{_designArray}/bifacial_rows", null))));
        TrackingPanel.Children.Add(new Expander { Header = T("bifacial_rear"), Content = panel });
    }
}
