// SPDX-License-Identifier: GPL-3.0-or-later
//! Editable DOCX export (docx-rs) from the same view model as the PDF.
//! Charts are rasterised from the same SVGs with resvg.
use crate::View;
use docx_rs::*;

fn svg_to_png(svg: &str) -> Result<(Vec<u8>, u32, u32), String> {
    let mut opt = resvg::usvg::Options::default();
    let db = opt.fontdb_mut();
    for f in crate::FONTS { db.load_font_data(f.to_vec()); }
    db.set_sans_serif_family(crate::charts::FONT);
    let tree = resvg::usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?;
    let scale = 2.0;
    let size = tree.size().to_int_size().scale_by(scale).ok_or("bad size")?;
    let mut pix = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("pixmap")?;
    pix.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pix.as_mut());
    Ok((pix.encode_png().map_err(|e| e.to_string())?, size.width(), size.height()))
}

const TEAL: &str = "0F5C63";
const MUTED: &str = "6B7280";

/// Editable documents use a font every office suite has (LibreOffice maps Calibri to metric-compatible Carlito).
const DOC_FONT: &str = "Calibri";
fn text(t: &str) -> Run { Run::new().add_text(t).fonts(RunFonts::new().ascii(DOC_FONT).hi_ansi(DOC_FONT).cs(DOC_FONT)) }
fn para(t: &str) -> Paragraph { Paragraph::new().add_run(text(t).size(20)) }
fn heading(t: &str) -> Paragraph { Paragraph::new().add_run(text(t).size(28).bold().color(TEAL)).line_spacing(LineSpacing::new().before(280).after(100)) }

fn table_of<const N: usize>(header: Option<&[String; N]>, rows: &[[String; N]]) -> Table {
    let widths: Vec<usize> = match N { 2 => vec![5600, 4000], 3 => vec![5600, 2000, 2000], 4 => vec![3600, 2000, 2000, 2000], _ => vec![9600 / N; N] };
    let mk_row = |cells: &[String; N], bold: bool| TableRow::new(cells.iter().enumerate().map(|(i, c)| {
        let mut r = text(c).size(19);
        if bold { r = r.bold().color(MUTED); }
        let mut p = Paragraph::new().add_run(r);
        if i > 0 { p = p.align(AlignmentType::Right); }
        TableCell::new().add_paragraph(p).width(widths[i], WidthType::Dxa)
    }).collect());
    let mut rows_v = vec![];
    if let Some(h) = header { rows_v.push(mk_row(h, true)); }
    rows_v.extend(rows.iter().map(|r| mk_row(r, false)));
    // Full text width (≈ 16.9 cm = 9600 twips); first column wider for labels.
    let grid: Vec<usize> = match N { 2 => vec![5600, 4000], 3 => vec![5600, 2000, 2000], 4 => vec![3600, 2000, 2000, 2000], _ => vec![9600 / N; N] };
    Table::new(rows_v).set_grid(grid.clone()).width(9600, WidthType::Dxa).layout(TableLayoutType::Fixed)
}

fn image_para(svg: &str, width_emu: u32) -> Result<Paragraph, String> {
    let (png, w, h) = svg_to_png(svg)?;
    let height_emu = (width_emu as f64 * h as f64 / w as f64) as u32;
    Ok(Paragraph::new().add_run(Run::new().add_image(Pic::new(&png).size(width_emu, height_emu))))
}

pub fn render(v: &View) -> Result<Vec<u8>, String> {
    let s = &v.s;
    let width = 6_000_000; // ≈ 16.7 cm
    let mut doc = Docx::new()
        .add_paragraph(Paragraph::new().add_run(text("SOLARCRAFT").size(18).bold().color(TEAL)))
        .add_paragraph(Paragraph::new().add_run(text(&v.title).size(44).bold()))
        .add_paragraph(Paragraph::new().add_run(text(&v.project_name).size(28).color(MUTED)))
        .add_paragraph(para(&format!("{}: {} — {}", s["prepared_for"], v.customer.name, v.customer.address)))
        .add_paragraph(para(&format!("{}: {} · {}", s["date"], v.date, v.designer)));
    let kpis: Vec<[String; 2]> = v.kpis.iter().map(|k| [k.label.clone(), format!("{} {}", k.value, k.unit)]).collect();
    doc = doc.add_table(table_of(None, &kpis))
        .add_paragraph(Paragraph::new().add_run(text(s["monthly_chart"]).bold().size(20)))
        .add_paragraph(image_para(&v.monthly_svg, width)?)
        .add_paragraph(heading(s["system"]));
    for a in &v.arrays { doc = doc.add_paragraph(Paragraph::new().add_run(text(&a.name).bold().size(20))).add_table(table_of(None, &a.rows)); }
    if !v.roof_svgs.is_empty() {
        doc = doc.add_paragraph(Paragraph::new().add_run(text(s["roof_plan"]).bold().size(20)));
        for svg in &v.roof_svgs { doc = doc.add_paragraph(image_para(svg, width)?); }
    }
    doc = doc.add_paragraph(heading(s["site_weather"])).add_table(table_of(None, &v.site_rows))
        .add_paragraph(Paragraph::new().add_run(text(&v.weather_notes.join(" · ")).size(17).color(MUTED)))
        .add_paragraph(Paragraph::new().add_run(text(&v.licence).size(17).color(MUTED)))
        .add_paragraph(heading(s["losses"]))
        .add_paragraph(Paragraph::new().add_run(text(s["losses_intro"]).size(18).color(MUTED)))
        .add_paragraph(image_para(&v.waterfall_svg, width)?)
        .add_table(table_of(Some(&[s["irradiation"].to_string(), "kWh/m²".to_string(), s["change"].to_string()]), &v.irr_rows))
        .add_paragraph(para(""))
        .add_table(table_of(Some(&[s["step"].to_string(), format!("{} (kWh)", s["energy"]), s["change"].to_string()]), &v.loss_rows))
        .add_paragraph(heading(s["monthly_table"]))
        .add_table(table_of(Some(&v.monthly_header), &v.monthly_rows))
        .add_paragraph(heading(s["assumptions"]))
        .add_table(table_of(None, &v.assumption_rows))
        .add_paragraph(Paragraph::new().add_run(text(&format!("{}: ", s["models"])).bold().size(19)).add_run(text(&v.models_text).size(19)));
    if !v.notes.is_empty() {
        doc = doc.add_paragraph(heading(s["notes"]));
        for n in &v.notes { doc = doc.add_paragraph(para(&format!("• {n}"))); }
    }
    doc = doc.add_paragraph(Paragraph::new().add_run(text(&v.disclaimer).size(18).italic()))
        .add_paragraph(Paragraph::new().add_run(text(&format!("{}: {}", s["run_info"], v.run_rows.iter().map(|r| format!("{} {}", r[0], r[1])).collect::<Vec<_>>().join(" · "))).size(16).color(MUTED)))
        .add_paragraph(Paragraph::new().add_run(text(&v.footer).size(16).color(MUTED)));
    let mut buf = std::io::Cursor::new(Vec::new());
    doc.build().pack(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}
