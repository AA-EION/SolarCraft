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
