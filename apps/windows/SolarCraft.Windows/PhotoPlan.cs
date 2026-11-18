// SPDX-License-Identifier: GPL-3.0-or-later
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
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

    private void PopulatePhotoArrays()
    {
        var index = Math.Max(0, PhotoArrayBox.SelectedIndex);
        var arrays = Arr(_project?["arrays"]);
        PhotoArrayBox.ItemsSource = arrays.Select(a => Api.Str(a?["name"])).ToList();
        PhotoArrayBox.SelectedIndex = arrays.Count > 0 ? Math.Min(index, arrays.Count - 1) : -1;
        RefreshPhotoArray();
    }

    private void OnPhotoArray(object sender, SelectionChangedEventArgs e) => RefreshPhotoArray();

    private void RefreshPhotoArray()
    {
        var index = PhotoArrayBox.SelectedIndex;
        if (SavedPhotoButton is null) return;
        var arrays = Arr(_project?["arrays"]);
        var array = index >= 0 && index < arrays.Count ? arrays[index] : null;
        var exists = Arr(_project?["roof_photos"]).Any(p => S(p, "array_id") == S(array, "id"));
        SavedPhotoButton.Visibility = exists ? Visibility.Visible : Visibility.Collapsed;
    }

    private async void OnPhotoPlan(object sender, RoutedEventArgs e)
    {
        if (_file is not string file || PhotoArrayBox.SelectedIndex < 0) return;
        var array = PhotoArrayBox.SelectedIndex;
        var picker = new FileOpenPicker();
        foreach (var ext in new[] { ".png", ".jpg", ".jpeg" }) picker.FileTypeFilter.Add(ext);
        InitPicker(picker);
        var picked = await picker.PickSingleFileAsync();
        if (picked is not null) await EditPhoto(file, array, picked.Path);
    }

    private async void OnSavedPhoto(object sender, RoutedEventArgs e)
    {
        if (_file is string file && PhotoArrayBox.SelectedIndex >= 0)
            await EditPhoto(file, PhotoArrayBox.SelectedIndex, null);
    }

    private async Task EditPhoto(string file, int array, string? source)
    {
        try
        {
            var original = await Task.Run(() => source is null
                ? Api.Invoke("photo_saved", new { file, array })
                : Api.Invoke("photo_source", new { source }));
            var sourceHash = S(original, "source_hash");
            var bitmap = await PhotoBitmap(S(original, "png_base64"));
            var scale = Math.Min(430.0 / bitmap.PixelWidth, 245.0 / bitmap.PixelHeight);
            var width = bitmap.PixelWidth * scale;
            var height = bitmap.PixelHeight * scale;
            var image = new Image { Source = bitmap, Stretch = Stretch.Fill, Width = width, Height = height };
            var overlay = new Canvas { Width = width, Height = height, Background = new SolidColorBrush(Colors.Transparent) };
            overlay.Clip = new RectangleGeometry { Rect = new Rect(0, 0, width, height) };
            var host = new Grid { Width = width, Height = height, HorizontalAlignment = HorizontalAlignment.Left, Children = { image, overlay } };
            static List<double[]> ReadPoints(JsonNode? ring) => Arr(ring).Select(p => Arr(p).Select(v => v!.GetValue<double>()).ToArray()).ToList();
            var control = original?["control"];
            var corners = ReadPoints(control?["corners"]);
            var outline = ReadPoints(original?["outline"]);
            var obstacles = Arr(original?["obstacles"]).Select(ReadPoints).ToList();
            var heights = Arr(original?["obstacle_heights"]).Select(v => v!.GetValue<double>()).ToList();
            while (heights.Count < obstacles.Count) heights.Add(1);
            var roofWidth = Num(T("photo_width"), N(control, "width_m", 10), 198);
            var roofDepth = Num(T("photo_depth"), N(control, "depth_m", 5), 198);
            var obstacleHeight = Num(T("photo_height"), 1, 160);
            obstacleHeight.Minimum = 0;
            obstacleHeight.Maximum = 30;
            var trace = new ComboBox { Width = 235, HorizontalAlignment = HorizontalAlignment.Left };
            var instructions = Note("");
            var status = Note("");
            var previewImage = new Image { MaxWidth = 430, MaxHeight = 210, Stretch = Stretch.Uniform, Visibility = Visibility.Collapsed };
            var preview = new Button { Content = T("photo_preview") };
            var clear = new Button { Content = T("photo_clear_trace") };
            var undo = new Button { Content = T("undo") };
            var add = new Button { Content = T("photo_add_obstacle") };
            var remove = new Button { Content = T("photo_remove_obstacle") };
            var panel = new StackPanel { Spacing = 9, Children = {
                Row(trace, obstacleHeight), instructions, host, Row(undo, clear), Row(add, remove),
                Row(roofWidth, roofDepth), preview, status, previewImage } };
            var dialog = new ContentDialog {
                XamlRoot = RoofPage.XamlRoot, Title = T("photo_plan"),
                Content = new ScrollViewer { Content = panel, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto },
                PrimaryButtonText = T("photo_apply"), CloseButtonText = T("cancel"),
            };
            bool busy = false;
            bool updating = false;
            int active = 0;
            List<double[]> Points() => active switch { 0 => corners, 1 => outline, _ => obstacles[active - 2] };
            void InvalidatePreview()
            {
                previewImage.Visibility = Visibility.Collapsed;
                status.Text = "";
            }
            void Mark(List<double[]> points, Windows.UI.Color color, bool reference)
            {
                var line = new Polyline { Stroke = new SolidColorBrush(color), StrokeThickness = 2, IsHitTestVisible = false };
                foreach (var p in points) line.Points.Add(new Point(p[0] * width, p[1] * height));
                if (points.Count >= 3 && (!reference || points.Count == 4)) line.Points.Add(line.Points[0]);
                overlay.Children.Add(line);
                for (var i = 0; i < points.Count; i++)
                {
                    var p = points[i];
                    var size = reference ? 22 : 8;
                    var dot = new Border { Width = size, Height = size, Background = new SolidColorBrush(color),
                        CornerRadius = new CornerRadius(size / 2), IsHitTestVisible = false };
                    if (reference)
                        dot.Child = new TextBlock { Text = (i + 1).ToString(), Foreground = new SolidColorBrush(Colors.Black),
                            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
                    Canvas.SetLeft(dot, p[0] * width - size / 2); Canvas.SetTop(dot, p[1] * height - size / 2);
                    overlay.Children.Add(dot);
                }
            }
            void Refresh()
            {
                var complete = corners.Count == 4 && (outline.Count == 0 || outline.Count >= 3) && obstacles.All(o => o.Count >= 3);
                preview.IsEnabled = dialog.IsPrimaryButtonEnabled = !busy && complete && roofWidth.Value > 0 && roofDepth.Value > 0;
                trace.IsEnabled = !busy;
                clear.IsEnabled = undo.IsEnabled = !busy && Points().Count > 0;
                add.IsEnabled = !busy && corners.Count == 4 && obstacles.Count < 48;
                remove.IsEnabled = !busy && active >= 2;
                roofWidth.IsEnabled = roofDepth.IsEnabled = obstacleHeight.IsEnabled = !busy;
                obstacleHeight.Visibility = active >= 2 ? Visibility.Visible : Visibility.Collapsed;
                instructions.Text = T(active == 0 ? "photo_instructions" : "photo_trace");
                overlay.Children.Clear();
                Mark(corners, Colors.Turquoise, true);
                Mark(outline, Colors.White, false);
                foreach (var polygon in obstacles) Mark(polygon, Colors.Gold, false);
            }
            void SelectTrace(int index)
            {
                updating = true;
                active = index;
                trace.ItemsSource = new[] { T("photo_corners"), T("photo_outline") }
                    .Concat(obstacles.Select((_, i) => $"{T("photo_obstacle")} {i + 1}")).ToList();
                trace.SelectedIndex = active;
                if (active >= 2) obstacleHeight.Value = heights[active - 2];
                updating = false;
                Refresh();
            }
            SelectTrace(0);
            trace.SelectionChanged += (_, _) =>
            {
                if (updating || trace.SelectedIndex < 0) return;
                active = trace.SelectedIndex;
                updating = true;
                if (active >= 2) obstacleHeight.Value = heights[active - 2];
                updating = false;
                Refresh();
            };
            overlay.PointerPressed += (_, args) =>
            {
                if (busy || (active > 0 && corners.Count != 4)) return;
                var points = Points();
                if (points.Count >= (active == 0 ? 4 : 128)) return;
                var pos = args.GetCurrentPoint(overlay).Position;
                points.Add([Math.Clamp(pos.X / width, 0, 1), Math.Clamp(pos.Y / height, 0, 1)]);
                InvalidatePreview(); Refresh();
            };
            clear.Click += (_, _) => { Points().Clear(); InvalidatePreview(); Refresh(); };
            undo.Click += (_, _) => { var points = Points(); points.RemoveAt(points.Count - 1); InvalidatePreview(); Refresh(); };
            add.Click += (_, _) => { obstacles.Add([]); heights.Add(1); InvalidatePreview(); SelectTrace(obstacles.Count + 1); };
            remove.Click += (_, _) => { obstacles.RemoveAt(active - 2); heights.RemoveAt(active - 2); InvalidatePreview(); SelectTrace(1); };
            obstacleHeight.ValueChanged += (_, _) => { if (!updating && active >= 2 && double.IsFinite(obstacleHeight.Value)) { heights[active - 2] = obstacleHeight.Value; InvalidatePreview(); } };
            roofWidth.ValueChanged += (_, _) => { InvalidatePreview(); Refresh(); };
            roofDepth.ValueChanged += (_, _) => { InvalidatePreview(); Refresh(); };
            async Task<bool> Run(bool previewOnly)
            {
                var request = new { file, source, preview = previewOnly, options = new { array,
                    source_hash = sourceHash, control = new { corners, width_m = roofWidth.Value, depth_m = roofDepth.Value },
                    outline = outline.Count == 0 ? null : outline, obstacles, obstacle_heights = heights } };
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
