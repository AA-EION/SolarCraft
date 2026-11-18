// SPDX-License-Identifier: GPL-3.0-or-later
//! SVG charts for reports (SolarCraft's own design). Rendered by Typst in
//! PDF and rasterised with resvg for DOCX, so both formats show the same chart.
use crate::i18n::Lang;
use std::fmt::Write;

pub const FONT: &str = "Source Sans 3";
/// CSS font-family value: the name must be quoted because "3" is not a valid identifier start.
const FONT_CSS: &str = "'Source Sans 3', sans-serif";
const INK: &str = "#1f2937";
const MUTED: &str = "#6b7280";
const GRID: &str = "#e5e7eb";
const BAR: &str = "#0f5c63";
const LOSS: &str = "#c2410c";
const GAIN: &str = "#15803d";

fn nice_step(max: f64) -> f64 {
    let raw = max / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag)
}

fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// Up to two series with the same numeric x axis and caller-supplied tick labels.
pub fn time_lines(series: &[(String, Vec<(u32, f64)>)], ticks: &[(u32, &str)], unit: &str, lang: Lang) -> String {
    let points = || series.iter().flat_map(|(_, data)| data.iter().copied());
    let start = ticks.first().map_or(0, |tick| tick.0);
    let end = ticks.last().map_or(start + 1, |tick| tick.0).max(start + 1);
    let min = points().map(|(_, y)| y).fold(f64::INFINITY, f64::min);
    let max = points().map(|(_, y)| y).fold(f64::NEG_INFINITY, f64::max);
    let (min, max) = if min.is_finite() && max.is_finite() { (min, max.max(min + 1.0)) } else { (0.0, 1.0) };
    let x = |period: u32| 81.0 + 534.0 * (f64::from(period) - f64::from(start)) / f64::from(end - start);
    let y = |value: f64| 20.0 + 143.0 * (max - value) / (max - min);
    let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="220" viewBox="0 0 640 220" font-family="{FONT_CSS}">"#);
    for k in 0..=4 {
        let value = min + (max - min) * f64::from(k) / 4.0;
        let yy = y(value);
        let _ = write!(svg, r#"<line x1="81" x2="615" y1="{yy:.1}" y2="{yy:.1}" stroke="{GRID}"/><text x="74" y="{:.1}" font-size="10" fill="{MUTED}" text-anchor="end">{}</text>"#, yy + 4.0, esc(&lang.num(value, 1)));
    }
    let _ = write!(svg, r#"<text x="74" y="184" font-size="10" fill="{MUTED}" text-anchor="end">{}</text>"#, esc(unit));
    for &(period, label) in ticks {
        let _ = write!(svg, r#"<text x="{:.1}" y="184" font-size="11" fill="{INK}" text-anchor="middle">{}</text>"#, x(period), esc(label));
    }
    for (i, (label, data)) in series.iter().enumerate() {
        let color = [BAR, "#2563eb"][i % 2];
        if !label.is_empty() {
            let left = 81 + i * 250;
            let _ = write!(svg, r#"<rect x="{left}" y="201" width="9" height="9" fill="{color}"/><text x="{}" y="210" font-size="11" fill="{INK}">{}</text>"#, left + 15, esc(label));
        }
        let _ = write!(svg, r#"<polyline fill="none" stroke="{color}" stroke-width="2" points=""#);
        for &(period, value) in data { let _ = write!(svg, "{:.2},{:.2} ", x(period), y(value)); }
        svg.push_str("\"/>");
    }
    svg.push_str("</svg>");
    svg
}

/// A conservation diagram through one accounting boundary. Ribbon widths are
/// in kWh. The junction does not infer individual source-to-load paths.
pub fn balance_flow(inputs: &[(String, f64)], outputs: &[(String, f64)], lang: Lang) -> String {
    let left: Vec<_> = inputs.iter().filter(|(_, value)| *value > 1e-6).collect();
    let right: Vec<_> = outputs.iter().filter(|(_, value)| *value > 1e-6).collect();
    let total = |side: &[&(String, f64)]| side.iter().map(|(_, v)| v).sum::<f64>();
    let scale = 126.0 / total(&left).max(total(&right)).max(1.0);
    let colors = ["#0f766e", "#0369a1", "#a16207", "#6d28d9"];
    let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="700" height="260" viewBox="0 0 700 260" font-family="{FONT_CSS}">"#);
    for (side, to_right) in [(&left, false), (&right, true)] {
        let mut trunk = 38.0;
        for (i, (label, value)) in side.iter().enumerate() {
            let height = *value * scale;
            let color = colors[i % colors.len()];
            let outer = 25.0 + i as f64 * 26.0 + (trunk - 38.0);
            let (edge, centre, tx, anchor) = if to_right { (560, 354, 690, "end") } else { (140, 346, 10, "start") };
            let ctrl = (edge + centre) / 2;
            let _ = write!(svg, r#"<path d="M{edge},{outer:.2} C{ctrl},{outer:.2} {ctrl},{trunk:.2} {centre},{trunk:.2} L{centre},{:.2} C{ctrl},{:.2} {ctrl},{:.2} {edge},{:.2} Z" fill="{color}" fill-opacity="0.43"/>"#, trunk + height, trunk + height, outer + height, outer + height);
            let label_y = if height >= 14.0 { outer + 12.0 } else { outer + height + 11.0 };
            let _ = write!(svg, r#"<text x="{tx}" y="{label_y:.1}" text-anchor="{anchor}" font-size="11" fill="{INK}">{}</text><text x="{tx}" y="{:.1}" text-anchor="{anchor}" font-size="10" fill="{MUTED}">{} kWh</text>"#, esc(label), label_y + 12.0, esc(&lang.num(*value, 1)));
            trunk += height;
        }
    }
    let _ = write!(svg, r#"<rect x="346" y="38" width="8" height="{:.2}" fill="{INK}"/>"#, total(&left).max(total(&right)) * scale);
    svg.push_str("</svg>");
    svg
}

pub fn connections(rows: &[(String, String)], grid_label: &str) -> String {
    let height = rows.len() * 47 + 24;
    let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="700" height="{height}" viewBox="0 0 700 {height}" font-family="{FONT_CSS}">"#);
    let y1 = 31;
    let y2 = (rows.len() - 1) * 47 + y1;
    let _ = write!(svg, r#"<line x1="525" x2="525" y1="{y1}" y2="{y2}" stroke="{BAR}" stroke-width="2"/>"#);
    for (i, (modules, inverter)) in rows.iter().enumerate() {
        let y = i * 47 + 31;
        let label: String = inverter.chars().take(37).collect();
        let _ = write!(svg, r##"<rect x="12" y="{}" width="175" height="33" rx="6" fill="#ecf7f5" stroke="#b5d7d3"/><rect x="250" y="{}" width="215" height="33" rx="6" fill="#edf4fa" stroke="#adc5d9"/><path d="M187 {y}h63 M465 {y}h60" fill="none" stroke="{BAR}" stroke-width="1.5"/><text x="99" y="{}" font-size="12" text-anchor="middle" fill="{INK}">{}</text><text x="357" y="{}" font-size="12" text-anchor="middle" fill="{INK}">{}</text>"##, y - 16, y - 16, y + 4, esc(modules), y + 4, esc(&label));
    }
    let mid = (y1 + y2) / 2;
    let _ = write!(svg, r##"<path d="M525 {mid}h24" stroke="{BAR}" stroke-width="2"/><rect x="549" y="{}" width="140" height="33" rx="6" fill="#f3f4f6" stroke="#a6b0bc"/><text x="619" y="{}" font-size="12" text-anchor="middle" fill="{INK}">{}</text>"##, mid - 16, mid + 4, esc(grid_label));
    svg.push_str("</svg>");
    svg
}

/// The requested load and its sources, with unserved energy as a separate segment.
pub fn load_supply(series: &[(String, f64)], demand: f64, lang: Lang) -> String {
    let colors = ["#0f766e", "#6d28d9", "#2563eb", "#92400e", "#a3a3a3", "#be185d"];
    let shown = series.iter().zip(colors).filter(|((_, value), _)| *value > 1e-6);
    let height = 48 + 21 * shown.clone().count();
    let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="{height}" viewBox="0 0 640 {height}" font-family="{FONT_CSS}">"#);
    let mut x = 12.0;
    for (row, ((label, value), color)) in shown.enumerate() {
        let width = 616.0 * (value / demand).clamp(0.0, 1.0);
        let y = 49 + 21 * row;
        let _ = write!(svg, r#"<rect x="{x:.2}" y="5" width="{width:.2}" height="23" fill="{color}"/><rect x="12" y="{}" width="10" height="10" rx="2" fill="{color}"/><text x="29" y="{y}" font-size="13" fill="{INK}">{}</text><text x="628" y="{y}" text-anchor="end" font-size="13" fill="{INK}">{} kWh</text>"#, y - 10, esc(label), esc(&lang.num(*value, 1)));
        x += width;
    }
    svg.push_str("</svg>");
    svg
}

/// Cumulative cash flow. Include year zero and draw the zero crossing at the actual value.
pub fn cash_flow(series: &[(u32, f64)], currency: &str, lang: Lang) -> String {
    let last = series.last().map_or(1, |point| point.0.max(1));
    let lo = series.iter().map(|point| point.1).fold(0.0, f64::min);
    let hi = series.iter().map(|point| point.1).fold(0.0, f64::max);
    let x = |year| 115.0 + 500.0 * f64::from(year) / f64::from(last);
    let y = |amount| 20.0 + 155.0 * (hi - amount) / (hi - lo).max(1.0);
    let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="210" viewBox="0 0 640 210" font-family="{FONT_CSS}">"#);
    for level in [lo, hi].into_iter().filter(|&value| value != 0.0).chain([0.0]) {
        let yy = y(level);
        let _ = write!(svg, r#"<line x1="115" x2="615" y1="{yy:.1}" y2="{yy:.1}" stroke="{GRID}"/><text x="108" y="{:.1}" text-anchor="end" font-size="11" fill="{MUTED}">{}</text>"#, yy + 4.0, esc(&lang.num(level, 0)));
    }
    let end_label = esc(currency);
    let _ = write!(svg, r#"<line x1="115" x2="615" y1="{0:.1}" y2="{0:.1}" stroke="{MUTED}"/><text x="115" y="196" font-size="11" fill="{INK}">0</text><text x="615" y="196" text-anchor="end" font-size="11" fill="{INK}">{last}</text><text x="109" y="196" text-anchor="end" font-size="10" fill="{MUTED}">{end_label}</text><polyline fill="none" stroke="{BAR}" stroke-width="2" points=""#, y(0.0));
    for (year, value) in series { let _ = write!(svg, "{:.1},{:.1} ", x(*year), y(*value)); }
    svg.push_str("\"/></svg>");
    svg
}

/// Monthly bar chart. `values` in kWh.
pub fn monthly_bars(values: &[f64; 12], labels: [&str; 12], unit: &str, lang: Lang) -> String {
    let (w, h, l, r, t, b) = (640.0, 260.0, 56.0, 12.0, 16.0, 30.0);
    let max = values.iter().cloned().fold(0.0, f64::max).max(1.0);
    let step = nice_step(max);
    let top = (max / step).ceil() * step;
    let y = |v: f64| t + (h - t - b) * (1.0 - v / top);
    let mut s = String::new();
    let _ = write!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{FONT_CSS}">"#);
    let mut v = 0.0;
    while v <= top + 1e-9 {
        let yy = y(v);
        let _ = write!(s, r#"<line x1="{l}" y1="{yy:.1}" x2="{}" y2="{yy:.1}" stroke="{GRID}" stroke-width="1"/>"#, w - r);
        let _ = write!(s, r#"<text x="{}" y="{:.1}" font-size="11" fill="{MUTED}" text-anchor="end">{}</text>"#, l - 6.0, yy + 4.0, esc(&lang.num(v, 0)));
        v += step;
    }
    let _ = write!(s, r#"<text x="12" y="{}" font-size="11" fill="{MUTED}" transform="rotate(-90 12 {})" text-anchor="middle">{}</text>"#, (t + h - b) / 2.0, (t + h - b) / 2.0, esc(unit));
    let slot = (w - l - r) / 12.0;
    for (i, val) in values.iter().enumerate() {
        let x = l + slot * i as f64 + slot * 0.18;
        let bw = slot * 0.64;
        let yy = y(*val);
        let _ = write!(s, r#"<rect x="{x:.1}" y="{yy:.1}" width="{bw:.1}" height="{:.1}" rx="2" fill="{BAR}"/>"#, (h - b - yy).max(0.0));
        let _ = write!(s, r#"<text x="{:.1}" y="{}" font-size="11" fill="{INK}" text-anchor="middle">{}</text>"#, x + bw / 2.0, h - b + 16.0, esc(labels[i]));
    }
    s.push_str("</svg>");
    s
}

/// Horizontal waterfall: `steps` = (label, value_after_step, is_total).
pub fn waterfall(steps: &[(String, f64, bool)], unit: &str, lang: Lang) -> String {
    let row = 22.0;
    let (w, l, r, t) = (640.0, 230.0, 70.0, 8.0);
    let h = t + row * steps.len() as f64 + 24.0;
    let max = steps.iter().map(|s| s.1).fold(0.0, f64::max).max(1.0);
    let x = |v: f64| l + (w - l - r) * (v / max);
    let mut s = String::new();
    let _ = write!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{FONT_CSS}">"#);
    let mut prev = 0.0;
    for (i, (label, v, total)) in steps.iter().enumerate() {
        let yy = t + row * i as f64;
        let _ = write!(s, r#"<text x="{}" y="{:.1}" font-size="11" fill="{INK}" text-anchor="end"{}>{}</text>"#, l - 8.0, yy + 14.0, if *total { r#" font-weight="600""# } else { "" }, esc(label));
        let (a, b, color) = if *total { (0.0, *v, BAR) } else if *v <= prev { (*v, prev, LOSS) } else { (prev, *v, GAIN) };
        let _ = write!(s, r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{}" rx="1.5" fill="{color}"/>"#, x(a), yy + 3.0, (x(b) - x(a)).max(0.8), row - 7.0);
        let txt = if *total { lang.num(*v, 0) } else { format!("{}{}", if v >= &prev { "+" } else { "" }, lang.num(v - prev, 0)) };
        let _ = write!(s, r#"<text x="{:.1}" y="{:.1}" font-size="11" fill="{MUTED}">{}</text>"#, x(b.max(a)) + 6.0, yy + 14.0, esc(&txt));
        prev = *v;
    }
    let _ = write!(s, r#"<text x="{}" y="{:.1}" font-size="11" fill="{MUTED}" text-anchor="end">{}</text>"#, w - 4.0, h - 6.0, esc(unit));
    s.push_str("</svg>");
    s
}

/// Roof plan: outline, obstacles and placed modules in roof-plane metres
/// (x along the eave, y up the slope; the eave is drawn at the bottom).
pub fn roof_plan(face: &solarcraft_geometry::layout::RoofFace, modules: &[solarcraft_geometry::poly::Rect], caption: &str, lang: Lang) -> String {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for v in &face.outline { x0 = x0.min(v[0]); y0 = y0.min(v[1]); x1 = x1.max(v[0]); y1 = y1.max(v[1]); }
    if !(x1 > x0 && y1 > y0) { return String::new(); }
    let (w, pad) = (640.0, 36.0);
    let scale = ((w - 2.0 * pad) / (x1 - x0)).min(320.0 / (y1 - y0));
    let h = (y1 - y0) * scale + 2.0 * pad + 18.0;
    let px = |x: f64| pad + (x - x0) * scale;
    let py = |y: f64| pad + (y1 - y) * scale;
    let path = |pts: &[[f64; 2]]| pts.iter().enumerate().map(|(i, p)| format!("{}{:.1},{:.1}", if i == 0 { "M" } else { "L" }, px(p[0]), py(p[1]))).collect::<String>() + "Z";
    let mut s = String::new();
    let _ = write!(s, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h:.0}" viewBox="0 0 {w} {h:.0}" font-family="{FONT_CSS}">"#);
    let _ = write!(s, r##"<path d="{}" fill="#f3f4f6" stroke="{INK}" stroke-width="1.5"/>"##, path(&face.outline));
    for m in modules {
        let _ = write!(s, r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{BAR}" stroke="#ffffff" stroke-width="0.6"/>"##, px(m.x0), py(m.y1), (m.x1 - m.x0) * scale, (m.y1 - m.y0) * scale);
    }
    for ob in &face.obstacles { let _ = write!(s, r##"<path d="{}" fill="#9ca3af" stroke="{INK}" stroke-width="1"/>"##, path(ob)); }
    // Eave dimension line.
    let yb = py(y0) + 14.0;
    let _ = write!(s, r#"<line x1="{:.1}" y1="{yb:.1}" x2="{:.1}" y2="{yb:.1}" stroke="{MUTED}" stroke-width="1"/>"#, px(x0), px(x1));
    let _ = write!(s, r#"<text x="{:.1}" y="{:.1}" font-size="11" fill="{MUTED}" text-anchor="middle">{} m</text>"#, (px(x0) + px(x1)) / 2.0, yb + 13.0, esc(&lang.num(x1 - x0, 2)));
    let _ = write!(s, r#"<text x="{pad}" y="16" font-size="11" fill="{INK}">{}</text>"#, esc(caption));
    s.push_str("</svg>");
    s
}
