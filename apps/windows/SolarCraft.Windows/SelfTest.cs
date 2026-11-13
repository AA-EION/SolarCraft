// SPDX-License-Identifier: GPL-3.0-or-later
using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace SolarCraft;

/// <summary>Headless checks run by CI on each architecture (no window is created).</summary>
public static unsafe class SelfTest
{
    public static int Run(string outPath)
    {
        var sb = new StringBuilder();
        int failures = 0;
        void Check(bool ok, string msg) { sb.AppendLine((ok ? "PASS " : "FAIL ") + msg); if (!ok) failures++; }
        sb.AppendLine($"solarcraft selftest arch={RuntimeInformation.ProcessArchitecture} core={Core.Version} os={RuntimeInformation.OSDescription}");
        var input = new ScQuickInput { Latitude = 4.711, Longitude = -74.072, ElevationM = 2640, TiltDeg = 10, AzimuthDeg = 180, Modules = 10, Year = 2026 };
        try
        {
            var calls = 0;
            var (s, h) = Core.QuickAnnual(input, new SyncProgress(_ => calls++));
            Check(s.Steps == 8760 && h.Length == 8760 && s.SpecificYieldKwhPerKwp > 1500 && calls > 0, $"annual run yield={s.SpecificYieldKwhPerKwp:F1} progress_calls={calls}");
        }
        catch (Exception e) { Check(false, "annual run threw " + e.Message); }
        try { using var cts = new CancellationTokenSource(); cts.Cancel(); Core.QuickAnnual(input, null, cts.Token); Check(false, "cancel"); }
        catch (OperationCanceledException) { Check(true, "cancel"); }
        var buf = new double[131072];
        fixed (double* w = buf) Core.sc_fill_probe(w, (nuint)buf.Length); // warm-up
        var sw = Stopwatch.StartNew();
        double sum; fixed (double* p = buf) sum = Core.sc_fill_probe(p, (nuint)buf.Length);
        sw.Stop();
        Check(sum == 4294934528 && sw.Elapsed.TotalMilliseconds < 5, $"1 MiB FFI probe {sw.Elapsed.TotalMilliseconds:F3} ms");
        Check(Core.sc_panic_probe() == Core.InternalError, "core panic is contained");
        // Full offline workflow through the JSON API used by the UI.
        var dir = Path.Combine(Path.GetTempPath(), "sc-selftest-" + Environment.ProcessId);
        Directory.CreateDirectory(dir);
        var file = Path.Combine(dir, "t.solarcraft");
        try
        {
            Api.Invoke("create_project", new { file, name = "Selftest", latitude = 6.2442, longitude = -75.5812, target_kwp = 4.0 });
            var set = Api.Invoke("set_fields", new { file, edits = new[] { new { pointer = "/arrays/0/tilt", value = 12.0 } } })!;
            Check(Api.Num(set["project"]!["arrays"]![0]!["tilt"]) == 12 && Api.IsUser(set["project"]!["arrays"]![0]!["tilt"]), "set_fields");
            var roof = Api.Invoke("set_roof", new { file, outline = new[] { new[] { 0.0, 0.0 }, new[] { 11.0, 0.0 }, new[] { 8.0, 5.0 }, new[] { 3.0, 5.0 } }, obstacles = MainWindow.ParseObstacles("5,2,0.8,0.8,2.0"), obstacle_heights = MainWindow.ParseObstacleHeights("5,2,0.8,0.8,2.0") })!;
            var fit = roof["modules_fit"]!.GetValue<int>();
            var stored = Api.Invoke("get_project", new { file })!["arrays"]![0]!["roof"]!["modules"]!.AsArray().Count;
            Check(fit >= 8 && stored == fit, $"roof layout ({fit} modules)");
            Api.Invoke("use_demo_weather", new { file });
            var calls = 0;
            Api.Invoke("simulate", new { file }, new SyncProgress(_ => calls++));
            var e = Api.Invoke("explain", new { file })!;
            var kwh = e["annual"]!["ac_kwh"]!.GetValue<double>();
            Check(e["monthly"]!.AsArray().Count == 12 && kwh > 1000 && calls > 0, $"workflow simulate/explain {kwh:F0} kWh");
            var shade = e["irradiance_kwh_m2"]!.AsArray().First(x => x!["key"]!.GetValue<string>() == "shading")!["delta_pct"]!.GetValue<double>();
            Check(shade < 0, $"near shading counted ({shade:F2} %)");
            var rep = Api.Invoke("export_report", new { file, output = Path.Combine(dir, "r.pdf"), format = "pdf", lang = "es" })!;
            Check(rep["bytes"]!.GetValue<int>() > 20000, "report PDF");
            Check(Api.Invoke("ui_strings", new { lang = "es" })!["simulate"]!.GetValue<string>() == "Simular", "UI strings (es)");
            try { Api.Invoke("fetch_weather_pvgis", new { file }); Check(false, "consent enforced"); }
            catch (CoreError ce) { Check(ce.Code == "CONSENT_REQUIRED", "consent enforced"); }
        }
        catch (Exception ex) { Check(false, "workflow: " + ex.Message); }
        finally { try { Directory.Delete(dir, true); } catch { } }
        File.WriteAllText(outPath, sb.ToString());
        return failures == 0 ? 0 : 1;
    }

    sealed class SyncProgress(Action<double> a) : IProgress<double> { public void Report(double v) => a(v); }
}
