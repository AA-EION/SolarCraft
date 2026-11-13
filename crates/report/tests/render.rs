// SPDX-License-Identifier: GPL-3.0-or-later
use solarcraft_model::Project;
use solarcraft_pv::{irradiance as irr, spa};
use solarcraft_report::{build_view, docx, i18n::Lang, pdf, ReportInput, WeatherInfo};
use solarcraft_weather::{Hour, Provenance, Weather};

fn fixture() -> (Project, solarcraft_engine::SimResult) {
    let (lat, lon) = (4.711, -74.072);
    let obs = spa::Observer::new(lat, lon, 2600.0);
    let start = solarcraft_pv::days_from_civil(2025, 1, 1) * 86400;
    let hours = (0..8760).map(|h| {
        let t = start + h * 3600 + 1800;
        let sp = spa::solar_position(t as f64, &obs, 69.0);
        let ghi = irr::clearsky_haurwitz(sp.apparent_zenith) * 0.7;
        let (dni, dhi) = irr::erbs(ghi, sp.apparent_zenith, irr::dni_extra(solarcraft_pv::day_of_year_utc(t as f64) as f64, 1366.1));
        Hour { unix_mid: t, ghi, dni, dhi, temp_air: 14.0, wind: 2.0, pressure: 75000.0 }
    }).collect();
    let w = Weather { latitude: lat, longitude: lon, elevation: 2600.0, horizon_applied: true, provenance: Provenance { provider: "test".into(), dataset: "synthetic".into(), url: String::new(), retrieved_unix: 0, licence: "Weather data: test".into(), notes: vec![] }, hours };
    let mut p = Project::new("t", "Casa de prueba", lat, lon);
    p.set_by_pointer("/arrays/0/roof", serde_json::json!({"face": {"outline": [[0.0, 0.0], [11.0, 0.0], [8.0, 5.0], [3.0, 5.0]], "obstacles": [[[5.0, 2.0], [5.8, 2.0], [5.8, 2.8], [5.0, 2.8]]]},
        "params": {"module_length": 0.0, "module_width": 0.0, "orientation": "best", "gap_x": 0.02, "gap_y": 0.02, "setback": 0.4, "obstacle_clearance": 0.3, "search_step": 0.05}, "modules": [], "orientation": "best", "inputs_key": ""})).unwrap();
    p.customer.name = "Cliente Ñandú & Hijos <S.A.>".into(); // markup-like characters must print literally
    p.weather = Some(solarcraft_model::WeatherRef { hash: "abc".into(), provider: "test".into(), summary: String::new() });
    let r = solarcraft_engine::simulate(&p, &w, "abc", |_| true).unwrap();
    (p, r)
}

#[test]
fn all_languages_render_pdf_and_docx() {
    let (p, r) = fixture();
    for (lang, title) in [(Lang::En, "Photovoltaic system proposal"), (Lang::Es, "Propuesta de sistema fotovoltaico"), (Lang::De, "Angebot Photovoltaikanlage"), (Lang::PtBr, "Proposta de sistema fotovoltaico")] {
        let input = ReportInput { project: &p, result: &r, weather: WeatherInfo { provider: "PVGIS".into(), dataset: "TMY".into(), licence: "Weather data: test".into(), horizon_applied: true }, lang, date: (2026, 11, 12), designer: "Test".into() };
        let v = build_view(&input);
        assert_eq!(v.title, title);
        let json = serde_json::to_string(&v).unwrap();
        for bad in ["PRV-", "to be confirmed", "outside the inverter MPP voltage window"] {
            assert!(lang == Lang::En && bad.starts_with("outside") || !json.contains(bad), "{lang:?} leaks '{bad}'");
        }
        assert!(v.monthly_svg.contains("font-family=\"'Source Sans 3'"));
        assert_eq!(v.roof_count, 1);
        assert!(v.roof_svgs[0].matches("<rect").count() == p.arrays[0].roof.as_ref().unwrap().modules.len());
        let pdf_bytes = pdf::render(&v, input.date).unwrap_or_else(|e| panic!("{lang:?}: {e}"));
        assert!(pdf_bytes.starts_with(b"%PDF-") && pdf_bytes.len() > 20_000);
        let d = docx::render(&v).unwrap();
        assert!(d.starts_with(b"PK"), "docx must be a zip");
        let text = String::from_utf8_lossy(&d);
        assert!(text.contains("word/document.xml") && text.contains("word/media/"));
    }
}

#[test]
fn customer_text_is_escaped_in_view() {
    let (p, r) = fixture();
    let input = ReportInput { project: &p, result: &r, weather: WeatherInfo { provider: "x".into(), dataset: "y".into(), licence: "z".into(), horizon_applied: false }, lang: Lang::Es, date: (2026, 1, 2), designer: String::new() };
    let v = build_view(&input);
    assert_eq!(v.customer.name, "Cliente Ñandú & Hijos <S.A.>");
    assert_eq!(v.date, "02/01/2026");
    assert!(pdf::render(&v, input.date).is_ok());
}
