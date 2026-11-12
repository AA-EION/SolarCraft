// SPDX-License-Identifier: GPL-3.0-or-later
using System;
using System.Diagnostics;
using System.IO;
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
        File.WriteAllText(outPath, sb.ToString());
        return failures == 0 ? 0 : 1;
    }

    sealed class SyncProgress(Action<double> a) : IProgress<double> { public void Report(double v) => a(v); }
}
