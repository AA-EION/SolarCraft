// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using System;
using System.Linq;
using System.Threading;

namespace SolarCraft;

public static class Program
{
    /// <summary>Set by <c>--screenshots &lt;dir&gt;</c>: render every page of a simulated demo project, then exit.</summary>
    public static string? ScreenshotDir;

    [STAThread]
    public static int Main(string[] args)
    {
        int i = Array.IndexOf(args, "--selftest");
        if (i >= 0) return SelfTest.Run(i + 1 < args.Length ? args[i + 1] : "selftest.txt");
        int k = Array.IndexOf(args, "--screenshots");
        if (k >= 0 && k + 1 < args.Length) ScreenshotDir = args[k + 1];
        WinRT.ComWrappersSupport.InitializeComWrappers();
        Application.Start(p =>
        {
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = p;
            new App();
        });
        return Environment.ExitCode;
    }
}
