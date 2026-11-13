// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Windows.Storage.Pickers;

namespace SolarCraft;

public sealed partial class MainWindow : Window
{
    private Dictionary<string, string> _s = new();
    private string? _file;
    private JsonNode? _project;
    private bool _loading;
    private CancellationTokenSource? _cts;

    public MainWindow()
    {
        InitializeComponent();
        try
        {
            var ui = Api.Invoke("ui_strings", new { lang = CultureInfo.CurrentUICulture.Name })!.AsObject();
            foreach (var kv in ui) _s[kv.Key] = kv.Value!.GetValue<string>();
        }
        catch (Exception ex) { ShowError(ex.Message); }
        ApplyStrings();
    }

    private string T(string k) => _s.TryGetValue(k, out var v) ? v : k;

    private void ApplyStrings()
    {
        Title = T("app_title");
        NewButton.Label = T("new_project"); OpenButton.Label = T("open_project"); UndoButton.Label = T("undo");
        SimulateButton.Label = T("simulate"); CancelButton.Label = T("cancel");
        NavProject.Content = T("project"); NavSite.Content = T("site_weather"); NavDesign.Content = T("design"); NavRoof.Content = T("roof"); NavResults.Content = T("results"); NavReport.Content = T("report");
        EmptyText.Text = T("no_project");
        NameBox.Header = T("name"); CustomerBox.Header = T("customer"); AddressBox.Header = T("address");
        LatBox.Header = T("latitude"); LonBox.Header = T("longitude"); ElevBox.Header = T("elevation"); AlbedoBox.Header = T("albedo");
        WeatherHeader.Text = T("weather"); FetchButton.Content = T("fetch_pvgis"); ImportButton.Content = T("import_weather"); DemoButton.Content = T("demo_weather"); DemoWarning.Text = T("demo_warning");
        ModeRadios.Header = T("mode"); EasyRadio.Content = T("easy"); DetailedRadio.Content = T("detailed"); ModeHint.Text = T("mode_hint");
        KwpBox.Header = T("target_kwp"); TiltBox.Header = T("tilt"); AzimuthBox.Header = T("azimuth"); MountingBox.Header = T("mounting");
        MountingBox.ItemsSource = new[] { T("open_rack"), T("roof_mounted"), T("integrated") };
        MpsBox.Header = T("modules_per_string"); StringsBox.Header = T("strings"); IssuesHeader.Text = T("issues");
        NoResults.Text = T("no_results"); MonthlyHeader.Text = T("monthly_energy"); LossesHeader.Text = T("losses");
        RoofShapeBox.Header = T("roof_shape");
        RoofShapeBox.ItemsSource = new[] { T("roof_rect"), T("roof_trapezoid"), T("roof_custom"), T("roof_none") };
        EaveBox.Header = T("eave_length"); RidgeBox.Header = T("ridge_length"); SlopeBox.Header = T("slope_length");
        CustomBox.Header = T("custom_points"); ObstaclesBox.Header = T("obstacles"); SetbackBox.Header = T("setback"); ClearanceBox.Header = T("clearance");
        ClickHint.Text = T("click_to_add"); UndoPointButton.Content = T("undo_point"); ClearPointsButton.Content = T("clear_points");
        RoofHint.Text = T("roof_hint"); ApplyRoofButton.Content = T("apply_roof");
        LangBox.Header = T("language"); FormatBox.Header = T("format"); DesignerBox.Header = T("designer"); ExportButton.Content = T("export_report");
        var two = CultureInfo.CurrentUICulture.TwoLetterISOLanguageName;
        LangBox.SelectedIndex = two switch { "es" => 1, "de" => 2, "pt" => 3, _ => 0 };
    }

    private void ShowError(string msg) { Info.Severity = InfoBarSeverity.Error; Info.Title = T("error"); Info.Message = msg; Info.IsOpen = true; }
    private void ShowInfo(string msg) { Info.Severity = InfoBarSeverity.Success; Info.Title = ""; Info.Message = msg; Info.IsOpen = true; }

    private bool Try(Action a)
    {
        try { a(); return true; } catch (Exception ex) { ShowError(ex.Message); return false; }
    }

