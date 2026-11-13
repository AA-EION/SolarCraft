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
