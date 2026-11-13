// SPDX-License-Identifier: GPL-3.0-or-later
//! Customer reports. A locale-formatted view model ([`View`]) is built once
//! from the project and results; the PDF renderer (Typst, original template)
//! and the editable DOCX writer both consume it, so the formats cannot drift.
pub mod charts;
pub mod docx;
pub mod i18n;
pub mod pdf;

use i18n::Lang;
use serde::Serialize;
use solarcraft_engine::SimResult;
use solarcraft_model::{Mounting, Project};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct WeatherInfo { pub provider: String, pub dataset: String, pub licence: String, pub horizon_applied: bool }

#[derive(Debug, Clone)]
pub struct ReportInput<'a> {
    pub project: &'a Project,
    pub result: &'a SimResult,
    pub weather: WeatherInfo,
    pub lang: Lang,
    /// (year, month, day) printed on the report.
    pub date: (i32, u8, u8),
    pub designer: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Kpi { pub label: String, pub value: String, pub unit: String }

#[derive(Debug, Clone, Serialize)]
pub struct ArrayView { pub name: String, pub rows: Vec<[String; 2]> }

#[derive(Debug, Clone, Serialize)]
pub struct CustomerView { pub name: String, pub address: String }

#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub lang: String,
    pub s: HashMap<&'static str, &'static str>,
    pub title: String,
    pub project_name: String,
    pub designer: String,
    pub customer: CustomerView,
    pub date: String,
    pub kpis: Vec<Kpi>,
    pub arrays: Vec<ArrayView>,
    pub site_rows: Vec<[String; 2]>,
    pub weather_notes: Vec<String>,
    pub licence: String,
    pub loss_rows: Vec<[String; 3]>,
    pub monthly_header: [String; 4],
    pub monthly_rows: Vec<[String; 4]>,
    pub assumption_rows: Vec<[String; 2]>,
    pub models_text: String,
    pub notes: Vec<String>,
    pub disclaimer: String,
    pub run_rows: Vec<[String; 2]>,
    pub footer: String,
    #[serde(skip)]
    pub monthly_svg: String,
    #[serde(skip)]
    pub waterfall_svg: String,
    /// Roof plan SVGs (one per array with a roof); the template reads roof-<i>.svg.
    #[serde(skip)]
    pub roof_svgs: Vec<String>,
    pub roof_count: usize,
}

