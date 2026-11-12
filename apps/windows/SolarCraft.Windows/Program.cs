// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using System;
using System.Linq;
using System.Threading;

namespace SolarCraft;

public static class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        int i = Array.IndexOf(args, "--selftest");
        if (i >= 0) return SelfTest.Run(i + 1 < args.Length ? args[i + 1] : "selftest.txt");
        WinRT.ComWrappersSupport.InitializeComWrappers();
        Application.Start(p =>
        {
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = p;
            new App();
        });
        return 0;
    }
}
