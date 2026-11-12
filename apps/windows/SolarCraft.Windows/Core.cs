// SPDX-License-Identifier: GPL-3.0-or-later
using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Threading;

namespace SolarCraft;

[StructLayout(LayoutKind.Sequential)]
public struct ScQuickInput { public double Latitude, Longitude, ElevationM, TiltDeg, AzimuthDeg; public uint Modules, Year; }

[StructLayout(LayoutKind.Sequential)]
public struct ScAnnualSummary { public double AnnualAcKwh, SpecificYieldKwhPerKwp, DcKwp; public uint Steps; }

public sealed class CoreException(int code) : Exception($"SolarCraft core error {code}") { public int Code { get; } = code; }

/// <summary>C# façade over the C ABI in crates/ffi/include/solarcraft.h.</summary>
public static unsafe class Core
{
    const string Lib = "solarcraft_ffi";
    public const int Ok = 0, Cancelled = 1, InvalidArgument = 2, InternalError = 3;

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)] static extern IntPtr sc_version();
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    static extern int sc_quick_annual(ScQuickInput* input, ScAnnualSummary* output, double* hourly, nuint len,
        delegate* unmanaged[Cdecl]<double, IntPtr, int> progress, IntPtr user);
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)] public static extern double sc_fill_probe(double* buf, nuint len);
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)] public static extern int sc_panic_probe();

    public static string Version => Marshal.PtrToStringUTF8(sc_version()) ?? "?";

    sealed class Job(IProgress<double>? progress, CancellationToken token) { public readonly IProgress<double>? P = progress; public readonly CancellationToken T = token; }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    static int OnProgress(double fraction, IntPtr user)
    {
        var job = (Job)GCHandle.FromIntPtr(user).Target!;
        job.P?.Report(fraction);
        return job.T.IsCancellationRequested ? 0 : 1;
    }

    public static (ScAnnualSummary Summary, double[] Hourly) QuickAnnual(ScQuickInput input, IProgress<double>? progress = null, CancellationToken token = default)
    {
        var hourly = new double[8784];
        ScAnnualSummary output;
        var handle = GCHandle.Alloc(new Job(progress, token));
        try
        {
            int rc;
            fixed (double* h = hourly)
                rc = sc_quick_annual(&input, &output, h, (nuint)hourly.Length, &OnProgress, GCHandle.ToIntPtr(handle));
            if (rc == Cancelled) throw new OperationCanceledException(token);
            if (rc != Ok) throw new CoreException(rc);
        }
        finally { handle.Free(); }
        return (output, hourly[..(int)output.Steps]);
    }
}