pub fn build_view(inp: &ReportInput) -> View {
    let l = inp.lang;
    let s = i18n::table(l);
    let p = inp.project;
    let r = inp.result;
    let n = |v: f64, d: usize| l.num(v, d);
    let date = match l { Lang::En => format!("{:04}-{:02}-{:02}", inp.date.0, inp.date.1, inp.date.2), _ => format!("{:02}/{:02}/{:04}", inp.date.2, inp.date.1, inp.date.0) };
    let kpis = vec![
        Kpi { label: s["kpi_size"].into(), value: n(r.annual.dc_kwp, 2), unit: "kWp".into() },
        Kpi { label: s["kpi_energy"].into(), value: n(r.annual.ac_kwh, 0), unit: "kWh".into() },
        Kpi { label: s["kpi_yield"].into(), value: n(r.annual.specific_yield_kwh_kwp, 0), unit: "kWh/kWp".into() },
        Kpi { label: s["kpi_pr"].into(), value: n(r.annual.performance_ratio * 100.0, 1), unit: "%".into() },
    ];
    let mounting = match p.simulation.mounting.value { Mounting::OpenRack => s["open_rack"], Mounting::RoofMounted => s["roof_mounted"], Mounting::Integrated => s["integrated"] };
    let arrays = p.arrays.iter().map(|a| {
        let m = p.library.module(&a.module_id.value);
        let inv = p.library.inverter(&a.inverter_id.value);
        let n_mod = a.modules_per_string.value * a.strings.value;
        let kwp = m.map(|m| n_mod as f64 * m.pmp_stc() / 1000.0).unwrap_or(0.0);
        let kwac = inv.map(|i| i.pac_max_w * a.inverter_count.value as f64 / 1000.0).unwrap_or(0.0);
        ArrayView { name: format!("{} · {} kWp", a.name.value, n(kwp, 2)), rows: vec![
            [s["module"].into(), m.map(|m| format!("{} {} ({} W)", m.manufacturer, m.model, n(m.pmp_stc(), 0))).unwrap_or_default()],
            [s["modules"].into(), n_mod.to_string()],
            [s["strings"].into(), format!("{} × {}", a.strings.value, a.modules_per_string.value)],
            [s["tilt"].into(), format!("{}°", n(a.tilt.value, 1))],
            [s["azimuth"].into(), format!("{}°", n(a.azimuth.value, 1))],
            [s["mounting"].into(), mounting.into()],
            [s["inverter"].into(), inv.map(|i| format!("{} × {} {}", a.inverter_count.value, i.manufacturer, i.model)).unwrap_or_default()],
            [s["dc_ac"].into(), if kwac > 0.0 { n(kwp / kwac, 2) } else { "–".into() }],
        ] }
    }).collect();
    let lat = p.site.latitude.value;
    let lon = p.site.longitude.value;
    let site_rows = vec![
        [s["location"].into(), format!("{}° {}, {}° {}", n(lat.abs(), 4), if lat >= 0.0 { "N" } else { "S" }, n(lon.abs(), 4), if lon >= 0.0 { "E" } else { "W" })],
        [s["elevation"].into(), format!("{} m", n(p.site.elevation.value, 0))],
        [s["weather_source"].into(), format!("{} — {}", inp.weather.provider, inp.weather.dataset)],
        [s["ghi"].into(), format!("{} kWh/m²", n(r.annual.ghi_kwh_m2, 0))],
        [s["poa"].into(), format!("{} kWh/m²", n(r.annual.poa_kwh_m2, 0))],
    ];
    let loss_label = |key: &str, fallback: &str| -> String {
        let k = match key { "soiling" => "soiling", "lid" => "lid", "mismatch" => "mismatch", "dc_wiring" => "dc_wiring", "ac_wiring" => "ac_wiring", "availability" => "availability", _ => "" };
        if k.is_empty() { fallback.to_string() } else { s[k].to_string() }
    };
    // Engine labels are English; localised labels for the main steps, engine wording for the rest.
    let step_label = |key: &str, en: &str| -> String {
        match (l, key) {
            (Lang::En, _) => en.to_string(),
            (_, "nominal") => match l { Lang::Es => "Energía nominal (eficiencia STC)", Lang::De => "Nennenergie (STC-Wirkungsgrad)", _ => "Energia nominal (eficiência STC)" }.into(),
            (_, "irradiance_level") => match l { Lang::Es => "Nivel de irradiancia", Lang::De => "Einstrahlungsniveau", _ => "Nível de irradiância" }.into(),
            (_, "temperature") => match l { Lang::Es => "Temperatura de módulo", Lang::De => "Modultemperatur", _ => "Temperatura do módulo" }.into(),
            (_, "mpp_window") => match l { Lang::Es => "Ventana de tensión MPP del inversor", Lang::De => "MPP-Spannungsfenster des Wechselrichters", _ => "Janela de tensão MPP do inversor" }.into(),
            (_, "quality") => match l { Lang::Es => "Calidad de módulo", Lang::De => "Modulqualität", _ => "Qualidade do módulo" }.into(),
            (_, "dc_out") => match l { Lang::Es => "Salida CC del generador", Lang::De => "DC-Ausgang Generator", _ => "Saída CC do gerador" }.into(),
            (_, "inverter") => match l { Lang::Es => "Eficiencia del inversor", Lang::De => "Wechselrichterwirkungsgrad", _ => "Eficiência do inversor" }.into(),
            (_, "clipping") => match l { Lang::Es => "Recorte por límite de potencia", Lang::De => "Begrenzung (Leistungslimit)", _ => "Corte por limite de potência" }.into(),
            (_, "night") => match l { Lang::Es => "Consumo nocturno del inversor", Lang::De => "Nachtverbrauch Wechselrichter", _ => "Consumo noturno do inversor" }.into(),
            (_, "grid") => match l { Lang::Es => "Energía a la red", Lang::De => "Netzeinspeisung", _ => "Energia para a rede" }.into(),
            _ => loss_label(key, en),
        }
    };
    let loss_rows = r.energy_ledger.iter().map(|st| {
        let total = matches!(st.key.as_str(), "nominal" | "dc_out" | "grid");
        [step_label(&st.key, &st.label), n(st.value, 0), if total { String::new() } else { format!("{} %", n(st.delta_pct, 2)) }]
    }).collect();
    let waterfall_steps: Vec<(String, f64, bool)> = r.energy_ledger.iter()
        .filter(|st| matches!(st.key.as_str(), "nominal" | "dc_out" | "grid") || st.delta.abs() > 1e-6)
        .map(|st| (step_label(&st.key, &st.label), st.value, matches!(st.key.as_str(), "nominal" | "dc_out" | "grid"))).collect();
    let months = i18n::month_abbr(l);
    let mut vals = [0.0; 12];
    for m in &r.monthly { if (1..=12).contains(&m.month) { vals[m.month as usize - 1] = m.ac_kwh; } }
    let monthly_rows = {
        let mut v: Vec<[String; 4]> = r.monthly.iter().map(|m| [months[(m.month as usize).clamp(1, 12) - 1].to_string(), n(m.ghi_kwh_m2, 1), n(m.poa_kwh_m2, 1), n(m.ac_kwh, 0)]).collect();
        v.push([s["total"].into(), n(r.annual.ghi_kwh_m2, 1), n(r.annual.poa_kwh_m2, 1), n(r.annual.ac_kwh, 0)]);
        v
    };
    let lo = &p.losses;
    let pct = |v: f64| format!("{} %", n(v, 1));
    let model_name = format!("{:?}", p.simulation.transposition.value).replace("HayDavies", "Hay–Davies").replace("Perez", "Perez et al. (1990)");
    let roof_svgs: Vec<String> = p.arrays.iter().filter_map(|a| a.roof.as_ref().map(|r| {
        let cap = format!("{} — {}", a.name.value, s["roof_caption"].replace("%N%", &r.modules.len().to_string()).replace("%S%", &n(r.params.setback, 2)));
        charts::roof_plan(&r.face, &r.modules, &cap, l)
    })).filter(|svg| !svg.is_empty()).collect();
    View {
        lang: l.tag().into(), title: s["title"].into(), project_name: p.name.value.clone(), designer: inp.designer.clone(),
        customer: CustomerView { name: if p.customer.name.is_empty() { "—".into() } else { p.customer.name.clone() }, address: p.customer.address.clone() },
        date, kpis, arrays, site_rows,
        weather_notes: { let mut v = vec![format!("{} — {}", s["weather_tmy"], inp.weather.dataset)]; if inp.weather.horizon_applied { v.push(s["note_horizon"].into()); } v },
        licence: inp.weather.licence.clone(), loss_rows,
        monthly_header: [s["month"].into(), "GHI kWh/m²".into(), "POA kWh/m²".into(), format!("{} kWh", s["energy"])],
        monthly_rows,
        assumption_rows: vec![[s["soiling"].into(), pct(lo.soiling_pct.value)], [s["lid"].into(), pct(lo.lid_pct.value)], [s["mismatch"].into(), pct(lo.mismatch_pct.value)], [s["dc_wiring"].into(), pct(lo.dc_wiring_pct.value)], [s["ac_wiring"].into(), pct(lo.ac_wiring_pct.value)], [s["availability"].into(), pct(lo.availability_pct.value)], ["Albedo".into(), n(p.site.albedo.value, 2)]],
        models_text: s["models_text"].replace("%MODEL%", &model_name),
        notes: r.warnings.iter().filter(|w| w.code != "HORIZON_IN_WEATHER").map(|w| match w.code.as_str() {
            "MPP_WINDOW_HOURS" => s["note_mpp"].replace("%S%", &w.subject).replace("%N%", &n(w.value, 0)),
            _ => w.text.clone(),
        }).collect(),
        disclaimer: s["disclaimer"].into(),
        run_rows: vec![["engine".into(), r.manifest.engine.clone()], ["weather".into(), r.manifest.weather_hash.chars().take(12).collect()], ["hours".into(), r.manifest.hours.to_string()]],
        footer: format!("{} {}", s["generated_with"], env!("CARGO_PKG_VERSION")),
        monthly_svg: charts::monthly_bars(&vals, months, "kWh", l),
        waterfall_svg: charts::waterfall(&waterfall_steps, "kWh", l),
        roof_svgs: roof_svgs.clone(),
        roof_count: roof_svgs.len(),
        s,
    }
}

pub(crate) static FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-It.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-Semibold.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-Bold.ttf"),
];
