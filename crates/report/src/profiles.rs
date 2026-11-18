// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{charts, tr, ReportInput, View};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReportType {
    CustomerShort,
    #[default]
    CustomerExtended,
    SystemPlanner,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
    Figure { file: String, index: usize, caption: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentSection {
    pub id: &'static str,
    pub title: String,
    pub blocks: Vec<ContentBlock>,
}

impl ContentSection {
    pub(crate) fn new(id: &'static str, title: impl Into<String>) -> Self {
        Self { id, title: title.into(), blocks: vec![] }
    }

    pub(crate) fn text(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() { self.blocks.push(ContentBlock::Text { text }); }
    }

    pub(crate) fn table(&mut self, header: Vec<String>, rows: impl IntoIterator<Item = impl Into<Vec<String>>>) {
        let rows: Vec<_> = rows.into_iter().map(Into::into).collect();
        if !rows.is_empty() { self.blocks.push(ContentBlock::Table { header, rows }); }
    }

    pub(crate) fn figure(&mut self, svg: String, caption: impl Into<String>, view: &mut View) {
        let index = view.figures.len();
        let file = format!("figure-{index}.svg");
        view.figures.push(svg);
        self.blocks.push(ContentBlock::Figure { file, index, caption: caption.into() });
    }
}

pub(crate) fn add_roof_photo(view: &mut View, caption: String, svg: String) {
    let lang = crate::i18n::Lang::parse(&view.lang).unwrap_or(crate::i18n::Lang::En);
    let index = view.contents.iter().position(|section| section.id == "photo_plan");
    let mut section = index.map(|i| view.contents.remove(i)).unwrap_or_else(|| ContentSection::new("photo_plan",
        tr(lang, "Calibrated roof photographs", "Fotos de cubierta calibradas", "Kalibrierte Dachfotos", "Fotos calibradas do telhado")));
    section.figure(svg, caption, view);
    let index = index.unwrap_or_else(|| view.contents.iter().position(|s| matches!(s.id, "notes" | "run")).unwrap_or(view.contents.len()));
    view.contents.insert(index, section);
}

pub(crate) fn compose(view: &mut View, input: &ReportInput) {
    use ReportType::*;
    let l = input.lang;
    let s = &view.s;
    let r = input.result;
    let short = view.report_type == CustomerShort;
    if short {
        view.title = tr(l, "Customer summary", "Resumen para el cliente", "Kundenkurzbericht", "Resumo para o cliente");
        view.sections.retain(|section| !matches!(section.id, "inverter_performance" | "cash_flow"));
    } else if view.report_type == SystemPlanner {
        view.title = tr(l, "System planner report", "Informe de diseño del sistema", "Anlagenplanungsbericht", "Relatório de projeto do sistema");
    }
    let mut overview = ContentSection::new("production", s["monthly_chart"]);
    overview.figure(view.monthly_svg.clone(), "", view);
    view.contents.push(overview);

    let mut equipment = equipment_summary(input);
    if !short {
        for array in &view.arrays {
            equipment.text(array.name.clone());
            equipment.table(vec![], array.rows.iter().cloned());
        }
    }
    view.contents.push(equipment);
    if !short {
        let mut site = ContentSection::new("site_weather", view.s["site_weather"]);
        site.table(vec![], view.site_rows.iter().cloned());
        site.text(view.weather_notes.join(" · "));
        site.text(view.licence.clone());
        view.contents.push(site);

        let mut monthly = ContentSection::new("monthly_results", view.s["monthly_table"]);
        monthly.table(view.monthly_header.to_vec(), view.monthly_rows.iter().cloned());
        view.contents.push(monthly);
    }

    for section in view.sections.clone() {
        let mut content = ContentSection::new(section.id, section.title);
        content.text(section.intro);
        content.table(section.header, section.rows);
        if section.id == "energy_balance" && r.balance.load_kwh > 0.0 {
            let b = &r.balance;
            let series = [
                (tr(l, "Direct solar", "Solar directa", "Direkte PV", "Solar direta"), b.pv_to_load_kwh),
                (tr(l, "Battery", "Batería", "Batterie", "Bateria"), b.battery_to_load_kwh),
                (tr(l, "Grid", "Red", "Netz", "Rede"), b.grid_to_load_kwh),
                (tr(l, "Generator", "Generador", "Generator", "Gerador"), b.generator_to_load_kwh),
                (tr(l, "Unserved demand", "Demanda no atendida", "Nicht versorgter Bedarf", "Demanda não atendida"), b.unserved_kwh + b.shed_noncritical_kwh),
            ];
            let caption = tr(l, "Sources of customer electricity", "Fuentes de electricidad del cliente", "Quellen des Kundenstroms", "Fontes de eletricidade do cliente");
            content.figure(charts::load_supply(&series, b.load_kwh, l), caption, view);
        }
        if section.id == "finance" {
            if let Some(finance) = &r.finance {
                let caption = tr(l, "Cumulative cash flow", "Flujo de caja acumulado", "Kumulierter Cashflow", "Fluxo de caixa acumulado");
                let series: Vec<_> = finance.years.iter().map(|year| (year.year, year.cumulative)).collect();
                if !series.is_empty() { content.figure(charts::cash_flow(&series, &finance.currency, l), caption, view); }
            }
        }
        view.contents.push(content);
    }

    if !short {
        crate::detail::results(view, input);
        let mut losses = ContentSection::new("losses", view.s["losses"]);
        losses.text(view.s["losses_intro"]);
        losses.figure(view.waterfall_svg.clone(), "", view);
        losses.table(vec![view.s["irradiation"].into(), "kWh/m²".into(), view.s["change"].into()], view.irr_rows.iter().cloned());
        losses.table(vec![view.s["step"].into(), format!("{} (kWh)", view.s["energy"]), view.s["change"].into()], view.loss_rows.iter().cloned());
        view.contents.push(losses);

        let mut assumptions = ContentSection::new("assumptions", view.s["assumptions"]);
        assumptions.table(vec![], view.assumption_rows.iter().cloned());
        assumptions.text(view.models_text.clone());
        view.contents.push(assumptions);

        if !view.roof_svgs.is_empty() {
            let mut plans = ContentSection::new("roof_plans", view.s["roof_plan"]);
            for svg in view.roof_svgs.clone() { plans.figure(svg, "", view); }
            view.contents.push(plans);
        }
        if let Some(scene) = input.project.scene.as_ref().filter(|scene| !scene.objects.is_empty() || !scene.fields.is_empty()) {
            use solarcraft_scene::view::{render, svg, Camera, ColorBy};
            let title = tr(l, "3D layout", "Diseño 3D", "3D-Layout", "Layout 3D");
            let mut plans = ContentSection::new("scene_plans", title.clone());
            let camera = Camera { width: 900.0, height: 480.0, ..Default::default() };
            plans.figure(svg(&render(scene, &camera, ColorBy::Material, None, None), &title), "", view);
            if view.report_type == SystemPlanner && scene.fields.iter().any(|field| !field.strings.is_empty()) {
                let title = tr(l, "String layout", "Distribución de cadenas", "Stringplan", "Layout das strings");
                let plan = Camera { pitch_deg: 90.0, yaw_deg: 0.0, ..camera };
                plans.figure(svg(&render(scene, &plan, ColorBy::String, None, None), &title), title, view);
            }
            view.contents.push(plans);
        }
    }
    if view.report_type == SystemPlanner {
        view.contents.extend(planner_sections(input));
        crate::detail::planner(view, input);
    }
    if !view.notes.is_empty() {
        let mut notes = ContentSection::new("notes", view.s["notes"]);
        for note in &view.notes { notes.text(note); }
        view.contents.push(notes);
    }
    let mut run = ContentSection::new("run", view.s["run_info"]);
    if short { run.text(format!("{} — {}", input.weather.provider, input.weather.dataset)); }
    run.text(view.disclaimer.clone());
    if !short { run.table(vec![], view.run_rows.iter().cloned()); }
    view.contents.push(run);
}

fn equipment_summary(input: &ReportInput) -> ContentSection {
    let p = input.project;
    let l = input.lang;
    let electrical = solarcraft_model::derive::electrical(p);
    let assigned = solarcraft_model::derive::modules_by_array(&electrical);
    let mut counts = BTreeMap::<(String, String), u32>::new();
    for array in &p.arrays {
        if let (Some(&count), Some(module)) = (assigned.get(&array.id), p.library.module(&array.module_id.value)) {
            let kind = tr(l, "PV module", "Módulo FV", "PV-Modul", "Módulo FV");
            *counts.entry((kind, format!("{} {}", module.manufacturer, module.model))).or_default() += count;
        }
    }
    for unit in &electrical.inverters {
        if let Some(inverter) = p.library.inverter(&unit.inverter_id) {
            let kind = tr(l, "Inverter", "Inversor", "Wechselrichter", "Inversor");
            *counts.entry((kind, format!("{} {}", inverter.manufacturer, inverter.model))).or_default() += 1;
        }
    }
    if let Some((storage, battery)) = p.storage.as_ref().and_then(|s| p.library.battery(&s.battery_id).map(|b| (s, b))) {
        let kind = tr(l, "Battery", "Batería", "Batterie", "Bateria");
        counts.insert((kind, format!("{} {}", battery.manufacturer, battery.model)), storage.units);
    }
    let mut section = ContentSection::new("equipment", tr(l, "System components", "Componentes del sistema", "Anlagenkomponenten", "Componentes do sistema"));
    section.table(vec![
        tr(l, "Equipment", "Equipo", "Gerät", "Equipamento"),
        tr(l, "Model", "Modelo", "Modell", "Modelo"),
        tr(l, "Quantity", "Cantidad", "Anzahl", "Quantidade"),
    ], counts.into_iter().map(|((kind, model), count)| vec![kind, model, count.to_string()]));
    section
}

fn planner_sections(input: &ReportInput) -> Vec<ContentSection> {
    let p = input.project;
    let r = input.result;
    let l = input.lang;
    let n = |value, digits| l.num(value, digits);
    let electrical = solarcraft_model::derive::electrical(p);
    let assigned = solarcraft_model::derive::modules_by_array(&electrical);
    let mut datasheets = ContentSection::new("datasheets", tr(l, "Selected equipment specifications", "Especificaciones de equipos", "Daten der ausgewählten Geräte", "Especificações dos equipamentos"));
    let mut seen = std::collections::HashSet::new();
    for array in &p.arrays {
        if !assigned.contains_key(&array.id) || !seen.insert(&array.module_id.value) { continue; }
        if let Some(module) = p.library.module(&array.module_id.value) {
            datasheets.text(format!("{} {}", module.manufacturer, module.model));
            datasheets.table(vec![], [
                [tr(l, "STC power", "Potencia STC", "STC-Leistung", "Potência STC"), format!("{} W", n(module.pmp_stc(), 1))],
                ["Voc / Isc".into(), format!("{} V / {} A", n(module.v_oc, 2), n(module.i_sc, 2))],
                ["Vmp / Imp".into(), format!("{} V / {} A", n(module.v_mp, 2), n(module.i_mp, 2))],
                [tr(l, "Dimensions", "Dimensiones", "Abmessungen", "Dimensões"), format!("{} × {} m", n(module.length_m, 3), n(module.width_m, 3))],
            ]);
        }
    }
    seen.clear();
    for unit in &electrical.inverters {
        if !seen.insert(&unit.inverter_id) { continue; }
        if let Some(inverter) = p.library.inverter(&unit.inverter_id) {
            datasheets.text(format!("{} {}", inverter.manufacturer, inverter.model));
            datasheets.table(vec![], [
                [tr(l, "Maximum AC output", "Potencia CA máxima", "Maximale AC-Leistung", "Potência CA máxima"), format!("{} kW", n(inverter.pac_max_w / 1000.0, 2))],
                [tr(l, "Maximum DC voltage", "Tensión CC máxima", "Maximale DC-Spannung", "Tensão CC máxima"), format!("{} V", n(inverter.v_dc_max, 0))],
                [tr(l, "MPP trackers", "Seguidores MPP", "MPP-Tracker", "Rastreadores MPP"), inverter.mppt_count.to_string()],
            ]);
        }
    }
    let mut out = vec![datasheets];
    if let Some(scene) = &p.scene {
        let mut routing = ContentSection::new("string_routes", tr(l, "String cable routes", "Rutas de cableado de cadenas", "String-Kabelwege", "Rotas de cabos das strings"));
        routing.table(vec![
            tr(l, "Field", "Campo", "Modulfeld", "Campo"), "String".into(),
            tr(l, "Modules", "Módulos", "Module", "Módulos"),
            tr(l, "Jumpers", "Puentes", "Modulverbinder", "Pontes"),
            tr(l, "To inverter", "Al inversor", "Zum Wechselrichter", "Ao inversor"),
        ], scene.fields.iter().flat_map(|field| field.strings.iter().map(move |route| [
            field.name.clone(), route.id.clone(), route.modules.len().to_string(), format!("{} m", n(route.jumper_m, 1)),
            field.inverter_at.map_or("–".into(), |_| format!("{} m", n(route.home_run_m, 1))),
        ])));
        if !routing.blocks.is_empty() { out.push(routing); }
    }
    if !r.module_shading.is_empty() {
        let mut shading = ContentSection::new("field_shading", tr(l, "3D shading by field", "Sombreado 3D por campo", "3D-Verschattung je Modulfeld", "Sombreamento 3D por campo"));
        shading.table(vec![tr(l, "Field", "Campo", "Modulfeld", "Campo"),
            tr(l, "Geometric beam loss", "Pérdida directa geométrica", "Geometrischer Direktstrahlungsverlust", "Perda direta geométrica"),
            tr(l, "Sky factor", "Factor de cielo", "Himmelsfaktor", "Fator de céu"),
        ], r.module_shading.iter().map(|field| [
            field.field.clone(), format!("{} %", n(field.beam_area_loss_pct, 2)), format!("{} %", n(field.diffuse_factor * 100.0, 1)),
        ]));
        out.push(shading);
    }
    out
}
