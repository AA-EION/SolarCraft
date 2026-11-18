// SPDX-License-Identifier: GPL-3.0-or-later
//! Customer reports. A locale-formatted view model ([`View`]) is built once
//! from the project and results; the PDF renderer (Typst, original template)
//! and the editable DOCX writer both consume it, so the formats cannot drift.
pub mod charts;
pub mod docx;
pub mod i18n;
pub mod pdf;
mod profiles;
mod detail;
pub use profiles::{ContentBlock, ContentSection, ReportType};

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

/// A generic table section (plant design, energy balance, battery, economics).
#[derive(Debug, Clone, Serialize)]
pub struct Section { pub id: &'static str, pub title: String, pub intro: String, pub header: Vec<String>, pub rows: Vec<Vec<String>> }

/// Pick the phrase for the report language (en, es, de, pt-BR).
fn tr(l: Lang, en: &str, es: &str, de: &str, pt: &str) -> String { match l { Lang::En => en, Lang::Es => es, Lang::De => de, _ => pt }.to_string() }

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
    pub irr_rows: Vec<[String; 3]>,
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
    /// Plant sections built from the run (empty when not applicable).
    pub sections: Vec<Section>,
    pub report_type: ReportType,
    /// Ordered sections shared by the preview, PDF and DOCX.
    pub contents: Vec<ContentSection>,
    #[serde(skip)]
    pub(crate) figures: Vec<String>,
}

/// Rasterize a preview at its designed on-screen dimensions.
pub fn figure_png(view: &View, index: usize) -> Result<Vec<u8>, String> {
    let svg = view.figures.get(index).ok_or("missing report figure")?;
    docx::svg_to_png(svg, 1.0).map(|(data, _, _)| data)
}

pub fn build_view(inp: &ReportInput) -> View {
    build_view_for(inp, ReportType::default())
}

