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
        NavProject.Content = T("project"); NavSite.Content = T("site_weather"); NavDesign.Content = T("design"); NavResults.Content = T("results"); NavReport.Content = T("report");
        EmptyText.Text = T("no_project");
        NameBox.Header = T("name"); CustomerBox.Header = T("customer"); AddressBox.Header = T("address");
        LatBox.Header = T("latitude"); LonBox.Header = T("longitude"); ElevBox.Header = T("elevation"); AlbedoBox.Header = T("albedo");
        WeatherHeader.Text = T("weather"); FetchButton.Content = T("fetch_pvgis"); ImportButton.Content = T("import_weather"); DemoButton.Content = T("demo_weather"); DemoWarning.Text = T("demo_warning");
        ModeRadios.Header = T("mode"); EasyRadio.Content = T("easy"); DetailedRadio.Content = T("detailed"); ModeHint.Text = T("mode_hint");
        KwpBox.Header = T("target_kwp"); TiltBox.Header = T("tilt"); AzimuthBox.Header = T("azimuth"); MountingBox.Header = T("mounting");
        MountingBox.ItemsSource = new[] { T("open_rack"), T("roof_mounted"), T("integrated") };
        MpsBox.Header = T("modules_per_string"); StringsBox.Header = T("strings"); IssuesHeader.Text = T("issues");
        NoResults.Text = T("no_results"); MonthlyHeader.Text = T("monthly_energy"); LossesHeader.Text = T("losses");
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
        _loading = false;
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
