// SPDX-License-Identifier: GPL-3.0-or-later
// 3D scene page: the core renders the viewport (z-buffered PNG) and answers
// picks, so Windows and macOS show the same picture. Every change is a scene
// transaction through the shared JSON API (undoable).
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using Windows.Storage.Pickers;
using Windows.Storage.Streams;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private const int SceneW = 900, SceneH = 540;
    private double _yaw = 25, _pitch = 35, _zoom = 1;
    private string _colorBy = "material";
    private JsonNode? _hit;
    private Image? _sceneImage;
    private StackPanel? _sceneInspector, _sceneLegend;
    private TextBlock? _sceneCounts, _sceneReport;
    private Windows.Foundation.Point? _pressAt;
    private (double yaw, double pitch) _pressCam;
    private bool _dragging, _rendering, _renderPending;

    private object SceneCamera() => new { yaw_deg = _yaw, pitch_deg = _pitch, zoom = _zoom, width = SceneW, height = SceneH };
    private string? SceneSelected() => _hit?["owner"]?.GetValue<string>();

    private void BuildScene()
    {
        var p = ScenePage; p.Children.Clear();
        if (_file is null) return;
        p.Children.Add(H(T("scene3d")));
        var color = Combo(T("color_by"), new List<(string, string)> { ("material", T("material")), ("string", T("string")), ("shading", T("shading")) }, _colorBy);
        color.SelectionChanged += (_, _) => { _colorBy = Sel(color); RenderScene(); };
        p.Children.Add(Row(color,
            Btn("−", () => { _zoom = Math.Max(0.3, _zoom / 1.25); RenderScene(); }),
            Btn("+", () => { _zoom = Math.Min(12, _zoom * 1.25); RenderScene(); }),
            Btn("⟲", () => { _yaw = 25; _pitch = 35; _zoom = 1; RenderScene(); }),
            Btn(T("undo"), () => { Try(() => Api.Invoke("undo", new { file = _file })); Reload(); })));
        _sceneImage = new Image { Width = SceneW, Height = SceneH, Stretch = Stretch.Fill };
        var host = new Grid { Width = SceneW, Height = SceneH, HorizontalAlignment = HorizontalAlignment.Left, BorderThickness = new Thickness(1), BorderBrush = new SolidColorBrush(Colors.Gray), Background = new SolidColorBrush(Colors.WhiteSmoke) };
        host.Children.Add(_sceneImage);
        _sceneLegend = new StackPanel { Margin = new Thickness(6), Spacing = 2, HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top };
        host.Children.Add(_sceneLegend);
        host.PointerPressed += OnScenePressed;
        host.PointerMoved += OnSceneMoved;
        host.PointerReleased += OnSceneReleased;
        p.Children.Add(host);
        _sceneCounts = Note("");
        p.Children.Add(_sceneCounts);
        _sceneInspector = new StackPanel { Spacing = 6, Width = 420 };
        var adder = BuildSceneAdder();
        p.Children.Add(Row(new StackPanel { Children = { H(T("selection")), _sceneInspector } }, new StackPanel { Children = { H(T("add_building")), adder } }));
        p.Children.Add(H(T("fidelity")));
        var fmt = Combo("", new List<(string, string)> { ("ifc", "IFC"), ("glb", "GLB"), ("obj", "OBJ"), ("dxf", "DXF"), ("igz", "IGZ (IngeTrazo)"), ("gltf", "glTF"), ("json", "JSON") }, "ifc");
        _sceneReport = new TextBlock { FontFamily = new FontFamily("Consolas"), FontSize = 12, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
        p.Children.Add(Row(Btn(T("import_model"), ImportModel), fmt, Btn(T("export_model"), () => ExportModel(Sel(fmt)))));
        p.Children.Add(_sceneReport);
        ShowInspector();
        RenderScene();
    }

    private async void RenderScene()
    {
        if (_file is not { } file || _sceneImage is not { } target) return;
        if (_rendering)
        {
            _renderPending = true;
            return;
        }
        _rendering = true;
        try
        {
            var request = new { file, camera = SceneCamera(), color_by = _colorBy, selected = SceneSelected() };
            var (counts, frame, bytes) = await Task.Run(() =>
            {
                var scene = Api.Invoke("scene_get", new { file });
                var rendered = Api.Invoke("scene_image", request)!;
                return (scene?["counts"], rendered, Convert.FromBase64String(rendered["png_base64"]!.GetValue<string>()));
            });
            if (file != _file || target != _sceneImage) return;
            if (_sceneCounts is not null && counts is not null)
                _sceneCounts.Text = $"{counts["objects"]} objects · {counts["faces"]} faces · {counts["fields"]} fields · {counts["modules"]} modules · {counts["strings"]} strings";
            using var ms = new InMemoryRandomAccessStream();
            await ms.WriteAsync(bytes.AsBuffer());
            ms.Seek(0);
            var bmp = new BitmapImage();
            await bmp.SetSourceAsync(ms);
            if (file != _file || target != _sceneImage) return;
            target.Source = bmp;
            if (_sceneLegend is not null)
            {
                _sceneLegend.Children.Clear();
                foreach (var e in Arr(frame["legend"]))
                {
                    var rgb = Arr(e?[1]).Select(x => (byte)x!.GetValue<int>()).ToArray();
                    if (rgb.Length != 3) continue;
                    _sceneLegend.Children.Add(Row(new Border { Width = 10, Height = 10, Background = new SolidColorBrush(Windows.UI.Color.FromArgb(255, rgb[0], rgb[1], rgb[2])) }, new TextBlock { Text = e?[0]?.ToString() ?? "", FontSize = 11 }));
                }
            }
        }
        catch (Exception ex) { ShowError(ex.Message); }
        finally
        {
            _rendering = false;
            if (_renderPending)
            {
                _renderPending = false;
                RenderScene();
            }
        }
    }

    private void OnScenePressed(object sender, PointerRoutedEventArgs e)
    {
        _pressAt = e.GetCurrentPoint((UIElement)sender).Position;
        _pressCam = (_yaw, _pitch);
        _dragging = false;
        ((UIElement)sender).CapturePointer(e.Pointer);
    }

    private void OnSceneMoved(object sender, PointerRoutedEventArgs e)
    {
        if (_pressAt is not { } a) return;
        var q = e.GetCurrentPoint((UIElement)sender).Position;
        var (dx, dy) = (q.X - a.X, q.Y - a.Y);
        if (!_dragging && Math.Abs(dx) + Math.Abs(dy) < 4) return;
        _dragging = true;
        _yaw = _pressCam.yaw - dx * 0.4;
        _pitch = Math.Clamp(_pressCam.pitch + dy * 0.3, 3, 89);
        RenderScene();
    }

    private void OnSceneReleased(object sender, PointerRoutedEventArgs e)
    {
        ((UIElement)sender).ReleasePointerCapture(e.Pointer);
        var at = _pressAt;
        _pressAt = null;
        if (_dragging || at is null) { _dragging = false; RenderScene(); return; }
        Try(() =>
        {
            _hit = Api.Invoke("scene_pick", new { file = _file, camera = SceneCamera(), x = at.Value.X, y = at.Value.Y });
            if (_hit?["hit"] is null) _hit = null;
        });
        ShowInspector();
        RenderScene();
    }

    private void SceneEdit(object[] edits)
    {
        if (Try(() =>
        {
            var r = Api.Invoke("scene_edit", new { file = _file, edits })!;
            var errs = Arr(r["issues"]).Where(i => S(i, "severity") == "error").Select(i => S(i, "message")).ToList();
            if (errs.Count > 0) ShowError(string.Join("\n", errs));
        })) Reload();
    }

    private string FirstArray() => S(Arr(_project?["arrays"]).FirstOrDefault(), "id");

    private void ShowInspector()
    {
        var p = _sceneInspector; if (p is null) return;
        p.Children.Clear();
        if (_hit is null) { p.Children.Add(Note("Click the model to select a roof face, object or module.")); return; }
        var hit = S(_hit, "hit");
        p.Children.Add(new TextBlock { Text = hit, Style = (Style)Application.Current.Resources["BodyStrongTextBlockStyle"] });
        if (_hit["object"] is JsonNode o && o is JsonObject)
        {
            var id = S(o, "id");
            p.Children.Add(Note($"{S(o, "name")} · {S(o["kind"], "type")} · {S(o, "layer")}"));
            p.Children.Add(Row(
                Btn("←", () => SceneEdit(new object[] { new { op = "move", id, dx = -0.5, dy = 0.0 } })),
                Btn("→", () => SceneEdit(new object[] { new { op = "move", id, dx = 0.5, dy = 0.0 } })),
                Btn("↓", () => SceneEdit(new object[] { new { op = "move", id, dx = 0.0, dy = -0.5 } })),
                Btn("↑", () => SceneEdit(new object[] { new { op = "move", id, dx = 0.0, dy = 0.5 } })),
                Btn(T("delete_selected"), () => { _hit = null; SceneEdit(new object[] { new { op = "remove_object", id } }); })));
            if (hit.Contains("/roof-"))
            {
                p.Children.Add(Btn(T("place_modules"), () =>
                {
                    if (Try(() => Api.Invoke("scene_place_roof", new { file = _file, face = hit }))) Reload();
                }, true));
            }
        }
        if (_hit["field"] is JsonNode f && f is JsonObject)
        {
            var fid = S(f, "id");
            p.Children.Add(Note($"{S(f, "name")} · {f["modules"]} modules · {f["strings"]} strings · array {S(f, "array_id")}"));
            var mps = Num(T("modules_per_string"), 8, 200);
            var storedField = Arr(_project?["scene"]?["fields"]).FirstOrDefault(row => S(row, "id") == fid);
            var point = Arr(storedField?["inverter_at"]);
            var toInverter = new CheckBox { Content = T("workflow_home_run"), IsChecked = point.Count == 3 };
            double Coordinate(int index) => point.Count == 3 ? point[index]!.GetValue<double>() : 0.0;
            var x = Num("X (m)", Coordinate(0), 125);
            var y = Num("Y (m)", Coordinate(1), 125);
            var z = Num("Z (m)", Coordinate(2), 125);
            var coordinates = Row(x, y, z);
            void CoordinateVisibility() => coordinates.Visibility = toInverter.IsChecked == true ? Visibility.Visible : Visibility.Collapsed;
            toInverter.Checked += (_, _) => CoordinateVisibility();
            toInverter.Unchecked += (_, _) => CoordinateVisibility();
            CoordinateVisibility();
            p.Children.Add(toInverter);
            p.Children.Add(coordinates);
            p.Children.Add(Row(mps,
                Btn(T("route_strings"), () => SceneEdit(new object[] { new {
                    op = "route_strings", field = fid, modules_per_string = (int)mps.Value,
                    inverter_at = toInverter.IsChecked == true ? new[] { x.Value, y.Value, z.Value } : null
                } })),
                Btn(T("fit_to_array"), () => { Try(() => Api.Invoke("scene_bind", new { file = _file, field = fid, array_id = string.IsNullOrEmpty(S(f, "array_id")) ? FirstArray() : S(f, "array_id"), match_array = true })); Reload(); })));
            if (hit.Contains("/m")) p.Children.Add(Btn("Remove " + hit.Split('/').Last(), () => SceneEdit(new object[] { new { op = "remove_modules", field = fid, modules = new[] { hit } } })));
            p.Children.Add(Btn(T("delete_selected"), () => { _hit = null; SceneEdit(new object[] { new { op = "remove_field", id = fid } }); }));
        }
    }

    private StackPanel BuildSceneAdder()
    {
        var p = new StackPanel { Spacing = 6, Width = 460 };
        var kind = Combo("", new List<(string, string)> { ("building", T("add_building")), ("tree", T("add_tree")), ("block", T("add_obstacle")) }, "building");
        var x = Num("x (m)", 0, 110); var y = Num("y (m)", 0, 110);
        var w = Num("W (m)", 14, 110); var d = Num("D (m)", 9, 110); var eave = Num(T("eave_height"), 5.5, 140);
        var roof = Combo(T("roof_type"), new List<(string, string)> { ("flat", "flat"), ("shed", "shed"), ("gable", "gable"), ("hip", "hip") }, "gable");
        var pitch = Num(T("pitch"), 25, 110); var rot = Num(T("rotation"), 0, 110);
        var crown = Num("R (m)", 3, 110); var height = Num("H (m)", 9, 110); var tr = Num(T("transparency"), 0.25, 140);
        var bl = Num("L (m)", 0.8, 90); var bw = Num("W (m)", 0.8, 90); var bh = Num("H (m)", 1.5, 90); var bz = Num("z (m)", 5, 90);
        var building = Row(w, d, eave); var buildingRoof = Row(roof, pitch, rot);
        var tree = Row(crown, height, tr); var block = Row(bl, bw, bh, bz);
        void Show() { var k = Sel(kind); building.Visibility = buildingRoof.Visibility = k == "building" ? Visibility.Visible : Visibility.Collapsed; tree.Visibility = k == "tree" ? Visibility.Visible : Visibility.Collapsed; block.Visibility = k == "block" ? Visibility.Visible : Visibility.Collapsed; }
        kind.SelectionChanged += (_, _) => Show();
        p.Children.Add(kind); p.Children.Add(Row(x, y)); p.Children.Add(building); p.Children.Add(buildingRoof); p.Children.Add(tree); p.Children.Add(block);
        p.Children.Add(Btn("+", () =>
        {
            var k = Sel(kind);
            if (k == "building")
            {
                var (c, s) = (Math.Cos(rot.Value * Math.PI / 180), Math.Sin(rot.Value * Math.PI / 180));
                var fp = new[] { (0.0, 0.0), (w.Value, 0.0), (w.Value, d.Value), (0.0, d.Value) }.Select(q => new[] { x.Value + q.Item1 * c - q.Item2 * s, y.Value + q.Item1 * s + q.Item2 * c }).ToArray();
                var rt = Sel(roof);
                object rf = rt switch { "flat" => new { type = "flat" }, "shed" => new { type = "shed", pitch_deg = pitch.Value, facing_deg = 180 + rot.Value }, _ => new { type = rt, pitch_deg = pitch.Value } };
                SceneEdit(new object[] { new { op = "add_object", name = "Building", layer = "building", kind = new { type = "building", footprint = fp, eave_height = eave.Value, roof = rf } } });
            }
            else if (k == "tree")
                SceneEdit(new object[] { new { op = "add_object", name = "Tree", layer = "vegetation", kind = new { type = "tree", center = new[] { x.Value, y.Value }, trunk_height = height.Value * 0.35, trunk_radius = 0.25, crown_radius = crown.Value, crown_height = height.Value * 0.65, transparency = tr.Value } } });
            else
                SceneEdit(new object[] { new { op = "add_object", name = "Obstacle", layer = "obstacles", kind = new { type = "block", center = new[] { x.Value, y.Value }, base_z = bz.Value, length = bl.Value, width = bw.Value, height = bh.Value, rotation_deg = rot.Value } } });
        }, true));
        Show();
        return p;
    }

    private static string Describe(JsonNode? r)
    {
        var f = r?["fidelity"];
        var lines = new List<string>
        {
            $"{S(f, "format").ToUpperInvariant()} {S(f, "direction")}: {f?["objects"]} objects, {f?["fields"]} fields, {f?["modules"]} modules, {f?["triangles"]} triangles",
            $"axes: {S(f, "axes")}", $"georeference: {S(f, "georeference")}", $"max coordinate error: {N(f, "max_coordinate_error_m"):0.00e0} m",
        };
        lines.AddRange(Arr(f?["conversions"]).Select(c => "converted: " + c));
        lines.AddRange(Arr(f?["unsupported"]).Select(c => "not carried: " + c));
        lines.AddRange(Arr(r?["notes"]).Select(c => "note: " + c));
        return string.Join("\n", lines);
    }

    private async void ImportModel()
    {
        var picker = new FileOpenPicker();
        foreach (var e in new[] { ".glb", ".gltf", ".obj", ".dxf", ".ifc", ".igz", ".json" }) picker.FileTypeFilter.Add(e);
        InitPicker(picker);
        var f = await picker.PickSingleFileAsync();
        if (f is null) return;
        JsonNode? r = null;
        if (Try(() => r = Api.Invoke("scene_import", new { file = _file, source = f.Path, options = new { relink = true } })))
        {
            Reload();
            if (_sceneReport is not null) _sceneReport.Text = Describe(r);
        }
    }

    private async void ExportModel(string format)
    {
        var picker = new FileSavePicker { SuggestedFileName = System.IO.Path.GetFileNameWithoutExtension(_file) };
        picker.FileTypeChoices.Add(format.ToUpperInvariant(), new List<string> { "." + format });
        InitPicker(picker);
        var f = await picker.PickSaveFileAsync();
        if (f is null) return;
        JsonNode? r = null;
        if (Try(() => r = Api.Invoke("scene_export", new { file = _file, output = f.Path, format })) && _sceneReport is not null) _sceneReport.Text = Describe(r);
    }
}
