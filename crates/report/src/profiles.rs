// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{charts, tr, ReportInput, View};
use serde::{Deserialize, Serialize};

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
    let chart = if input.project.workflow.system.islanded() {
        tr(l, "Monthly PV energy at the AC bus", "Energía FV mensual en el bus CA", "Monatliche PV-Energie am AC-Inselnetz", "Energia FV mensal no barramento CA")
    } else { s["monthly_chart"].into() };
    let mut overview = ContentSection::new("production", chart);
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
                ("V2H".into(), b.ev_to_load_kwh),
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
    if !short && !input.project.hourly_cables.is_empty() {
        let mut wiring = ContentSection::new("hourly_wiring", tr(l, "Hourly cable losses", "Pérdidas horarias de cableado", "Stündliche Leitungsverluste", "Perdas horárias nos cabos"));
        let n = |series: &[f32]| l.num(series.iter().map(|&w| f64::from(w)).sum::<f64>() / 1000.0, 2);
        wiring.table(vec![], [
            ["DC".into(), format!("{} kWh", n(&r.hourly.dc_cable_wh))],
            ["AC".into(), format!("{} kWh", n(&r.flows.ac_cable_loss))],
        ]);
        if !r.routed_cables.is_empty() {
            wiring.table(vec![tr(l,"Circuit","Circuito","Stromkreis","Circuito"), "kWh".into()],
                r.routed_cables.iter().map(|c| [c.label.clone(), l.num(c.loss_kwh,2)]));
        }
        view.contents.push(wiring);
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
    let mut section = ContentSection::new("equipment", tr(l, "System components", "Componentes del sistema", "Anlagenkomponenten", "Componentes do sistema"));
    section.table(vec![
        tr(l, "Equipment", "Equipo", "Gerät", "Equipamento"),
        tr(l, "Model", "Modelo", "Modell", "Modelo"),
        tr(l, "Quantity", "Cantidad", "Anzahl", "Quantidade"),
    ], solarcraft_model::derive::equipment_counts(p).into_iter().map(|row| {
        let kind = match row.kind {
            "module" => tr(l, "PV module", "Módulo FV", "PV-Modul", "Módulo FV"),
            "inverter" => tr(l, "Inverter", "Inversor", "Wechselrichter", "Inversor"),
            "battery" => tr(l, "Battery", "Batería", "Batterie", "Bateria"),
            "battery_inverter" => tr(l, "Battery inverter", "Inversor de batería", "Batteriewechselrichter", "Inversor de bateria"),
            "generator" => tr(l, "Generator", "Generador", "Generator", "Gerador"),
            _ => row.kind.into(),
        };
        vec![kind, row.model, row.quantity.to_string()]
    }));
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
    if !p.cable_plan.is_empty() {
        let mut cables = ContentSection::new("specified_cables", tr(l, "Cable estimates", "Estimación de cables", "Leitungsverluste", "Estimativa de cabos"));
        cables.text(tr(l, "Resistive loss at the specified operating point. See the wiring assumptions and hourly cable loss section for the applied model.",
            "Pérdida resistiva en el punto indicado. Consulte los supuestos y las pérdidas horarias para ver el modelo aplicado.",
            "Ohmscher Verlust am angegebenen Betriebspunkt. Die angewandte Methode steht in den Annahmen und stündlichen Leitungsverlusten.",
            "Perda resistiva no ponto indicado. Veja as premissas e as perdas horárias para o modelo aplicado."));
        cables.table(vec![
            tr(l, "Circuit", "Circuito", "Stromkreis", "Circuito"),
            "mm²".into(), "m".into(), "ΔV (%)".into(), "W".into(),
        ], p.cable_plan.iter().filter_map(|c| {
            let e = c.estimate().ok()?;
            Some([c.label.clone(), n(c.area_mm2, 2), n(e.conductor_m, 1), n(e.drop_pct, 2), n(e.loss_w, 1)])
        }));
        out.push(cables);
        use solarcraft_model::protection::{Standard, Status};
        let assessments: Vec<_> = p.cable_plan.iter().filter_map(|row| {
            let circuit = solarcraft_model::cable::resolve(p, &electrical, row).ok()?;
            let estimate = circuit.estimate().ok()?;
            Some((circuit, estimate))
        }).collect();
        let status = |code| match code {
            Status::Pass => tr(l,"Pass","Cumple","Bestanden","Aprovado"),
            Status::Fail => tr(l,"Fail","Falla","Fehlgeschlagen","Falha"),
            Status::Incomplete => tr(l,"Incomplete","Incompleto","Unvollständig","Incompleto"),
        };
        let format = |value: Option<f64>| value.map_or("–".into(), |a| n(a, 3));
        let circuit_name = tr(l,"Circuit","Circuito","Stromkreis","Circuito");
        let rule_set = tr(l,"Rule set","Norma","Norm","Norma");
        let outcome = tr(l,"Status","Estado","Status","Estado");
        let mut checks = ContentSection::new("protection_checks", tr(l,"Cable protection checks","Comprobaciones de protección","Schutzprüfungen für Leitungen","Verificações de proteção"));
        checks.table(vec![circuit_name.clone(), rule_set.clone(), outcome.clone(),
            tr(l,"Minimum device (A)","Protección mínima (A)","Mindestschutz (A)","Proteção mínima (A)")],
            assessments.iter().filter_map(|(circuit, estimate)| {
                let e = estimate.code_checks.as_ref()?;
                let standard = match e.standard { Standard::Nec2023Pv => "NEC 2023 PV DC", Standard::Iec60364 => "IEC 60364-4-43" };
                Some([circuit.label.clone(), standard.into(), status(e.status), format(e.minimum_device_a)])
            }));
        if !checks.blocks.is_empty() { out.push(checks); }

        let mut fault = ContentSection::new("fault_coordination", tr(l, "Fault duty and device coordination",
            "Corriente de falla y coordinación", "Fehlerstrom und Schutzkoordination", "Corrente de falta e coordenação"));
        fault.table(vec![circuit_name.clone(), tr(l,"Device pair","Par de dispositivos","Gerätepaar","Par de dispositivos"),
            tr(l,"Near (kA)","Origen (kA)","Einspeisung (kA)","Origem (kA)"),
            tr(l,"Remote (kA)","Extremo (kA)","Leitungsende (kA)","Extremidade (kA)"),
            tr(l,"Remote clearing (s)","Despeje al extremo (s)","Abschaltung am Ende (s)","Atuação no fim (s)"), outcome.clone()],
            assessments.iter().filter_map(|(c, e)| {
                let inputs = c.code_protection.as_ref()?;
                if inputs.fault_study.is_none() && inputs.coordination.is_none() { return None; }
                let result = e.code_checks.as_ref()?;
                let duty = result.fault_duty.as_ref();
                let pair = result.coordination.as_ref();
                let devices = pair.map_or("–".into(), |q| format!("{} / {}", q.upstream_model, q.downstream_model));
                Some([c.label.clone(), devices, format(duty.map(|d| d.near_ka)), format(duty.map(|d| d.remote_ka)),
                    format(pair.and_then(|q| q.remote_clear_seconds)), status(result.status)])
            }));
        if !fault.blocks.is_empty() { out.push(fault); }

        let mut local = ContentSection::new("local_cable_checks", tr(l,"Local cable checks","Requisitos locales de cables","Lokale Leitungsanforderungen","Requisitos locais de cabos"));
        local.table(vec![circuit_name, rule_set, tr(l,"Path drop (%)","Caída del recorrido (%)","Spannungsfall (%)","Queda no percurso (%)"),
            tr(l,"Limit (%)","Límite (%)","Grenze (%)","Limite (%)"), outcome], assessments.iter().filter_map(|(c, e)| {
                let q = e.local_checks.as_ref()?;
                Some([c.label.clone(), q.reference.clone(), format(q.total_drop_pct), format(q.limit_pct), status(q.status)])
            }));
        if !local.blocks.is_empty() { out.push(local); }
    }
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
        let mut shading = ContentSection::new("field_shading", tr(l, "Shading by field", "Sombreado por campo", "Verschattung je Modulfeld", "Sombreamento por campo"));
        shading.table(vec![tr(l, "Field", "Campo", "Modulfeld", "Campo"),
            tr(l, "Geometric beam loss", "Pérdida directa geométrica", "Geometrischer Direktstrahlungsverlust", "Perda direta geométrica"),
            tr(l, "Sky factor", "Factor de cielo", "Himmelsfaktor", "Fator de céu"),
            tr(l, "Electrical model", "Modelo eléctrico", "Elektrisches Modell", "Modelo elétrico"),
        ], r.module_shading.iter().map(|field| {
            let model = if field.half_cell {
                tr(l, "Parallel half-cells", "Medias células en paralelo", "Parallele Halbzellen", "Meias células em paralelo")
            } else if field.nonlinear {
                tr(l, "Substring I–V curves", "Curvas I–V de subcadenas", "I–V-Kurven der Teilstrings", "Curvas I–V de subcadeias")
            } else {
                tr(l, "Field average", "Promedio del campo", "Modulfeld-Mittelwert", "Média do campo")
            };
            [field.field.clone(), format!("{} %", n(field.beam_area_loss_pct, 2)), format!("{} %", n(field.diffuse_factor * 100.0, 1)), model]
        }));
        out.push(shading);
        let title = tr(l, "Module shading schedule", "Tabla de sombreado por módulo", "Verschattung je Modul", "Tabela de sombreamento por módulo");
        let mut modules = ContentSection::new("module_shading", title);
        modules.text(tr(l,
            "Beam loss is weighted by direct normal irradiance. The column describes blocked beam samples, and sky view describes unblocked sky. Electrical output is calculated for each tracker.",
            "La pérdida directa se pondera por irradiancia directa normal. La tabla muestra el haz bloqueado y la fracción de cielo libre. La producción eléctrica se calcula por seguidor.",
            "Der Direktstrahlungsverlust wird nach Direktnormalstrahlung gewichtet. Die Tabelle zeigt blockierte Direktstrahlung und freien Himmel. Die elektrische Leistung wird je MPP-Tracker berechnet.",
            "A perda direta é ponderada pela irradiância direta normal. A tabela mostra o feixe bloqueado e a fração de céu livre. A potência elétrica é calculada por rastreador."));
        for field in &r.module_shading {
            modules.text(field.field.clone());
            modules.table(vec![
                tr(l, "Module", "Módulo", "Modul", "Módulo"), "String".into(),
                tr(l, "Beam loss", "Pérdida directa", "Direktverlust", "Perda direta"),
                tr(l, "Sky view", "Vista del cielo", "Himmelssicht", "Visão do céu"),
            ], field.modules.iter().map(|m| [
                m.id.clone(), m.string.map(|id| (id + 1).to_string()).unwrap_or_else(|| "—".into()),
                format!("{} %", n(m.beam_area_loss_pct, 2)), format!("{} %", n(m.sky_view * 100.0, 1)),
            ]));
        }
        out.push(modules);
    }
    out
}
