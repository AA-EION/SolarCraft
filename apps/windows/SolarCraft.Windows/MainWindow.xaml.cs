// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml;
using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace SolarCraft;

public sealed partial class MainWindow : Window
{
    private CancellationTokenSource? _cts;

    public MainWindow()
    {
        InitializeComponent();
        CoreVersion.Text = "Core " + Core.Version;
    }

    private async void OnRun(object sender, RoutedEventArgs e)
    {
        var input = new ScQuickInput { Latitude = Lat.Value, Longitude = Lon.Value, ElevationM = Elev.Value, TiltDeg = Tilt.Value, AzimuthDeg = Azimuth.Value, Modules = (uint)Modules.Value, Year = 2026 };
        _cts = new CancellationTokenSource();
        RunButton.IsEnabled = false; CancelButton.IsEnabled = true;
        var progress = new Progress<double>(f => Progress.Value = f);
        try
        {
            var (s, hourly) = await Task.Run(() => Core.QuickAnnual(input, progress, _cts.Token));
            Summary.Text = $"{s.AnnualAcKwh:N0} kWh/yr · {s.SpecificYieldKwhPerKwp:N0} kWh/kWp · {s.DcKwp:N2} kWp";
            int[] days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            int h = 0;
            Monthly.ItemsSource = days.Select((d, m) => { var kwh = hourly.Skip(h).Take(d * 24).Sum() / 1000; h += d * 24; return $"{m + 1:00}: {kwh,8:N0} kWh"; }).ToList();
        }
        catch (OperationCanceledException) { Summary.Text = "Cancelled"; }
        catch (Exception ex) { Summary.Text = ex.Message; }
        finally { RunButton.IsEnabled = true; CancelButton.IsEnabled = false; }
    }

    private void OnCancel(object sender, RoutedEventArgs e) => _cts?.Cancel();
}
