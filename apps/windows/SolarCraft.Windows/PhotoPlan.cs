// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Windows.Storage.Pickers;
using Windows.Storage.Streams;
using System.Runtime.InteropServices.WindowsRuntime;

namespace SolarCraft;

public sealed partial class MainWindow
{
    private static async Task<BitmapImage> PhotoBitmap(string data)
    {
        using var stream = new InMemoryRandomAccessStream();
        await stream.WriteAsync(Convert.FromBase64String(data).AsBuffer());
        stream.Seek(0);
        var bitmap = new BitmapImage();
        await bitmap.SetSourceAsync(stream);
        return bitmap;
    }

    private async void OnPhotoPlan(object sender, RoutedEventArgs e)
    {
        if (_file is not string file) return;
        var picker = new FileOpenPicker();
        foreach (var ext in new[] { ".png", ".jpg", ".jpeg" }) picker.FileTypeFilter.Add(ext);
        InitPicker(picker);
        var picked = await picker.PickSingleFileAsync();
        if (picked is null) return;
        try
        {
            var source = picked.Path;
            var original = await Task.Run(() => Api.Invoke("photo_source", new { source }));
            var sourceHash = S(original, "source_hash");
            var bitmap = await PhotoBitmap(S(original, "png_base64"));
            var scale = Math.Min(480.0 / bitmap.PixelWidth, 285.0 / bitmap.PixelHeight);
            var width = bitmap.PixelWidth * scale;
            var height = bitmap.PixelHeight * scale;
            var image = new Image { Source = bitmap, Stretch = Stretch.Fill, Width = width, Height = height };
            var overlay = new Canvas { Width = width, Height = height, Background = new SolidColorBrush(Colors.Transparent) };
            var host = new Grid { Width = width, Height = height, HorizontalAlignment = HorizontalAlignment.Left, Children = { image, overlay } };
            var points = new List<double[]>();
            var roofWidth = Num(T("photo_width"), 10, 190);
            var roofDepth = Num(T("photo_depth"), 5, 190);
            var instructions = Note(T("photo_instructions"));
            var status = Note("");
            var previewImage = new Image { MaxWidth = 480, MaxHeight = 220, Stretch = Stretch.Uniform, Visibility = Visibility.Collapsed };
            var preview = new Button { Content = T("photo_preview"), IsEnabled = false };
            var clear = new Button { Content = T("photo_reset") };
            var panel = new StackPanel { Spacing = 10, Children = { instructions, host, Row(roofWidth, roofDepth), Row(clear, preview), status, previewImage } };
            var dialog = new ContentDialog {
                XamlRoot = RoofPage.XamlRoot, Title = T("photo_plan"), Content = new ScrollViewer { Content = panel },
                PrimaryButtonText = T("photo_apply"), CloseButtonText = T("cancel"), IsPrimaryButtonEnabled = false,
            };
            bool busy = false;
            void Refresh()
            {
                preview.IsEnabled = !busy && points.Count == 4 && roofWidth.Value > 0 && roofDepth.Value > 0;
                dialog.IsPrimaryButtonEnabled = preview.IsEnabled;
                clear.IsEnabled = !busy;
                roofWidth.IsEnabled = !busy;
                roofDepth.IsEnabled = !busy;
            }
            overlay.PointerPressed += (_, args) =>
            {
                if (busy || points.Count == 4) return;
                var pos = args.GetCurrentPoint(overlay).Position;
                points.Add(new[] { Math.Clamp(pos.X / width, 0, 1), Math.Clamp(pos.Y / height, 0, 1) });
                var label = new Border { Width = 22, Height = 22, Background = new SolidColorBrush(Colors.DarkCyan),
                    CornerRadius = new CornerRadius(11), IsHitTestVisible = false,
                    Child = new TextBlock { Text = points.Count.ToString(), Foreground = new SolidColorBrush(Colors.White),
                        HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center } };
                Canvas.SetLeft(label, pos.X - 11); Canvas.SetTop(label, pos.Y - 11);
                overlay.Children.Add(label);
                Refresh();
            };
            clear.Click += (_, _) =>
            {
                points.Clear(); overlay.Children.Clear(); previewImage.Visibility = Visibility.Collapsed; status.Text = "";
                Refresh();
            };
            roofWidth.ValueChanged += (_, _) => Refresh();
            roofDepth.ValueChanged += (_, _) => Refresh();
            async Task<bool> Run(bool previewOnly)
            {
                var request = new { file, source, preview = previewOnly,
                    options = new { source_hash = sourceHash, control = new { corners = points.ToArray(), width_m = roofWidth.Value, depth_m = roofDepth.Value } } };
                busy = true;
                Refresh();
                try
                {
                    var result = await Task.Run(() => Api.Invoke("photo_layout", request));
                    status.Text = $"{Arr(result?["roof"]?["modules"]).Count} {T("modules_fit")}";
                    if (previewOnly)
                    {
                        previewImage.Source = await PhotoBitmap(S(result, "png_base64"));
                        previewImage.Visibility = Visibility.Visible;
                    }
                    return true;
                }
                catch (Exception error) { status.Text = error.Message; return false; }
                finally { busy = false; Refresh(); }
            }
            preview.Click += async (_, _) => await Run(true);
            dialog.PrimaryButtonClick += async (_, args) =>
            {
                var deferral = args.GetDeferral();
                try { args.Cancel = !await Run(false); }
                finally { deferral.Complete(); }
            };
            if (await dialog.ShowAsync() == ContentDialogResult.Primary) Reload();
        }
        catch (Exception error) { ShowError(error.Message); }
    }
}