    private void InitPicker(object picker) => WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));

    // ---- Project lifecycle ----
    private async void OnNew(object sender, RoutedEventArgs e)
    {
        var picker = new FileSavePicker { SuggestedFileName = "Project" };
        picker.FileTypeChoices.Add("SolarCraft", new List<string> { ".solarcraft" });
        InitPicker(picker);
        var f = await picker.PickSaveFileAsync();
        if (f is null) return;
        try { await f.DeleteAsync(); } catch { }
        if (Try(() => Api.Invoke("create_project", new { file = f.Path, name = System.IO.Path.GetFileNameWithoutExtension(f.Path), latitude = 4.711, longitude = -74.072 })))
            Open(f.Path);
    }

    private async void OnOpen(object sender, RoutedEventArgs e)
    {
        var picker = new FileOpenPicker();
        picker.FileTypeFilter.Add(".solarcraft");
        InitPicker(picker);
        var f = await picker.PickSingleFileAsync();
        if (f is not null) Open(f.Path);
    }

    private void Open(string path)
    {
        _file = path;
        Nav.IsEnabled = true; UndoButton.IsEnabled = true; SimulateButton.IsEnabled = true;
        EmptyText.Visibility = Visibility.Collapsed;
        Reload();
        Nav.SelectedItem ??= NavProject;
    }

    private void Reload()
    {
        if (_file is null) return;
        Try(() =>
        {
            _project = Api.Invoke("get_project", new { file = _file });
            ShowProject();
            ShowIssues(Api.Invoke("validate", new { file = _file })!["issues"]!.AsArray());
            JsonNode? r = null;
            try { r = Api.Invoke("explain", new { file = _file }); } catch (CoreError ce) when (ce.Code == "NO_RESULTS") { }
            ShowResults(r);
        });
    }

    private void ShowProject()
    {
        var p = _project!;
        var a = p["arrays"]![0]!;
        _loading = true;
        NameBox.Text = Api.Str(p["name"]); CustomerBox.Text = p["customer"]!["name"]!.GetValue<string>(); AddressBox.Text = p["customer"]!["address"]!.GetValue<string>();
        LatBox.Value = Api.Num(p["site"]!["latitude"]); LonBox.Value = Api.Num(p["site"]!["longitude"]); ElevBox.Value = Api.Num(p["site"]!["elevation"]); AlbedoBox.Value = Api.Num(p["site"]!["albedo"]);
        WeatherText.Text = p["weather"] is JsonNode w ? $"{w["provider"]}\n{w["summary"]}" : T("no_weather");
        var detailed = p["mode"]!.GetValue<string>() == "detailed";
        ModeRadios.SelectedIndex = detailed ? 1 : 0;
        KwpBox.Value = Api.Num(a["target_kwp"]); TiltBox.Value = Api.Num(a["tilt"]); AzimuthBox.Value = Api.Num(a["azimuth"]);
        MountingBox.SelectedIndex = Api.Str(p["simulation"]!["mounting"]) switch { "open_rack" => 0, "integrated" => 2, _ => 1 };
        MpsBox.Value = Api.Num(a["modules_per_string"]); StringsBox.Value = Api.Num(a["strings"]);
        MpsBox.IsEnabled = detailed; StringsBox.IsEnabled = detailed;
        foreach (var (box, key) in new[] { (KwpBox, "target_kwp"), (TiltBox, "tilt"), (AzimuthBox, "azimuth"), (MpsBox, "modules_per_string"), (StringsBox, "strings") })
            box.PlaceholderText = Api.IsUser(a[key]) ? "" : T("derived");
        var mod = p["library"]!["modules"]!.AsArray().FirstOrDefault(m => m!["id"]!.GetValue<string>() == Api.Str(a["module_id"]));
        var inv = p["library"]!["inverters"]!.AsArray().FirstOrDefault(i => i!["id"]!.GetValue<string>() == Api.Str(a["inverter_id"]));
        EquipmentText.Text = $"{T("module")}: {mod?["manufacturer"]} {mod?["model"]}\n{T("inverter")}: {Api.Num(a["inverter_count"])} × {inv?["manufacturer"]} {inv?["model"]}";
        ShowRoof(a["roof"]);
        _loading = false;
    }

    // ---- Roof ----
    private static List<double[]> ParsePoints(string text) => text.Split(';', StringSplitOptions.RemoveEmptyEntries)
        .Select(p => p.Split(',').Select(v => double.TryParse(v.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var d) ? d : double.NaN).ToArray())
        .Where(v => v.Length == 2 && !v.Any(double.IsNaN)).ToList();

    public static List<List<double[]>> ParseObstacles(string text) => text.Split(';', StringSplitOptions.RemoveEmptyEntries)
        .Select(p => p.Split(',').Select(v => double.TryParse(v.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var d) ? d : double.NaN).ToArray())
        .Where(v => (v.Length == 4 || v.Length == 5) && !v.Any(double.IsNaN) && v[2] > 0 && v[3] > 0)
        .Select(v => new List<double[]> { new[] { v[0], v[1] }, new[] { v[0] + v[2], v[1] }, new[] { v[0] + v[2], v[1] + v[3] }, new[] { v[0], v[1] + v[3] } }).ToList();

    /// Heights (5th value, metres; default 1) of the obstacles accepted by ParseObstacles.
    public static List<double> ParseObstacleHeights(string text) => text.Split(';', StringSplitOptions.RemoveEmptyEntries)
        .Select(p => p.Split(',').Select(v => double.TryParse(v.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var d) ? d : double.NaN).ToArray())
        .Where(v => (v.Length == 4 || v.Length == 5) && !v.Any(double.IsNaN) && v[2] > 0 && v[3] > 0)
        .Select(v => v.Length == 5 ? v[4] : 1.0).ToList();

    private void OnRoofShape(object sender, SelectionChangedEventArgs e)
    {
        var i = RoofShapeBox.SelectedIndex;
        EaveBox.Visibility = SlopeBox.Visibility = i is 0 or 1 ? Visibility.Visible : Visibility.Collapsed;
        RidgeBox.Visibility = i == 1 ? Visibility.Visible : Visibility.Collapsed;
        CustomBox.Visibility = DraftPanel.Visibility = i == 2 ? Visibility.Visible : Visibility.Collapsed;
        ObstaclesBox.Visibility = SetbackBox.Visibility = ClearanceBox.Visibility = i == 3 ? Visibility.Collapsed : Visibility.Visible;
    }

    // ---- Click-to-add outline editor (1 m grid, 0.1 m snap, y up the slope) ----
    private static string FormatPoints(IEnumerable<double[]> pts) => string.Join("; ", pts.Select(p => string.Format(CultureInfo.InvariantCulture, "{0:0.0},{1:0.0}", p[0], p[1])));

    private (double w, double h, double scale) DraftExtent(List<double[]> pts)
    {
        var w = Math.Max(12, (pts.Count > 0 ? pts.Max(p => p[0]) : 0) + 2);
        var h = Math.Max(8, (pts.Count > 0 ? pts.Max(p => p[1]) : 0) + 2);
        return (w, h, Math.Min(DraftCanvas.Width / w, DraftCanvas.Height / h));
    }

    private void DrawDraft()
    {
        DraftCanvas.Children.Clear();
        var pts = ParsePoints(CustomBox.Text);
        var (w, h, scale) = DraftExtent(pts);
        var gridBrush = new SolidColorBrush(ColorHelper.FromArgb(60, 128, 128, 128));
        for (var gx = 0; gx <= (int)w; gx++) DraftCanvas.Children.Add(new Line { X1 = gx * scale, Y1 = 0, X2 = gx * scale, Y2 = h * scale, Stroke = gridBrush, StrokeThickness = 0.5 });
        for (var gy = 0; gy <= (int)h; gy++) DraftCanvas.Children.Add(new Line { X1 = 0, Y1 = (h - gy) * scale, X2 = w * scale, Y2 = (h - gy) * scale, Stroke = gridBrush, StrokeThickness = 0.5 });
        if (pts.Count >= 2)
        {
            var pg = new Polygon { Stroke = new SolidColorBrush(Colors.Gray), StrokeThickness = 1.5, Fill = pts.Count >= 3 ? new SolidColorBrush(ColorHelper.FromArgb(40, 128, 128, 128)) : null };
            foreach (var p in pts) pg.Points.Add(new Windows.Foundation.Point(p[0] * scale, (h - p[1]) * scale));
            DraftCanvas.Children.Add(pg);
        }
        foreach (var p in pts)
        {
            var dot = new Ellipse { Width = 8, Height = 8, Fill = new SolidColorBrush(ColorHelper.FromArgb(255, 0x0f, 0x5c, 0x63)) };
            Canvas.SetLeft(dot, p[0] * scale - 4); Canvas.SetTop(dot, (h - p[1]) * scale - 4);
            DraftCanvas.Children.Add(dot);
        }
    }

    private void OnCustomChanged(object sender, TextChangedEventArgs e) => DrawDraft();

    private void OnDraftPressed(object sender, Microsoft.UI.Xaml.Input.PointerRoutedEventArgs e)
    {
        var pos = e.GetCurrentPoint(DraftCanvas).Position;
        var pts = ParsePoints(CustomBox.Text);
        var (w, h, scale) = DraftExtent(pts);
        var x = Math.Round(pos.X / scale * 10) / 10;
        var y = Math.Round((h * scale - pos.Y) / scale * 10) / 10;
        if (x < 0 || y < 0 || x > w || y > h) return;
        pts.Add(new[] { x, y });
        CustomBox.Text = FormatPoints(pts);
    }

    private void OnUndoPoint(object sender, RoutedEventArgs e)
    {
        var pts = ParsePoints(CustomBox.Text);
        if (pts.Count > 0) pts.RemoveAt(pts.Count - 1);
        CustomBox.Text = FormatPoints(pts);
    }

    private void OnClearPoints(object sender, RoutedEventArgs e) => CustomBox.Text = "";

    private void OnApplyRoof(object sender, RoutedEventArgs e)
    {
        if (_file is null) return;
        double eave = EaveBox.Value, ridge = RidgeBox.Value, slope = SlopeBox.Value;
        List<double[]>? outline = RoofShapeBox.SelectedIndex switch
        {
            0 => new() { new[] { 0.0, 0.0 }, new[] { eave, 0.0 }, new[] { eave, slope }, new[] { 0.0, slope } },
            1 => new() { new[] { 0.0, 0.0 }, new[] { eave, 0.0 }, new[] { eave - Math.Max(0, (eave - ridge) / 2), slope }, new[] { Math.Max(0, (eave - ridge) / 2), slope } },
            2 => ParsePoints(CustomBox.Text),
            _ => null,
        };
        if (outline is { Count: < 3 }) { ShowError(T("custom_points")); return; }
        Try(() =>
        {
            var r = outline is null
                ? Api.Invoke("set_roof", new { file = _file, array = 0, outline = (object?)null })!
                : Api.Invoke("set_roof", new { file = _file, array = 0, outline, obstacles = ParseObstacles(ObstaclesBox.Text), obstacle_heights = ParseObstacleHeights(ObstaclesBox.Text), @params = new { setback = SetbackBox.Value, obstacle_clearance = ClearanceBox.Value } })!;
            if (r["modules_fit"] is JsonNode n) ShowInfo($"{n.GetValue<int>()} {T("modules_fit")}");
        });
        Reload();
    }

    private void ShowRoof(JsonNode? roof)
    {
        RoofCanvas.Children.Clear();
        RoofCount.Text = "";
        if (roof is null) return;
        var outline = roof["face"]!["outline"]!.AsArray().Select(p => new[] { p![0]!.GetValue<double>(), p[1]!.GetValue<double>() }).ToList();
        if (outline.Count < 3) return;
        double minX = outline.Min(p => p[0]), maxX = outline.Max(p => p[0]), minY = outline.Min(p => p[1]), maxY = outline.Max(p => p[1]);
        if (maxX <= minX || maxY <= minY) return;
        const double pad = 12;
        var scale = Math.Min((RoofCanvas.Width - 2 * pad) / (maxX - minX), (RoofCanvas.Height - 2 * pad) / (maxY - minY));
        Windows.Foundation.Point Pt(double x, double y) => new(pad + (x - minX) * scale, pad + (maxY - y) * scale);
        Polygon Poly(IEnumerable<double[]> pts, Brush fill, Brush? stroke) { var pg = new Polygon { Fill = fill, Stroke = stroke, StrokeThickness = 1.5 }; foreach (var p in pts) pg.Points.Add(Pt(p[0], p[1])); return pg; }
        RoofCanvas.Children.Add(Poly(outline, new SolidColorBrush(ColorHelper.FromArgb(40, 128, 128, 128)), new SolidColorBrush(Colors.Gray)));
        var teal = new SolidColorBrush(ColorHelper.FromArgb(255, 0x0f, 0x5c, 0x63));
        var mods = roof["modules"]!.AsArray();
        foreach (var m in mods)
        {
            double x0 = m!["x0"]!.GetValue<double>(), y0 = m["y0"]!.GetValue<double>(), x1 = m["x1"]!.GetValue<double>(), y1 = m["y1"]!.GetValue<double>();
            var r = new Rectangle { Width = (x1 - x0) * scale, Height = (y1 - y0) * scale, Fill = teal, Stroke = new SolidColorBrush(Colors.White), StrokeThickness = 0.6 };
            var p = Pt(x0, y1);
            Canvas.SetLeft(r, p.X); Canvas.SetTop(r, p.Y);
            RoofCanvas.Children.Add(r);
        }
        foreach (var ob in roof["face"]!["obstacles"]!.AsArray())
            RoofCanvas.Children.Add(Poly(ob!.AsArray().Select(p => new[] { p![0]!.GetValue<double>(), p[1]!.GetValue<double>() }), new SolidColorBrush(Colors.DarkGray), null));
        RoofCount.Text = $"{mods.Count} {T("modules_fit")}";
    }

    private void ShowIssues(JsonArray issues)
    {
        IssuesList.ItemsSource = issues.Count == 0
            ? new List<string> { T("no_issues") }
            : issues.Select(i => (i!["severity"]!.GetValue<string>() == "error" ? "✖ " : "⚠ ") + i["message"]!.GetValue<string>()).ToList();
    }

    private void ShowResults(JsonNode? r)
    {
        NoResults.Visibility = r is null ? Visibility.Visible : Visibility.Collapsed;
        Kpis.Children.Clear(); MonthlyBars.Children.Clear(); LossGrid.Children.Clear(); LossGrid.RowDefinitions.Clear();
        if (r is null) return;
        var an = r["annual"]!;
        void Kpi(string label, double v, string fmt, string unit)
        {
            var sp = new StackPanel { Padding = new Thickness(12), CornerRadius = new CornerRadius(6), Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"], MinWidth = 160 };
            sp.Children.Add(new TextBlock { Text = label, Opacity = 0.7 });
            sp.Children.Add(new TextBlock { Text = $"{v.ToString(fmt, CultureInfo.CurrentCulture)} {unit}", Style = (Style)Application.Current.Resources["SubtitleTextBlockStyle"] });
            Kpis.Children.Add(sp);
        }
        Kpi(T("system_size"), an["dc_kwp"]!.GetValue<double>(), "N2", "kWp");
        Kpi(T("annual_energy"), an["ac_kwh"]!.GetValue<double>(), "N0", "kWh");
        Kpi(T("specific_yield"), an["specific_yield_kwh_kwp"]!.GetValue<double>(), "N0", "kWh/kWp");
        Kpi(T("performance_ratio"), an["performance_ratio"]!.GetValue<double>() * 100, "N1", "%");
        var months = r["monthly"]!.AsArray().Select(m => m!["ac_kwh"]!.GetValue<double>()).ToList();
        var max = Math.Max(1, months.Max());
        var names = CultureInfo.CurrentCulture.DateTimeFormat.AbbreviatedMonthNames;
        for (int i = 0; i < months.Count; i++)
        {
            var col = new StackPanel { VerticalAlignment = VerticalAlignment.Bottom, Width = 44 };
            col.Children.Add(new TextBlock { Text = months[i].ToString("N0", CultureInfo.CurrentCulture), FontSize = 11, HorizontalAlignment = HorizontalAlignment.Center });
            col.Children.Add(new Rectangle { Height = 170 * months[i] / max, Fill = new SolidColorBrush(ColorHelper.FromArgb(255, 0x0f, 0x5c, 0x63)), RadiusX = 2, RadiusY = 2 });
            col.Children.Add(new TextBlock { Text = names[i], FontSize = 11, HorizontalAlignment = HorizontalAlignment.Center });
            MonthlyBars.Children.Add(col);
        }
        int row = 0;
        foreach (var st in r["irradiance_kwh_m2"]!.AsArray())
        {
            var key = st!["key"]!.GetValue<string>();
            LossGrid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var cells = new[] {
                new TextBlock { Text = T("irr_" + key) },
                new TextBlock { Text = st["value"]!.GetValue<double>().ToString("N1", CultureInfo.CurrentCulture) + " kWh/m²", HorizontalAlignment = HorizontalAlignment.Right },
                new TextBlock { Text = key is "ghi" or "effective" ? "" : st["delta_pct"]!.GetValue<double>().ToString("N2", CultureInfo.CurrentCulture) + " %", HorizontalAlignment = HorizontalAlignment.Right, Opacity = 0.7 } };
            for (int c = 0; c < 3; c++) { Grid.SetRow(cells[c], row); Grid.SetColumn(cells[c], c); LossGrid.Children.Add(cells[c]); }
            row++;
        }
        LossGrid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(12) });
        row++;
        foreach (var st in r["energy_kwh"]!.AsArray())
        {
            var key = st!["key"]!.GetValue<string>();
            var total = key is "nominal" or "dc_out" or "grid";
            LossGrid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var cells = new[] {
                new TextBlock { Text = st["label"]!.GetValue<string>(), FontWeight = total ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal },
                new TextBlock { Text = st["value"]!.GetValue<double>().ToString("N0", CultureInfo.CurrentCulture), HorizontalAlignment = HorizontalAlignment.Right },
                new TextBlock { Text = total ? "" : st["delta_pct"]!.GetValue<double>().ToString("N2", CultureInfo.CurrentCulture) + " %", HorizontalAlignment = HorizontalAlignment.Right, Opacity = 0.7 } };
            for (int c = 0; c < 3; c++) { Grid.SetRow(cells[c], row); Grid.SetColumn(cells[c], c); LossGrid.Children.Add(cells[c]); }
            row++;
        }
    }

    // ---- Navigation ----
    private void OnNavigate(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        var tag = (args.SelectedItem as NavigationViewItem)?.Tag as string;
        ProjectPage.Visibility = tag == "project" ? Visibility.Visible : Visibility.Collapsed;
        SitePage.Visibility = tag == "site" ? Visibility.Visible : Visibility.Collapsed;
        DesignPage.Visibility = tag == "design" ? Visibility.Visible : Visibility.Collapsed;
        RoofPage.Visibility = tag == "roof" ? Visibility.Visible : Visibility.Collapsed;
        ResultsPage.Visibility = tag == "results" ? Visibility.Visible : Visibility.Collapsed;
        ReportPage.Visibility = tag == "report" ? Visibility.Visible : Visibility.Collapsed;
    }

    // ---- Edits ----
    private void Set(string pointer, object value)
    {
        if (_loading || _file is null) return;
        Try(() =>
        {
            var r = Api.Invoke("set_fields", new { file = _file, edits = new[] { new { pointer, value } } })!;
            _project = r["project"]; ShowProject(); ShowIssues(r["issues"]!.AsArray());
        });
    }

    private void OnTextCommit(object sender, RoutedEventArgs e) { if (sender is TextBox tb && tb.Tag is string ptr) Set(ptr, tb.Text); }
    private void OnNumber(NumberBox sender, NumberBoxValueChangedEventArgs args)
    {
        if (sender.Tag is not string ptr || double.IsNaN(args.NewValue) || args.NewValue == args.OldValue) return;
        if (ptr.EndsWith("modules_per_string") || ptr.EndsWith("/strings")) Set(ptr, (int)Math.Round(args.NewValue)); else Set(ptr, args.NewValue);
    }
    private void OnMounting(object sender, SelectionChangedEventArgs e)
    {
        if (MountingBox.SelectedIndex >= 0) Set("/simulation/mounting", MountingBox.SelectedIndex switch { 0 => "open_rack", 2 => "integrated", _ => "roof_mounted" });
    }
    private void OnMode(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || _file is null || ModeRadios.SelectedIndex < 0) return;
        if (Try(() => Api.Invoke("set_mode", new { file = _file, mode = ModeRadios.SelectedIndex == 1 ? "detailed" : "easy" }))) Reload();
    }
    private void OnUndo(object sender, RoutedEventArgs e) { if (_file is not null && Try(() => Api.Invoke("undo", new { file = _file }))) Reload(); }

    // ---- Weather ----
    private async void OnFetch(object sender, RoutedEventArgs e)
    {
        var dlg = new ContentDialog { Title = T("fetch_pvgis"), Content = T("fetch_consent"), PrimaryButtonText = "OK", CloseButtonText = T("cancel"), XamlRoot = Content.XamlRoot };
        if (await dlg.ShowAsync() != ContentDialogResult.Primary) return;
        await RunLong("fetch_weather_pvgis", new { file = _file, confirm_network = true });
    }
    private async void OnImport(object sender, RoutedEventArgs e)
    {
        var picker = new FileOpenPicker();
        picker.FileTypeFilter.Add(".epw"); picker.FileTypeFilter.Add(".json");
        InitPicker(picker);
        var f = await picker.PickSingleFileAsync();
        if (f is not null && Try(() => Api.Invoke("import_weather", new { file = _file, source = f.Path }))) Reload();
    }
    private void OnDemo(object sender, RoutedEventArgs e) { if (Try(() => Api.Invoke("use_demo_weather", new { file = _file }))) Reload(); }

    // ---- Simulation ----
    private async void OnSimulate(object sender, RoutedEventArgs e)
    {
        if (await RunLong("simulate", new { file = _file })) Nav.SelectedItem = NavResults;
    }
    private void OnCancel(object sender, RoutedEventArgs e) => _cts?.Cancel();

    private async Task<bool> RunLong(string method, object parameters)
    {
        _cts = new CancellationTokenSource();
        Progress.Value = 0; Progress.Visibility = Visibility.Visible; CancelButton.Visibility = Visibility.Visible; SimulateButton.IsEnabled = false;
        var progress = new Progress<double>(f => Progress.Value = f);
        var ok = true;
        try { await Task.Run(() => Api.Invoke(method, parameters, progress, _cts.Token)); }
        catch (OperationCanceledException) { ok = false; }
        catch (Exception ex) { ShowError(ex.Message); ok = false; }
        finally { Progress.Visibility = Visibility.Collapsed; CancelButton.Visibility = Visibility.Collapsed; SimulateButton.IsEnabled = true; }
        Reload();
        return ok;
    }

    // ---- Report ----
    private async void OnExport(object sender, RoutedEventArgs e)
    {
        var fmt = (FormatBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "pdf";
        var lang = (LangBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "en";
        var picker = new FileSavePicker { SuggestedFileName = Api.Str(_project?["name"]) };
        picker.FileTypeChoices.Add(fmt.ToUpperInvariant(), new List<string> { "." + fmt });
        InitPicker(picker);
        var f = await picker.PickSaveFileAsync();
        if (f is null) return;
        var designer = DesignerBox.Text;
        try
        {
            await Task.Run(() => Api.Invoke("export_report", new { file = _file, output = f.Path, format = fmt, lang, designer }));
            ShowInfo($"{T("report_saved")}: {f.Path}");
            await Windows.System.Launcher.LaunchFileAsync(f);
        }
        catch (Exception ex) { ShowError(ex.Message); }
    }
}
