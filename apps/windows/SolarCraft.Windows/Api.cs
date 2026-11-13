// SPDX-License-Identifier: GPL-3.0-or-later
using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;

namespace SolarCraft;

public sealed class CoreError(string code, string message) : Exception(message) { public string Code { get; } = code; }

/// <summary>JSON application API over the C ABI (sc_invoke). Same contract as the MCP tools.</summary>
public static unsafe class Api
{
    [DllImport("solarcraft_ffi", CallingConvention = CallingConvention.Cdecl)]
    static extern IntPtr sc_invoke(byte* request, delegate* unmanaged[Cdecl]<double, IntPtr, int> progress, IntPtr user);
    [DllImport("solarcraft_ffi", CallingConvention = CallingConvention.Cdecl)]
    static extern void sc_string_free(IntPtr s);

    sealed class Job(IProgress<double>? p, CancellationToken t) { public readonly IProgress<double>? P = p; public readonly CancellationToken T = t; }

    [UnmanagedCallersOnly(CallConvs = new[] { typeof(CallConvCdecl) })]
    static int OnProgress(double f, IntPtr user)
    {
        var job = (Job)GCHandle.FromIntPtr(user).Target!;
        job.P?.Report(f);
        return job.T.IsCancellationRequested ? 0 : 1;
    }

    /// <summary>Invoke a method; returns the "result" node or throws <see cref="CoreError"/>.</summary>
    public static JsonNode? Invoke(string method, object? parameters = null, IProgress<double>? progress = null, CancellationToken token = default)
    {
        var request = JsonSerializer.SerializeToUtf8Bytes(new JsonObject { ["method"] = method, ["params"] = parameters is null ? new JsonObject() : JsonSerializer.SerializeToNode(parameters) });
        var withNul = new byte[request.Length + 1];
        request.CopyTo(withNul, 0);
        var handle = GCHandle.Alloc(new Job(progress, token));
        IntPtr raw;
        try { fixed (byte* p = withNul) raw = sc_invoke(p, &OnProgress, GCHandle.ToIntPtr(handle)); }
        finally { handle.Free(); }
        string text;
        try { text = Marshal.PtrToStringUTF8(raw) ?? "{}"; } finally { sc_string_free(raw); }
        var resp = JsonNode.Parse(text)!;
        if (resp["ok"]?.GetValue<bool>() == true) return resp["result"];
        var code = resp["error"]?["code"]?.GetValue<string>() ?? "ERROR";
        if (code == "CANCELLED") throw new OperationCanceledException();
        throw new CoreError(code, resp["error"]?["message"]?.GetValue<string>() ?? "Unknown error");
    }

    /// <summary>Value of a project field {value, origin}.</summary>
    public static double Num(JsonNode? field) => field?["value"]?.GetValue<double>() ?? double.NaN;
    public static string Str(JsonNode? field) => field?["value"]?.ToString() ?? "";
    public static bool IsUser(JsonNode? field) => field?["origin"]?.GetValue<string>() == "user";
}