pub fn build_view_for(inp: &ReportInput, report_type: ReportType) -> View {
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
    let elec = solarcraft_model::derive::electrical(p);
    let assigned = solarcraft_model::derive::modules_by_array(&elec);
    let arrays = p.arrays.iter().map(|a| {
        let m = p.library.module(&a.module_id.value);
        let inv = p.library.inverter(&a.inverter_id.value);
        let n_mod = assigned.get(&a.id).copied().unwrap_or(0);
        let kwp = m.map(|m| n_mod as f64 * m.pmp_stc() / 1000.0).unwrap_or(0.0);
        let kwac = inv.map(|i| i.pac_max_w * a.inverter_count.value as f64 / 1000.0).unwrap_or(0.0);
        ArrayView { name: format!("{} · {} kWp", a.name.value, n(kwp, 2)), rows: vec![
            [s["module"].into(), m.map(|m| format!("{} {} ({} W)", m.manufacturer, m.model, n(m.pmp_stc(), 0))).unwrap_or_default()],
            [s["modules"].into(), n_mod.to_string()],
            [s["strings"].into(), elec.inverters.iter().flat_map(|u| u.mppts.iter().flatten()).filter(|g| g.array_id == a.id).map(|g| format!("{} × {}", g.strings, g.modules_per_string)).collect::<Vec<_>>().join(" + ")],
            [s["tilt"].into(), format!("{}°", n(a.tilt.value, 1))],
            [s["azimuth"].into(), format!("{}°", n(a.azimuth.value, 1))],
            [s["mounting"].into(), mounting.into()],
            [s["inverter"].into(), if elec.manual { elec.inverters.iter().filter(|u| u.mppts.iter().flatten().any(|g| g.array_id == a.id)).map(|u| u.name.clone()).collect::<Vec<_>>().join(", ") } else { inv.map(|i| format!("{} × {} {}", a.inverter_count.value, i.manufacturer, i.model)).unwrap_or_default() }],
            [s["dc_ac"].into(), if kwac > 0.0 && !elec.manual { n(kwp / kwac, 2) } else { "–".into() }],
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
            (_, "mpp_window") => match l { Lang::Es => "Límites de entrada del inversor (ventana MPP, corriente)", Lang::De => "Eingangsgrenzen des Wechselrichters (MPP-Fenster, Strom)", _ => "Limites de entrada do inversor (janela MPP, corrente)" }.into(),
            (_, "strings") => match l { Lang::Es => "Desajuste de cadenas en paralelo", Lang::De => "Fehlanpassung paralleler Strings", _ => "Descasamento de strings em paralelo" }.into(),
            (_, "quality") => match l { Lang::Es => "Calidad de módulo", Lang::De => "Modulqualität", _ => "Qualidade do módulo" }.into(),
            (_, "dc_out") => match l { Lang::Es => "Salida CC del generador", Lang::De => "DC-Ausgang Generator", _ => "Saída CC do gerador" }.into(),
            (_, "inverter") => match l { Lang::Es => "Eficiencia del inversor", Lang::De => "Wechselrichterwirkungsgrad", _ => "Eficiência do inversor" }.into(),
            (_, "clipping") => match l { Lang::Es => "Recorte por límite de potencia", Lang::De => "Begrenzung (Leistungslimit)", _ => "Corte por limite de potência" }.into(),
            (_, "night") => match l { Lang::Es => "Consumo nocturno del inversor", Lang::De => "Nachtverbrauch Wechselrichter", _ => "Consumo noturno do inversor" }.into(),
            (_, "grid") if p.workflow.system.islanded() =>
                tr(l, "PV energy at the AC bus", "Energía FV en el bus CA", "PV-Energie am AC-Inselnetz", "Energia FV no barramento CA"),
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
    let wire_pct = |manual: f64, hourly: Option<f64>, dc: bool| {
        if p.hourly_cables.routed.iter().any(|circuit| circuit.is_dc() == dc) {
            return format!("{} · {}", tr(l, "Linked circuits", "Circuitos vinculados", "Verknüpfte Stromkreise", "Circuitos vinculados"),
                format!("{} {}", pct(manual), tr(l, "for unlinked paths", "en líneas sin enlace", "für unverknüpfte Leitungen", "em linhas sem vínculo")));
        }
        hourly.map_or_else(|| pct(manual), |v| {
            format!("{} · {}", pct(v), tr(l, "design point (hourly)", "punto de diseño (horario)", "Auslegungspunkt (stündlich)", "ponto de projeto (horário)"))
        })
    };
    let model_name = format!("{:?}", p.simulation.transposition.value).replace("HayDavies", "Hay–Davies").replace("Perez", "Perez et al. (1990)");
    let roof_svgs: Vec<String> = p.arrays.iter().filter_map(|a| a.roof.as_ref().map(|r| {
        let cap = format!("{} — {}", a.name.value, s["roof_caption"].replace("%N%", &r.modules.len().to_string()).replace("%S%", &n(r.params.setback, 2)));
        charts::roof_plan(&r.face, &r.modules, &cap, l)
    })).filter(|svg| !svg.is_empty()).collect();
    let mut view = View {
        lang: l.tag().into(), title: s["title"].into(), project_name: p.name.value.clone(), designer: inp.designer.clone(),
        customer: CustomerView { name: if p.customer.name.is_empty() { "—".into() } else { p.customer.name.clone() }, address: p.customer.address.clone() },
        date, kpis, arrays, site_rows,
        weather_notes: {
            let mut notes = vec![];
            if inp.weather.dataset.starts_with("TMY") { notes.push(s["weather_tmy"].into()); }
            if inp.weather.horizon_applied { notes.push(s["note_horizon"].into()); }
            notes
        },
        licence: inp.weather.licence.clone(), loss_rows,
        irr_rows: r.irradiance_ledger.iter().map(|st| {
            let key = format!("irr_{}", st.key);
            let label = s.get(key.as_str()).map(|v| v.to_string()).unwrap_or_else(|| st.label.clone());
            [label, n(st.value, 1), if st.key == "ghi" || st.key == "effective" { String::new() } else { format!("{} %", n(st.delta_pct, 2)) }]
        }).collect(),
        monthly_header: [s["month"].into(), "GHI kWh/m²".into(), "POA kWh/m²".into(), format!("{} kWh", s["energy"])],
        monthly_rows,
        assumption_rows: vec![[s["soiling"].into(), pct(lo.soiling_pct.value)], [s["lid"].into(), pct(lo.lid_pct.value)], [s["mismatch"].into(), pct(lo.mismatch_pct.value)], [s["dc_wiring"].into(), wire_pct(lo.dc_wiring_pct.value, p.hourly_cables.dc_pct, true)], [s["ac_wiring"].into(), wire_pct(lo.ac_wiring_pct.value, p.hourly_cables.ac_pct, false)], [s["availability"].into(), pct(lo.availability_pct.value)], ["Albedo".into(), n(p.site.albedo.value, 2)]],
        models_text: s["models_text"].replace("%MODEL%", &model_name).replace("%PV_MODEL%", match r.manifest.pv_model {
            solarcraft_model::PvModel::SingleDiode => s["single_diode"],
            solarcraft_model::PvModel::TwoDiode => s["two_diode"],
        }),
        notes: r.warnings.iter().filter(|w| w.code != "HORIZON_IN_WEATHER").map(|w| match w.code.as_str() {
            "NEAR_SHADING" => s["note_shading"].replace("%S%", &w.subject),
            "MPP_WINDOW_HOURS" => s["note_mpp"].replace("%S%", &w.subject).replace("%N%", &n(w.value, 0)),
            "DC_BATTERY_LEDGER" => tr(l, &w.text, &format!("La cascada de pérdidas muestra la producción FV como si toda pasara por los inversores; {} kWh cargaron la batería en CC antes de la conversión (ver balance energético).", n(w.value, 0)),
                &format!("Der Verlustbaum zeigt die PV-Erzeugung, als würde sie vollständig gewandelt; {} kWh luden die DC-gekoppelte Batterie vor der Wandlung (siehe Energiebilanz).", n(w.value, 0)),
                &format!("A cascata de perdas mostra a produção FV como se toda passasse pelos inversores; {} kWh carregaram a bateria em CC antes da conversão (ver balanço energético).", n(w.value, 0))),
            _ => w.text.clone(),
        }).collect(),
        disclaimer: s["disclaimer"].into(),
        run_rows: vec![["inputs".into(), r.manifest.inputs_hash.chars().take(12).collect()], ["engine".into(), r.manifest.engine.clone()], ["weather".into(), r.manifest.weather_hash.chars().take(12).collect()], ["hours".into(), r.manifest.hours.to_string()]],
        footer: format!("{} {}", s["generated_with"], env!("CARGO_PKG_VERSION")),
        monthly_svg: charts::monthly_bars(&vals, months, "kWh", l),
        waterfall_svg: charts::waterfall(&waterfall_steps, "kWh", l),
        roof_svgs: roof_svgs.clone(),
        roof_count: roof_svgs.len(),
        sections: plant_sections(inp),
        report_type,
        contents: vec![],
        figures: vec![],
        s,
    };
    profiles::compose(&mut view, inp);
    view
}

impl View {
    pub fn add_roof_photo(&mut self, caption: String, svg: String) {
        profiles::add_roof_photo(self, caption, svg);
    }
}

/// Plant sections: every value is read from the stored run (traceable via the run's inputs hash).
pub fn plant_sections(inp: &ReportInput) -> Vec<Section> {
    let l = inp.lang;
    let r = inp.result;
    let p = inp.project;
    let n = |v: f64, d: usize| l.num(v, d);
    let mut out = vec![];
    if !r.electrical.is_empty() {
        let mut rows = vec![];
        for u in &r.electrical {
            rows.push(vec![u.name.clone(), format!("{} kWp / {} kW", n(u.stc_kw, 2), n(u.ac_kw, 1)), n(u.dc_kwh, 0), n(u.ac_kwh, 0), format!("{} kWh", n(u.clipped_kwh, 0))]);
            for m in &u.mppts {
                rows.push(vec![format!("  MPPT {}: {}", m.index, m.strings), format!("{} kWp", n(m.stc_kw, 2)), n(m.dc_kwh, 0), String::new(),
                               format!("{} / {} kWh · {} h", n(m.mismatch_kwh, 0), n(m.limit_loss_kwh, 0), m.limited_hours)]);
            }
        }
        out.push(Section { id: "inverter_performance", title: tr(l, "Inverters and string configuration", "Inversores y configuración de cadenas", "Wechselrichter und Stringverschaltung", "Inversores e configuração de strings"),
            intro: tr(l, "Simulated per MPP tracker: parallel-string mismatch and tracker-limit (voltage window, input current) losses are shown in the last column.", "Simulado por seguidor MPP: la última columna muestra las pérdidas por desajuste de cadenas en paralelo y por límites del seguidor (ventana de tensión, corriente de entrada).", "Je MPP-Tracker simuliert: Die letzte Spalte zeigt Fehlanpassung paralleler Strings und Tracker-Grenzen (Spannungsfenster, Eingangsstrom).", "Simulado por rastreador MPP: a última coluna mostra perdas por descasamento de strings em paralelo e por limites do rastreador (janela de tensão, corrente de entrada)."),
            header: vec![tr(l, "Inverter / MPPT", "Inversor / MPPT", "Wechselrichter / MPPT", "Inversor / MPPT"), tr(l, "Rating", "Potencia", "Leistung", "Potência"), "DC kWh".into(), "AC kWh".into(), tr(l, "Clipping · mismatch / limits", "Recorte · desajuste / límites", "Abregelung · Fehlanpassung / Grenzen", "Corte · descasamento / limites")], rows });
    }
    if let Some(ev) = &r.ev {
        let rows = [
            (tr(l, "Requested at the battery", "Solicitada en la batería", "An der Batterie angefordert", "Solicitada na bateria"), ev.requested_kwh),
            (tr(l, "Scheduled at the battery", "Programada en la batería", "An der Batterie eingeplant", "Programada na bateria"), ev.charged_kwh),
            (tr(l, "Charger electricity", "Electricidad del cargador", "Ladestrombedarf", "Eletricidade do carregador"), ev.electrical_kwh),
            (tr(l, "Unscheduled", "No programada", "Nicht eingeplant", "Não programada"), ev.unscheduled_kwh),
        ].into_iter().map(|(label, value)| vec![label, format!("{} kWh", n(value, 1))]).collect();
        out.push(Section { id: "ev", title: tr(l, "Electric vehicle charging", "Carga del vehículo eléctrico", "Elektrofahrzeugladung", "Carregamento do veículo elétrico"),
            intro: String::new(), header: vec![], rows });
    }
    if let Some(heat) = &r.heating {
        let total_heat = r.heat_store.as_ref().is_some_and(|tank| tank.hot_water_kwh > 0.0);
        let demand_label = if total_heat { tr(l, "Total heat demand", "Demanda térmica total", "Gesamtwärmebedarf", "Demanda térmica total") }
            else { tr(l, "Space heat demand", "Demanda de calefacción", "Raumwärmebedarf", "Demanda de aquecimento") };
        let rows = [
            (demand_label, heat.demand_kwh),
            (tr(l, "Heat pump electricity", "Electricidad de la bomba de calor", "Strom der Wärmepumpe", "Eletricidade da bomba de calor"), heat.pump_electric_kwh),
            (tr(l, "Resistance backup", "Respaldo eléctrico", "Elektrische Zusatzheizung", "Aquecimento elétrico auxiliar"), heat.backup_kwh),
            (tr(l, "Unserved heat", "Calor no atendido", "Ungedeckter Wärmebedarf", "Calor não atendido"), heat.unmet_heat_kwh),
        ].into_iter().map(|(label, value)| vec![label, format!("{} kWh", n(value, 1))]).collect();
        let title = if total_heat { tr(l, "Space heating and hot water", "Calefacción y agua caliente", "Raumheizung und Warmwasser", "Aquecimento ambiente e água quente") }
            else { tr(l, "Space heating", "Calefacción", "Raumheizung", "Aquecimento ambiente") };
        out.push(Section { id: "heating", title, intro: String::new(), header: vec![], rows });
    }
    if let Some(fleet) = &r.fleet {
        let rows = fleet.groups.iter().map(|g| vec![g.name.clone(), g.vehicles.to_string(),
            format!("{}–{}", n(g.min_soc_pct, 1), n(g.max_soc_pct, 1)), n(g.charged_kwh, 1),
            n(g.discharged_kwh, 1), g.departures_below_target.to_string(), n(g.unmet_travel_kwh, 1)]).collect();
        out.push(Section { id: "ev_fleet", title: tr(l, "EV fleet and bidirectional charging", "Flota eléctrica y carga bidireccional", "Elektroflotte und bidirektionales Laden", "Frota elétrica e carregamento bidirecional"),
            intro: String::new(), header: vec![
                tr(l,"Group","Grupo","Gruppe","Grupo"), tr(l,"Vehicles","Vehículos","Fahrzeuge","Veículos"), "SOC %".into(),
                tr(l,"Charge (kWh)","Carga (kWh)","Ladung (kWh)","Carga (kWh)"), "V2H kWh".into(),
                tr(l,"Below departure SOC","Salidas bajo el SOC","Unter Abfahrt-SOC","Saídas abaixo do SOC"),
                tr(l,"Unmet travel (kWh)","Viajes no cubiertos (kWh)","Fehlende Fahrenergie (kWh)","Energia de viagens não suprida (kWh)")], rows });
    }
    if let Some(store) = &r.heat_store {
        let rows = [
            (tr(l,"Charge","Carga","Ladung","Carga"), store.charge_heat_kwh),
            (tr(l,"Discharge","Descarga","Entladung","Descarga"), store.discharge_heat_kwh),
            (tr(l,"Standing loss","Pérdida en reposo","Stillstandsverlust","Perda em repouso"), store.standing_loss_kwh),
            (tr(l,"Hot water demand","Demanda de agua caliente","Warmwasserbedarf","Demanda de água quente"), store.hot_water_kwh),
        ].into_iter().map(|(label,value)| vec![label, format!("{} kWh", n(value,1))]).collect();
        out.push(Section { id: "heat_store", title: tr(l,"Thermal storage","Almacenamiento térmico","Wärmespeicher","Armazenamento térmico"),
            intro: String::new(), header: vec![], rows });
    }
    if !r.clusters.is_empty() {
        let rows = r.clusters.iter().map(|c| vec![c.name.clone(), n(c.storage_kwh,1),
            n(c.balance.load_served_kwh,1), n(c.balance.unserved_kwh,1), n(c.balance.generator_kwh,1),
            c.fuel_unit.as_ref().map_or("–".into(), |unit| format!("{} {unit}", n(c.balance.generator_fuel,1)))]).collect();
        out.push(Section { id: "island_clusters", title: tr(l,"Independent islands","Islas independientes","Unabhängige Inseln","Ilhas independentes"),
            intro: String::new(), header: vec![tr(l,"Island","Isla","Insel","Ilha"), tr(l,"Battery (kWh)","Batería (kWh)","Batterie (kWh)","Bateria (kWh)"),
                tr(l,"Served (kWh)","Atendida (kWh)","Versorgt (kWh)","Atendida (kWh)"), tr(l,"Unserved (kWh)","No atendida (kWh)","Ungedeckt (kWh)","Não atendida (kWh)"),
                tr(l,"Generator (kWh)","Generador (kWh)","Generator (kWh)","Gerador (kWh)"), tr(l,"Fuel","Combustible","Kraftstoff","Combustível")], rows });
    }
    if let Some(cost) = &r.island_finance {
        let rows = [
            (tr(l, "Investment", "Inversión", "Investition", "Investimento"), format!("{} {}", n(cost.investment, 2), cost.currency)),
            (tr(l, "Discounted lifetime cost", "Coste descontado de vida útil", "Abgezinste Lebenszykluskosten", "Custo descontado do ciclo de vida"),
                format!("{} {}", n(cost.discounted_cost, 2), cost.currency)),
            (tr(l, "Cost per served kWh", "Coste por kWh suministrado", "Kosten je versorgter kWh", "Custo por kWh suprido"),
                cost.cost_per_served_kwh.map_or("–".into(), |v| format!("{} {}/kWh", n(v, 3), cost.currency))),
        ].into_iter().map(|(label, value)| vec![label, value]).collect();
        out.push(Section { id: "island_cost", title: tr(l, "Off-grid economics", "Economía del sistema aislado", "Kosten der Inselanlage", "Economia do sistema isolado"),
            intro: tr(l, "The discounted calculation repeats the simulated year of delivered energy and fuel volume.",
                "El cálculo descontado repite el año simulado de energía suministrada y combustible.",
                "Die Abzinsung wiederholt das simulierte Jahr für gelieferte Energie und Kraftstoffmenge.",
                "O cálculo descontado repete o ano simulado de energia fornecida e combustível."), header: vec![], rows });
    }
    let b = &r.balance;
    if b.load_kwh > 0.0 || b.batt_charge_kwh > 0.0 {
        let kwh = |v: f64| format!("{} kWh", n(v, 0));
        let mut rows = vec![
            vec![tr(l, "PV generation before curtailment", "Generación FV antes de recortes", "PV-Erzeugung vor Abregelung", "Geração FV antes do corte"), kwh(b.pv_generation_kwh)],
            vec![tr(l, "Consumption", "Consumo", "Verbrauch", "Consumo"), kwh(b.load_kwh)],
            vec![tr(l, "Consumption served", "Consumo atendido", "Versorgter Verbrauch", "Consumo atendido"), kwh(b.load_served_kwh)],
            vec![tr(l, "PV used directly", "FV usada directamente", "PV-Direktverbrauch", "FV usada diretamente"), kwh(b.pv_to_load_kwh)],
            vec![tr(l, "Grid import", "Importación de la red", "Netzbezug", "Importação da rede"), kwh(b.import_kwh)],
            vec![tr(l, "Grid export", "Exportación a la red", "Netzeinspeisung", "Exportação para a rede"), kwh(b.export_kwh)],
            vec![tr(l, "Curtailed (export limit, outages)", "Limitada (límite de exportación, cortes)", "Abgeregelt (Einspeiselimit, Ausfälle)", "Limitada (limite de exportação, interrupções)"), kwh(b.curtailed_kwh)],
            vec![tr(l, "Self-consumption rate", "Tasa de autoconsumo", "Eigenverbrauchsquote", "Taxa de autoconsumo"), format!("{} %", n(b.self_consumption_rate * 100.0, 1))],
            vec![tr(l, "Autarky (self-sufficiency)", "Autarquía (autosuficiencia)", "Autarkiegrad", "Autossuficiência"), format!("{} %", n(b.autarky * 100.0, 1))],
        ];
        if p.workflow.system.islanded() {
            let label = tr(l, "Surplus curtailed", "Excedente limitado", "Abgeregelter Überschuss", "Excedente limitado");
            rows.drain(4..7);
            rows.push(vec![label, kwh(b.curtailed_kwh)]);
            if let Some(generator) = p.storage.as_ref().and_then(|st| st.generator_id.as_deref()).and_then(|id| p.library.generator(id)) {
                rows.push(vec![tr(l, "Generator energy", "Energía del generador", "Generatorenergie", "Energia do gerador"), kwh(b.generator_kwh)]);
                rows.push(vec![tr(l, "Generator fuel", "Combustible del generador", "Generator-Kraftstoff", "Combustível do gerador"), format!("{} {}", n(b.generator_fuel, 1), generator.fuel_unit)]);
            }
            if b.generator_hours > 0 {
                rows.push(vec![tr(l, "Generator hours and starts", "Horas y arranques del generador", "Generatorlaufzeit und Starts", "Horas e partidas do gerador"),
                    format!("{} h / {}", b.generator_hours, b.generator_starts)]);
                rows.push(vec![tr(l, "Generator energy to storage", "Energía del generador a la batería", "Generatorenergie zum Speicher", "Energia do gerador para a bateria"), kwh(b.generator_charge_kwh)]);
            }
            rows.push(vec![tr(l, "Unserved demand", "Demanda no atendida", "Nicht versorgter Bedarf", "Demanda não atendida"), kwh(b.unserved_kwh)]);
        } else if b.outage_hours > 0 { rows.push(vec![tr(l, "Outage hours · unserved critical load", "Horas de corte · carga crítica no atendida", "Ausfallstunden · nicht versorgte kritische Last", "Horas de interrupção · carga crítica não atendida"), format!("{} h · {}", b.outage_hours, kwh(b.unserved_kwh))]); }
        out.push(Section { id: "energy_balance", title: tr(l, "Energy balance", "Balance energético", "Energiebilanz", "Balanço energético"), intro: tr(l, "Hourly simulation for one year; the balance closes in every hour.", "Simulación horaria de un año; el balance cierra en cada hora.", "Stündliche Jahressimulation; die Bilanz ist in jeder Stunde geschlossen.", "Simulação horária de um ano; o balanço fecha em cada hora."), header: vec![], rows });
    }
    if let (Some(st), Some(bat)) = (&p.storage, p.storage.as_ref().and_then(|st| p.library.battery(&st.battery_id))) {
        let coupling = match &st.coupling { solarcraft_model::plant::Coupling::DcHybrid { unit_id } => format!("{} ({unit_id})", tr(l, "DC, hybrid inverter port", "CC, puerto del inversor híbrido", "DC, Hybrid-Wechselrichter", "CC, porta do inversor híbrido")),
            solarcraft_model::plant::Coupling::AcCoupled { battery_inverter_id } => format!("{} ({battery_inverter_id})", tr(l, "AC, battery inverter", "CA, inversor de batería", "AC, Batteriewechselrichter", "CA, inversor de bateria")),
            solarcraft_model::plant::Coupling::Integrated => tr(l, "AC, integrated battery system", "CA, sistema de batería integrado", "AC, integriertes Batteriesystem", "CA, sistema de bateria integrado") };
        let rows = vec![
            vec![tr(l, "Battery", "Batería", "Batterie", "Bateria"), format!("{} × {} {} ({} kWh)", st.units, bat.manufacturer, bat.model, n(bat.capacity_kwh * st.units as f64, 1))],
            vec![tr(l, "Connection", "Conexión", "Anschluss", "Conexão"), coupling],
            vec![tr(l, "Operating window", "Ventana de operación", "Betriebsfenster", "Janela de operação"), format!("{}–{} % SOC", n(st.soc_min_pct, 0), n(st.soc_max_pct, 0))],
            vec![tr(l, "Allocation: backup · self-consumption · grid services", "Asignación: respaldo · autoconsumo · servicios de red", "Aufteilung: Notstrom · Eigenverbrauch · Netzdienste", "Alocação: reserva · autoconsumo · serviços de rede"), format!("{} · {} · {} %", n(st.backup_reserve_pct, 0), n(st.self_consumption_pct(), 0), n(st.grid_services_pct, 0))],
            vec![tr(l, "Charged · discharged", "Cargada · descargada", "Geladen · entladen", "Carregada · descarregada"), format!("{} · {} kWh", n(b.batt_charge_kwh, 0), n(b.batt_discharge_kwh, 0))],
            vec![tr(l, "Full cycles per year", "Ciclos completos por año", "Vollzyklen pro Jahr", "Ciclos completos por ano"), n(b.battery_full_cycles, 0)],
        ];
        out.push(Section { id: "battery", title: tr(l, "Battery storage", "Almacenamiento en batería", "Batteriespeicher", "Armazenamento em bateria"), intro: String::new(), header: vec![], rows });
    }
    if let Some(f) = &r.finance {
        if f.investment > 0.0 {
            let c = &f.currency;
            let m = |v: f64| format!("{} {c}", n(v, 0));
            let mut rows = vec![
                vec![tr(l, "Investment", "Inversión", "Investition", "Investimento"), m(f.investment)],
                vec![tr(l, "Electricity bill, year 1, without the plant", "Factura eléctrica año 1 sin la planta", "Stromrechnung Jahr 1 ohne Anlage", "Conta de energia ano 1 sem a usina"), m(f.bill_without.total)],
                vec![tr(l, "Electricity bill, year 1, with the plant", "Factura eléctrica año 1 con la planta", "Stromrechnung Jahr 1 mit Anlage", "Conta de energia ano 1 com a usina"), m(f.bill_with.total)],
                vec![tr(l, "Net present value", "Valor presente neto", "Kapitalwert", "Valor presente líquido"), m(f.npv)],
                vec![tr(l, "Internal rate of return", "Tasa interna de retorno", "Interner Zinsfuß", "Taxa interna de retorno"), f.irr_pct.map_or("–".into(), |x| format!("{} %", n(x, 1)))],
                vec![tr(l, "Payback", "Retorno de la inversión", "Amortisation", "Retorno do investimento"), f.payback_years.map_or("–".into(), |x| format!("{} a", n(x, 1)))],
                vec!["LCOE".into(), format!("{} {c}/kWh", n(f.lcoe_per_kwh, 3))],
            ];
            rows.retain(|r| !r[1].is_empty());
            out.push(Section { id: "finance", title: tr(l, "Financial analysis", "Análisis financiero", "Wirtschaftlichkeit", "Análise financeira"), intro: p.tariff.name.clone(), header: vec![], rows });
            let years: Vec<Vec<String>> = f.years.iter().map(|y| vec![y.year.to_string(), n(y.savings + y.export_revenue, 0), n(-(y.om + y.replacements + y.loan), 0), n(y.cash_flow, 0), n(y.cumulative, 0)]).collect();
            out.push(Section { id: "cash_flow", title: tr(l, "Cash flow", "Flujo de caja", "Cashflow", "Fluxo de caixa"), intro: c.clone(),
                header: vec![tr(l, "Year", "Año", "Jahr", "Ano"), tr(l, "Savings + export", "Ahorro + exportación", "Ersparnis + Einspeisung", "Economia + exportação"), tr(l, "Costs", "Costes", "Kosten", "Custos"), tr(l, "Cash flow", "Flujo", "Cashflow", "Fluxo"), tr(l, "Cumulative", "Acumulado", "Kumuliert", "Acumulado")], rows: years });
        }
    }
    out
}

pub(crate) static FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-It.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-Semibold.ttf"),
    include_bytes!("../assets/fonts/SourceSans3-Bold.ttf"),
];
