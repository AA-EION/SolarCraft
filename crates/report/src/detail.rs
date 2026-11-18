// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{charts, tr, ContentSection, ReportInput, View};
use solarcraft_engine::calendar::local_time;
use solarcraft_model::{derive::electrical, plant::Coupling};

#[derive(Default)]
struct Month {
    load: f64,
    served: f64,
    import: f64,
    export: f64,
    charge: f64,
    discharge: f64,
    unserved: f64,
    generator: f64,
    temp_sum: f64,
    temp_hours: u32,
    soc: Option<(f64, f64)>,
}

pub(crate) fn results(view: &mut View, input: &ReportInput) {
    let p = input.project;
    let r = input.result;
    let l = input.lang;
    let n = |v, digits| l.num(v, digits);
    let names = crate::i18n::month_abbr(l);
    let ticks: Vec<_> = names.iter().enumerate().map(|(i, name)| (i as u32 + 1, *name)).collect();
    let all_details = view.report_type == crate::ReportType::SystemPlanner;
    let mut months: [Month; 12] = Default::default();
    let wh = |series: &[f32], i: usize| f64::from(series.get(i).copied().unwrap_or_default()) / 1000.0;
    for (i, &hour) in r.local_hour.iter().enumerate() {
        let month = &mut months[local_time(hour).month as usize - 1];
        month.load += wh(&r.flows.load, i);
        month.served += wh(&r.flows.load_served, i);
        month.import += wh(&r.flows.import, i);
        month.export += wh(&r.flows.export, i);
        month.charge += wh(&r.flows.batt_charge, i);
        month.discharge += wh(&r.flows.batt_discharge, i);
        month.unserved += wh(&r.flows.unserved, i);
        month.generator += wh(&r.flows.generator, i);
        if let Some(&c) = r.hourly.t_cell_c.get(i).filter(|_| r.hourly.poa_w_m2.get(i).is_some_and(|v| *v >= 20.0)) {
            month.temp_sum += f64::from(c);
            month.temp_hours += 1;
        }
        if let Some(&soc) = r.flows.soc_pct.get(i).filter(|_| p.storage.is_some()) {
            let soc = f64::from(soc);
            month.soc = Some(month.soc.map_or((soc, soc), |(lo, hi)| (lo.min(soc), hi.max(soc))));
        }
    }
    let islanded = p.workflow.system.islanded();
    let grid_title = if islanded {
        tr(l, "Monthly off-grid supply", "Suministro aislado mensual", "Monatliche Inselversorgung", "Suprimento isolado mensal")
    } else {
        tr(l, "Monthly consumption and grid exchange", "Consumo e intercambio mensual con la red", "Monatsverbrauch und Netzaustausch", "Consumo e intercâmbio mensal com a rede")
    };
    let exchanged = if islanded {
        [tr(l, "Unserved", "No atendido", "Ungedeckt", "Não atendido"), tr(l, "Generator", "Generador", "Generator", "Gerador")]
    } else {
        [tr(l, "Import", "Importación", "Bezug", "Importação"), tr(l, "Export", "Exportación", "Einspeisung", "Exportação")]
    };
    let mut grid = ContentSection::new("monthly_grid", grid_title);
    let mut labels = vec![view.s["month"].into(), tr(l, "Requested", "Solicitado", "Bedarf", "Solicitado"),
        tr(l, "Served", "Atendido", "Versorgt", "Atendido")];
    labels.extend(exchanged);
    if r.balance.load_kwh == 0.0 { labels.drain(1..3); }
    grid.text("kWh");
    grid.table(labels, months.iter().zip(names).map(|(m, name)| {
        let (first, second) = if islanded { (m.unserved, m.generator) } else { (m.import, m.export) };
        let mut row = vec![name.into(), n(m.load, 0), n(m.served, 0), n(first, 0), n(second, 0)];
        if r.balance.load_kwh == 0.0 { row.drain(1..3); }
        row
    }));
    if r.balance.load_kwh > 0.0 {
        let load = months.each_ref().map(|m| m.load);
        grid.figure(charts::monthly_bars(&load, names, "kWh", l), tr(l, "Requested consumption", "Consumo solicitado", "Verbrauchsbedarf", "Consumo solicitado"), view);
    }
    if all_details && r.balance.export_kwh > 0.0 {
        let export = months.each_ref().map(|m| m.export);
        grid.figure(charts::monthly_bars(&export, names, "kWh", l), tr(l, "Export to grid", "Exportación a la red", "Netzeinspeisung", "Exportação para a rede"), view);
    }
    view.contents.push(grid);

    let mut perf = ContentSection::new("monthly_performance", tr(l, "Monthly performance and module temperature", "Rendimiento y temperatura del módulo por mes", "Monatliche Leistung und Modultemperatur", "Desempenho e temperatura dos módulos por mês"));
    perf.table(vec![view.s["month"].into(), "PR %".into(), "kWh/kWp".into(), tr(l, "Mean at irradiance ≥20 W/m²", "Media con irradiancia ≥20 W/m²", "Mittel bei Einstrahlung ≥20 W/m²", "Média com irradiância ≥20 W/m²")],
        r.monthly.iter().filter_map(|r_month| {
            let index = r_month.month.checked_sub(1)? as usize;
            let m = months.get(index)?;
            let potential = r_month.poa_kwh_m2 * r.annual.dc_kwp;
            Some([names[index].into(), if potential > 0.0 { n(100.0 * r_month.ac_kwh / potential, 1) } else { "–".into() },
                n(r_month.ac_kwh / r.annual.dc_kwp, 1),
                if m.temp_hours > 0 { format!("{} °C", n(m.temp_sum / f64::from(m.temp_hours), 1)) } else { "–".into() }])
        }));
    if all_details {
        let pr = r.monthly.iter().filter(|m| m.poa_kwh_m2 > 0.0)
            .map(|m| (m.month, 100.0 * m.ac_kwh / (m.poa_kwh_m2 * r.annual.dc_kwp))).collect();
        let temp = months.iter().enumerate().filter(|(_, m)| m.temp_hours > 0)
            .map(|(i, m)| (i as u32 + 1, m.temp_sum / f64::from(m.temp_hours))).collect();
        perf.figure(charts::time_lines(&[(String::new(), pr)], &ticks, "%", l), tr(l, "Performance ratio", "Índice de rendimiento", "Performance Ratio", "Índice de desempenho"), view);
        perf.figure(charts::time_lines(&[(String::new(), temp)], &ticks, "°C", l), tr(l, "Mean module temperature in illuminated hours", "Temperatura media del módulo con irradiación", "Mittlere Modultemperatur bei Einstrahlung", "Temperatura média do módulo com irradiância"), view);
    }
    view.contents.push(perf);

    if all_details && !r.electrical.is_empty() {
        let mut units = ContentSection::new("monthly_inverters", tr(l, "Monthly net AC energy by inverter", "Energía CA neta mensual por inversor", "Monatliche AC-Nettoenergie je Wechselrichter", "Energia CA líquida mensal por inversor"));
        units.text(tr(l, "Includes night consumption in the simulated inverter energy.", "Incluye el consumo nocturno en la energía simulada del inversor.", "Die simulierte Wechselrichterenergie enthält den Nachtverbrauch.", "Inclui o consumo noturno na energia simulada do inversor."));
        for batch in r.electrical.chunks(2) {
            let mut header = vec![view.s["month"].into()];
            header.extend(batch.iter().map(|unit| format!("{} · kWh", unit.id)));
            units.table(header, names.iter().enumerate().map(|(i, name)| {
                std::iter::once((*name).into()).chain(batch.iter().map(|u| n(u.monthly_net_ac_kwh[i], 1))).collect::<Vec<_>>()
            }));
            let series: Vec<_> = batch.iter().map(|u| (
                u.id.chars().take(26).collect(), u.monthly_net_ac_kwh.iter().enumerate().map(|(i, v)| (i as u32 + 1, *v)).collect()
            )).collect();
            units.figure(charts::time_lines(&series, &ticks, "kWh", l), "", view);
        }
        view.contents.push(units);
    }

    if all_details && !r.array_irradiation.is_empty() {
        let mut radiation = ContentSection::new("array_irradiation", tr(l, "Irradiation by module area", "Irradiación por área de módulos", "Einstrahlung je Modulfläche", "Irradiação por área dos módulos"));
        let incident = tr(l, "Incident", "Incidente", "Einfallend", "Incidente");
        let effective = tr(l, "Effective", "Efectiva", "Wirksam", "Efetiva");
        for array in &r.array_irradiation {
            radiation.text(format!("{} · kWh/m²", array.name));
            radiation.table(vec![view.s["month"].into(), incident.clone(), effective.clone()],
                names.iter().enumerate().map(|(i, name)| [(*name).into(), n(array.poa_kwh_m2[i], 1), n(array.effective_kwh_m2[i], 1)]));
            let indexed = |values: &[f64; 12]| values.iter().enumerate().map(|(i, v)| (i as u32 + 1, *v)).collect();
            radiation.figure(charts::time_lines(&[
                (incident.clone(), indexed(&array.poa_kwh_m2)), (effective.clone(), indexed(&array.effective_kwh_m2))
            ], &ticks, "kWh/m²", l), "", view);
        }
        view.contents.push(radiation);
    }

    if p.storage.is_some() {
        let mut storage = ContentSection::new("monthly_storage", tr(l, "Monthly battery operation", "Operación mensual de batería", "Monatlicher Batteriebetrieb", "Operação mensal da bateria"));
        storage.table(vec![view.s["month"].into(), tr(l, "Charge kWh", "Carga kWh", "Ladung kWh", "Carga kWh"),
            tr(l, "Discharge kWh", "Descarga kWh", "Entladung kWh", "Descarga kWh"), "SOC min–max %".into()],
            months.iter().zip(names).map(|(m, name)| [name.into(), n(m.charge, 1), n(m.discharge, 1),
                m.soc.map_or_else(|| "–".into(), |(lo, hi)| format!("{}–{}", n(lo, 1), n(hi, 1)))]));
        let range = |max: bool| months.iter().enumerate().filter_map(|(i, m)|
            m.soc.map(|(lo, hi)| (i as u32 + 1, if max { hi } else { lo }))).collect();
        let series = [
            (tr(l, "Minimum", "Mínimo", "Minimum", "Mínimo"), range(false)),
            (tr(l, "Maximum", "Máximo", "Maximum", "Máximo"), range(true)),
        ];
        storage.figure(charts::time_lines(&series, &ticks, "%", l),
            tr(l, "Range of battery state of charge", "Rango de estado de carga", "Bereich des Batterieladezustands", "Faixa do estado de carga"), view);
        view.contents.push(storage);
    }
    if r.balance.outage_hours > 0 && !p.workflow.system.islanded() {
        let b = &r.balance;
        let mut outage = ContentSection::new("outage_service", tr(l, "Service during scheduled outages", "Servicio durante cortes programados", "Versorgung bei geplanten Ausfällen", "Serviço durante interrupções programadas"));
        outage.table(vec![], [
            [tr(l, "Outage duration", "Duración de los cortes", "Ausfalldauer", "Duração das interrupções"), format!("{} h", b.outage_hours)],
            [tr(l, "Unserved critical demand", "Demanda crítica sin atender", "Ungedeckter kritischer Bedarf", "Demanda crítica não atendida"), format!("{} kWh", n(b.unserved_kwh, 2))],
            [tr(l, "Noncritical demand disconnected", "Demanda no crítica desconectada", "Abgeschalteter unkritischer Bedarf", "Demanda não crítica desconectada"), format!("{} kWh", n(b.shed_noncritical_kwh, 2))],
            [tr(l, "Generator to customer load", "Generador al consumo del cliente", "Generator für Kundenlast", "Gerador para a carga do cliente"), format!("{} kWh", n(b.generator_to_load_kwh, 2))],
        ]);
        outage.table(vec![view.s["month"].into(), tr(l, "Generator kWh", "Generador kWh", "Generator kWh", "Gerador kWh"),
            tr(l, "Critical unserved kWh", "Crítica no atendida kWh", "Ungedeckt kritisch kWh", "Crítica não atendida kWh")],
            months.iter().zip(names).filter(|(m, _)| m.generator > 0.0 || m.unserved > 0.0)
                .map(|(m, name)| [name.into(), n(m.generator, 2), n(m.unserved, 2)]));
        if b.generator_kwh > 0.0 {
            outage.figure(charts::monthly_bars(&months.each_ref().map(|m| m.generator), names, "kWh", l),
                tr(l, "Generator production during outages", "Producción del generador durante cortes", "Generatorerzeugung bei Ausfällen", "Produção do gerador durante interrupções"), view);
        }
        view.contents.push(outage);
    }

    if let Some(f) = &r.finance {
        let mut bill = ContentSection::new("energy_account", tr(l, "Electricity account in year one", "Cuenta eléctrica del primer año", "Stromabrechnung im ersten Jahr", "Conta elétrica do primeiro ano"));
        let before = &f.bill_without;
        let after = &f.bill_with;
        let cost = |value| n(value, 2);
        let before_label = tr(l, "Before PV", "Sin FV", "Ohne PV", "Sem FV");
        let after_label = tr(l, "With PV", "Con FV", "Mit PV", "Com FV");
        let currency = if f.currency.chars().count() <= 8 { &f.currency } else { "" };
        bill.table(vec![f.currency.clone(), before_label.clone(), after_label.clone()], [
            [tr(l, "Imported energy", "Energía importada", "Bezugsenergie", "Energia importada"), cost(before.energy), cost(after.energy)],
            [tr(l, "Demand charges", "Cargos por demanda", "Leistungspreis", "Encargos de demanda"), cost(before.demand), cost(after.demand)],
            [tr(l, "Fixed charges", "Cargos fijos", "Grundpreis", "Encargos fixos"), cost(before.fixed), cost(after.fixed)],
            [tr(l, "Export credit", "Crédito de exportación", "Einspeisegutschrift", "Crédito de exportação"), cost(before.export_credit), cost(after.export_credit)],
            [view.s["total"].into(), cost(before.total), cost(after.total)],
        ]);
        let comparison = [(String::new(), vec![(0, before.total), (1, after.total)])];
        bill.figure(charts::time_lines(&comparison, &[(0, &before_label), (1, &after_label)], currency, l),
            tr(l, "Total bill comparison", "Comparación de facturas totales", "Vergleich der Gesamtrechnung", "Comparação da conta total"), view);
        view.contents.push(bill);
        if f.years.len() > 1 {
            let mut horizon = ContentSection::new("financial_projection", tr(l, "Long-term energy and financing", "Energía y financiación a largo plazo", "Langfristiger Ertrag und Finanzierung", "Energia e financiamento a longo prazo"));
            horizon.table(vec![tr(l, "Year", "Año", "Jahr", "Ano"), "kWh".into(), tr(l, "Maintenance", "Mantenimiento", "Wartung", "Manutenção"),
                tr(l, "Replacements", "Reposiciones", "Ersatz", "Reposições"), tr(l, "Loan payments", "Pagos de préstamo", "Kreditraten", "Pagamentos do empréstimo")],
                f.years.iter().skip(1).map(|y| [y.year.to_string(), n(y.energy_kwh, 0), cost(y.om), cost(y.replacements), cost(y.loan)]));
            let last = f.years.last().unwrap().year;
            let years = (0..=last).filter(|y| *y % 5 == 0 || *y == last).map(|y| (y, y.to_string())).collect::<Vec<_>>();
            let ticks: Vec<_> = years.iter().map(|(year, text)| (*year, text.as_str())).collect();
            let projections = [(String::new(), f.years.iter().skip(1).map(|y| (y.year, y.energy_kwh)).collect())];
            horizon.figure(charts::time_lines(&projections, &ticks, "kWh", l),
                tr(l, "Projected PV energy", "Energía FV proyectada", "Prognostizierter PV-Ertrag", "Energia FV projetada"), view);
            let inflation = 1.0 + p.tariff.import_escalation_pct / 100.0;
            let baseline = |year: u32| before.total * inflation.powf(f64::from(year.saturating_sub(1)));
            let bills = [
                (before_label, f.years.iter().skip(1).map(|y| (y.year, baseline(y.year))).collect()),
                (after_label, f.years.iter().skip(1).map(|y| (y.year, baseline(y.year) - y.savings - y.export_revenue)).collect()),
            ];
            horizon.figure(charts::time_lines(&bills, &ticks, currency, l),
                tr(l, "Projected electricity bills", "Facturas eléctricas proyectadas", "Prognostizierte Stromrechnungen", "Contas de energia projetadas"), view);
            if before.import_kwh > 0.0 {
                let prices = [(String::new(), f.years.iter().skip(1).map(|y| (y.year,
                    before.energy / before.import_kwh * inflation.powf(f64::from(y.year - 1)))).collect())];
                horizon.figure(charts::time_lines(&prices, &ticks, currency, l),
                    tr(l, "Effective import price per kWh", "Precio efectivo de importación por kWh", "Effektiver Bezugspreis je kWh", "Preço efetivo de importação por kWh"), view);
            }
            if let Some(loan) = &p.economics.loan {
                let mut remaining = loan.amount;
                let data = f.years.iter().map(|y| {
                    if y.year > 0 { remaining = (remaining * (1.0 + loan.rate_pct / 100.0) - y.loan).max(0.0); }
                    (y.year, y.cumulative - remaining)
                }).collect();
                horizon.figure(charts::time_lines(&[(String::new(), data)], &ticks, currency, l),
                    tr(l, "Cumulative cash flow after outstanding debt", "Flujo acumulado menos deuda pendiente", "Kumulierter Cashflow abzüglich Restschuld", "Fluxo acumulado após dívida pendente"), view);
            }
            view.contents.push(horizon);
        }
    }
}

