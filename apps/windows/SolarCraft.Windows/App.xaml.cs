// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI.Xaml;

namespace SolarCraft;

public partial class App : Application
{
    private Window? _window;
    public App() => InitializeComponent();
    protected override void OnLaunched(LaunchActivatedEventArgs args) { _window = new MainWindow(); _window.Activate(); }
}
