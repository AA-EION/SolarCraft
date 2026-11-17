// SPDX-License-Identifier: GPL-3.0-or-later
using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json.Nodes;
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
        // Plant workflow: stringing, consumption, DC-coupled battery with reserve, tariff, variants, report contents.
        try
        {
            var f = BuildDemo(Path.Combine(dir, "plant"));
            var b = Api.Invoke("results_summary", new { file = f })!["balance"]!;
            var dis = b["batt_discharge_kwh"]!.GetValue<double>();
            Check(dis > 100 && b["load_kwh"]!.GetValue<double>() > 4000 && b["max_abs_residual_wh"]!.GetValue<double>() < 1e-3, $"battery dispatch and closed balance ({dis:F0} kWh discharged)");
            var day = Api.Invoke("results_hours", new { file = f, day_of_year = 172 })!["hours"]!.AsArray();
            Check(day.Count == 24 && day.Count(h => h!["outage"]!.GetValue<bool>()) == 5, "hourly inspection with outage hours");
            Check(Api.Invoke("cash_flow", new { file = f })!["years"]!.AsArray().Count == 21, "cash flow 20 years");
            Check(Api.Invoke("compare_variants", new { file = f })!["variants"]!.AsArray().Count == 2, "variant comparison");
            var sections = Api.Invoke("report_preview", new { file = f, lang = "es" })!["sections"]!.AsArray().Count;
            Check(sections >= 4, $"report plant sections ({sections})");
            var mods = Api.Invoke("scene_shading", new { file = f })!["fields"]![0]!["modules"]!.AsArray().Count;
            Check(mods == 15, $"3D scene: per-module shading of the bound field ({mods} modules)");
            var img = Api.Invoke("scene_image", new { file = f, camera = new { yaw_deg = 25.0, pitch_deg = 35.0, width = 400.0, height = 300.0 } })!["png_base64"]!.GetValue<string>();
            Check(img.StartsWith("iVBORw0KGgo"), "3D viewport frame (PNG)");
            var ifc = Path.Combine(dir, "scene.ifc");
            var ex = Api.Invoke("scene_export", new { file = f, output = ifc })!;
            Check(File.Exists(ifc) && ex["fidelity"]!["solarcraft_metadata"]!.GetValue<bool>(), "IFC export with SolarCraft metadata");
        }
        catch (Exception ex) { Check(false, "plant workflow: " + ex.Message); }
        finally { try { Directory.Delete(dir, true); } catch { } }
        File.WriteAllText(outPath, sb.ToString());
        return failures == 0 ? 0 : 1;
    }

    /// <summary>Builds a complete, simulated demo project offline through the JSON API (self-test and screenshots).</summary>
    public static string BuildDemo(string dir)
    {
        Directory.CreateDirectory(dir);
        var file = Path.Combine(dir, "Casa Usaquén.solarcraft");
        if (File.Exists(file)) File.Delete(file);
        void Call(string method, string json) { var o = JsonNode.Parse(json)!.AsObject(); o["file"] = file; Api.Invoke(method, o); }
        Api.Invoke("create_project", new { file, name = "Casa Usaquén", latitude = 4.711, longitude = -74.072, target_kwp = 6.0 });
        Call("set_mode", "{\"mode\":\"detailed\"}");
        Call("use_demo_weather", "{}");
        var ids = Api.Invoke("add_generic_storage_equipment", new { file })!["ids"]!.AsArray().Select(i => i!.GetValue<string>()).ToList();
        Api.Invoke("set_fields", new { file, edits = new object[] { new { pointer = "/arrays/0/inverter_id", value = (object)ids[0] }, new { pointer = "/arrays/0/tilt", value = (object)15.0 } } });
        Call("set_roof", "{\"outline\":[[0,0],[12,0],[12,6],[0,6]],\"obstacles\":[[[5,4],[5.8,4],[5.8,4.8],[5,4.8]]],\"obstacle_heights\":[1.5],\"params\":{\"setback\":0.4,\"obstacle_clearance\":0.3}}");
        var unit = Api.Invoke("stringing", new { file })!["inverters"]![0]!["unit"]!["id"]!.GetValue<string>();
        Call("set_section", "{\"section\":\"grid\",\"value\":{\"feed_in\":\"surplus\",\"export_limit_kw\":4.0,\"phases\":1,\"voltage_v\":120}}");
        Call("set_section", "{\"section\":\"loads\",\"value\":{\"profile\":{\"kind\":\"synthetic\",\"profile\":\"residential\"},\"annual_kwh\":4200,\"critical_fraction\":0.3}}");
        Call("set_section", "{\"section\":\"storage\",\"value\":{\"battery_id\":\"" + ids[1] + "\",\"units\":1,\"coupling\":{\"kind\":\"dc_hybrid\",\"unit_id\":\"" + unit + "\"},\"model\":\"detailed\",\"soc_min_pct\":5,\"soc_max_pct\":100,\"initial_soc_pct\":50,\"backup_reserve_pct\":30,\"grid_services_pct\":0,\"charge_priority\":[\"backup\",\"self_consumption\",\"grid_services\"],\"grid_charge_backup\":false,\"grid_service\":{\"kind\":\"none\"},\"outages\":[{\"start_hour\":4107,\"hours\":5}]}}");
        Call("set_section", "{\"section\":\"tariff\",\"value\":{\"name\":\"TOU\",\"currency\":\"COP\",\"import_price\":800,\"tou\":[{\"name\":\"Peak\",\"price\":1100,\"start_hour\":18,\"end_hour\":22,\"weekdays\":true,\"weekends\":true,\"months\":[]}],\"export\":{\"kind\":\"fixed\",\"price\":300},\"fixed_per_month\":12000,\"demand_charge_per_kw_month\":0,\"import_escalation_pct\":4,\"export_escalation_pct\":2}}");
        Call("set_section", "{\"section\":\"economics\",\"value\":{\"horizon_years\":20,\"discount_rate_pct\":9,\"cost_fixed\":3000000,\"cost_per_kwp\":3500000,\"cost_per_battery_kwh\":1800000,\"om_pct_of_investment\":1,\"om_escalation_pct\":4,\"inverter_replacement_year\":13,\"inverter_replacement_cost\":4000000,\"battery_replacement_year\":12,\"battery_replacement_cost\":0,\"degradation_pct_per_year\":0.4,\"subsidy\":0,\"loan\":null}}");
        // 3D scene: house with a roof window, chimney and tree; a field on the south face fitted to array-1.
        Call("scene_edit", "{\"edits\":[{\"op\":\"add_object\",\"name\":\"Casa\",\"layer\":\"building\",\"kind\":{\"type\":\"building\",\"footprint\":[[0,0],[16,0],[16,10],[0,10]],\"eave_height\":5.5,\"roof\":{\"type\":\"gable\",\"pitch_deg\":20},\"openings\":[[[1,0.8],[2.2,0.8],[2.2,2.2],[1,2.2]]]}},"
            + "{\"op\":\"add_object\",\"name\":\"Chimenea\",\"layer\":\"obstacles\",\"kind\":{\"type\":\"block\",\"center\":[13.5,3],\"base_z\":5,\"length\":0.7,\"width\":0.7,\"height\":2.4}},"
            + "{\"op\":\"add_object\",\"name\":\"Urapán\",\"layer\":\"vegetation\",\"kind\":{\"type\":\"tree\",\"center\":[6,-9],\"trunk_height\":3,\"trunk_radius\":0.3,\"crown_radius\":2.8,\"crown_height\":6,\"transparency\":0.25}},"
            + "{\"op\":\"add_field\",\"name\":\"Cubierta sur\",\"mount\":{\"type\":\"roof\",\"face\":\"bld-1/roof-0\"}}]}");
        Call("scene_bind", "{\"field\":\"fld-4\",\"array_id\":\"array-1\",\"match_array\":true}");
        Call("simulate", "{}");
        Call("create_variant", "{\"name\":\"Reserve 30 %\"}");
        return file;
    }

    sealed class SyncProgress(Action<double> a) : IProgress<double> { public void Report(double v) => a(v); }
}