pub(crate) fn planner(view: &mut View, input: &ReportInput) {
    let l = input.lang;
    let p = input.project;
    let b = &input.result.balance;
    let n = |v| l.num(v, 1);
    let source = vec![
        (tr(l, "PV DC", "FV CC", "PV DC", "FV CC"), b.pv_dc_kwh),
        (tr(l, "Grid import", "Importación de red", "Netzbezug", "Importação da rede"), b.import_kwh),
        (tr(l, "Generator", "Generador", "Generator", "Gerador"), b.generator_kwh),
        (tr(l, "Stored energy decrease", "Descenso de energía almacenada", "Abnahme Speicherenergie", "Redução da energia armazenada"), (-b.stored_delta_kwh).max(0.0)),
    ];
    let use_ = vec![
        (tr(l, "Served demand", "Demanda atendida", "Versorgter Bedarf", "Demanda atendida"), b.load_served_kwh),
        (tr(l, "Grid export", "Exportación a la red", "Netzeinspeisung", "Exportação para a rede"), b.export_kwh),
        (tr(l, "Conversion and other losses", "Conversión y otras pérdidas", "Umwandlung und weitere Verluste", "Conversão e outras perdas"),
            b.clipped_kwh + b.curtailed_kwh + b.inverter_loss_kwh + b.ac_wiring_loss_kwh + b.standby_kwh + b.batt_conversion_loss_kwh + b.self_discharge_kwh),
        (tr(l, "Stored energy increase", "Aumento de energía almacenada", "Zunahme Speicherenergie", "Aumento da energia armazenada"), b.stored_delta_kwh.max(0.0)),
    ];
    let mut ledger = ContentSection::new("balance_sankey", tr(l, "Annual energy balance", "Balance energético anual", "Jahresenergiebilanz", "Balanço energético anual"));
    ledger.table(vec![tr(l, "Inputs", "Entradas", "Eingänge", "Entradas"), "kWh".into(), tr(l, "Outputs and losses", "Salidas y pérdidas", "Ausgänge und Verluste", "Saídas e perdas"), "kWh".into()],
        source.iter().zip(&use_).map(|((a, av), (b, bv))| [a.clone(), n(*av), b.clone(), n(*bv)]));
    ledger.figure(charts::balance_flow(&source, &use_, l), "", view);
    view.contents.push(ledger);

    let units = electrical(p);
    if !units.inverters.is_empty() {
        let mut circuit = ContentSection::new("connection_plan", tr(l, "Electrical connection overview", "Esquema general de conexiones", "Elektrische Anschlussübersicht", "Visão geral das conexões elétricas"));
        let grid = tr(l, "AC connection", "Conexión CA", "AC-Anschluss", "Conexão CA");
        circuit.text(format!("{grid} · {} V · {}×", n(p.grid.voltage_v), p.grid.phases));
        for chunk in units.inverters.chunks(7) {
            let rows = chunk.iter().map(|u| {
                let modules: u64 = u.mppts.iter().flatten().map(|g| u64::from(g.strings) * u64::from(g.modules_per_string)).sum();
                (format!("{modules} × {}", view.s["module"]), u.id.clone())
            }).collect::<Vec<_>>();
            circuit.figure(charts::connections(&rows, &grid), "", view);
        }
        view.contents.push(circuit);
    }
    if let Some(storage) = &p.storage {
        let mut hardware = ContentSection::new("storage_datasheets", tr(l, "Storage and backup equipment specifications", "Especificaciones de almacenamiento y respaldo", "Daten für Speicher- und Ersatzstromausrüstung", "Especificações de armazenamento e reserva"));
        if let Some(battery) = p.library.battery(&storage.battery_id) {
            hardware.text(format!("{} {}", battery.manufacturer, battery.model));
            hardware.table(vec![], [
                ["V DC".into(), n(battery.nominal_voltage_v)], ["kWh".into(), n(battery.capacity_kwh)],
                [tr(l, "Charge / discharge limit", "Límite de carga y descarga", "Lade- und Entladegrenze", "Limite de carga e descarga"),
                    format!("{} / {} kW", n(battery.p_charge_max_kw), n(battery.p_discharge_max_kw))],
                ["SOC min–max".into(), format!("{}–{} %", n(battery.soc_min_pct), n(battery.soc_max_pct))],
            ]);
            if let Some(cells) = &battery.cells {
                hardware.table(vec![], [["V/cell".into(), n(cells.cell_voltage_v)], ["S × P × strings".into(), format!("{} × {} × {}", cells.series, cells.parallel, cells.strings)],
                    ["Ω".into(), l.num(cells.pack_resistance(), 4)]]);
            }
            if let (Coupling::Integrated, Some(c)) = (&storage.coupling, &battery.converter) {
                hardware.table(vec![], [[tr(l, "Converter charge / discharge", "Carga y descarga del convertidor", "Wandler Laden und Entladen", "Carga e descarga do conversor"),
                    format!("{} / {} W", n(c.p_charge_max_w), n(c.p_discharge_max_w))]]);
            }
        }
        if let Coupling::AcCoupled { battery_inverter_id } = &storage.coupling {
            if let Some(i) = p.library.battery_inverter(battery_inverter_id) {
                hardware.text(format!("{} {}", i.manufacturer, i.model));
                hardware.table(vec![], [["W AC".into(), n(i.p_ac_max_w)], ["W standby".into(), n(i.standby_w)]]);
            }
        }
        if let Some(g) = storage.generator_id.as_ref().and_then(|id| p.library.generator(id)) {
            hardware.text(format!("{} {}", g.manufacturer, g.model));
            hardware.table(vec![], [["VA".into(), n(g.rated_va)], ["VA max".into(), n(g.max_va)], ["V AC".into(), n(g.voltage_v)]]);
        }
        if !hardware.blocks.is_empty() { view.contents.push(hardware); }
    }
}
