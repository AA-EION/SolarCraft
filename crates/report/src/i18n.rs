// SPDX-License-Identifier: GPL-3.0-or-later
//! Report strings and number formatting for the launch locales (D-05).
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang { En, Es, De, PtBr }

impl Lang {
    pub fn parse(s: &str) -> Option<Lang> {
        match s.to_ascii_lowercase().as_str() { "en" => Some(Lang::En), "es" | "es-co" => Some(Lang::Es), "de" => Some(Lang::De), "pt" | "pt-br" => Some(Lang::PtBr), _ => None }
    }
    pub fn tag(self) -> &'static str { match self { Lang::En => "en", Lang::Es => "es", Lang::De => "de", Lang::PtBr => "pt" } }
    fn idx(self) -> usize { self as usize }
    /// Decimal and thousands separators.
    pub fn separators(self) -> (char, Option<char>) {
        match self { Lang::En => ('.', Some(',')), Lang::Es => (',', Some('.')), Lang::De => (',', Some('.')), Lang::PtBr => (',', Some('.')) }
    }
    /// Format a number with `decimals` places using this locale.
    pub fn num(self, v: f64, decimals: usize) -> String {
        if !v.is_finite() { return "–".into(); }
        let (dec, thou) = self.separators();
        let s = format!("{:.*}", decimals, v.abs());
        let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((s.clone(), None));
        let mut grouped = String::new();
        // Spanish style guides omit the group separator for 4-digit numbers; keep it uniform for tables.
        for (i, ch) in int.chars().enumerate() {
            if i > 0 && (int.len() - i) % 3 == 0 { if let Some(t) = thou { grouped.push(t); } }
            grouped.push(ch);
        }
        let neg = if v < 0.0 && s.chars().any(|c| c != '0' && c != '.') { "−" } else { "" };
        match frac { Some(f) => format!("{neg}{grouped}{dec}{f}"), None => format!("{neg}{grouped}") }
    }
}

macro_rules! strings {
    ($($key:ident => [$en:expr, $es:expr, $de:expr, $pt:expr]),* $(,)?) => {
        pub fn table(lang: Lang) -> HashMap<&'static str, &'static str> {
            let mut m = HashMap::new();
            $( m.insert(stringify!($key), [$en, $es, $de, $pt][lang.idx()]); )*
            m
        }
    };
}

strings! {
    title => ["Photovoltaic system proposal", "Propuesta de sistema fotovoltaico", "Angebot Photovoltaikanlage", "Proposta de sistema fotovoltaico"],
    prepared_for => ["Prepared for", "Preparado para", "Erstellt für", "Preparado para"],
    date => ["Date", "Fecha", "Datum", "Data"],
    project => ["Project", "Proyecto", "Projekt", "Projeto"],
    kpi_size => ["System size", "Potencia instalada", "Anlagenleistung", "Potência instalada"],
    kpi_energy => ["Annual energy", "Energía anual", "Jahresertrag", "Energia anual"],
    kpi_yield => ["Specific yield", "Rendimiento específico", "Spezifischer Ertrag", "Produtividade específica"],
    kpi_pr => ["Performance ratio", "Índice de rendimiento (PR)", "Performance Ratio", "Taxa de desempenho (PR)"],
    monthly_chart => ["Monthly energy to grid", "Energía mensual a la red", "Monatliche Netzeinspeisung", "Energia mensal para a rede"],
    system => ["System", "Sistema", "Anlage", "Sistema"],
    array => ["Array", "Campo", "Generatorfeld", "Arranjo"],
    module => ["Module", "Módulo", "Modul", "Módulo"],
    modules => ["Modules", "Módulos", "Module", "Módulos"],
    strings => ["Strings × modules per string", "Cadenas × módulos por cadena", "Strings × Module pro String", "Strings × módulos por string"],
    tilt => ["Tilt", "Inclinación", "Neigung", "Inclinação"],
    azimuth => ["Azimuth (from north)", "Azimut (desde el norte)", "Azimut (ab Nord)", "Azimute (a partir do norte)"],
    mounting => ["Mounting", "Montaje", "Montageart", "Montagem"],
    inverter => ["Inverter", "Inversor", "Wechselrichter", "Inversor"],
    dc_ac => ["DC/AC ratio", "Relación CC/CA", "DC/AC-Verhältnis", "Relação CC/CA"],
    site_weather => ["Site and weather data", "Sitio y datos meteorológicos", "Standort und Wetterdaten", "Local e dados meteorológicos"],
    location => ["Location", "Ubicación", "Standort", "Localização"],
    elevation => ["Elevation", "Altitud", "Höhe", "Altitude"],
    weather_source => ["Weather source", "Fuente meteorológica", "Wetterdatenquelle", "Fonte meteorológica"],
    ghi => ["Global horizontal irradiation", "Irradiación global horizontal", "Globalstrahlung horizontal", "Irradiação global horizontal"],
    poa => ["Irradiation in the module plane", "Irradiación en el plano de los módulos", "Einstrahlung in Modulebene", "Irradiação no plano dos módulos"],
    losses => ["Energy losses", "Pérdidas de energía", "Energieverluste", "Perdas de energia"],
    losses_intro => ["From the nominal energy at standard test conditions to the energy delivered to the grid. Each step is computed hourly; the steps add up exactly.", "Desde la energía nominal en condiciones estándar hasta la energía entregada a la red. Cada paso se calcula hora a hora; los pasos suman exactamente.", "Von der Nennenergie bei Standard-Testbedingungen bis zur ins Netz eingespeisten Energie. Jeder Schritt wird stündlich berechnet; die Schritte summieren sich exakt.", "Da energia nominal nas condições padrão até a energia entregue à rede. Cada etapa é calculada hora a hora; as etapas somam exatamente."],
    step => ["Step", "Paso", "Schritt", "Etapa"],
    energy => ["Energy", "Energía", "Energie", "Energia"],
    change => ["Change", "Variación", "Änderung", "Variação"],
    month => ["Month", "Mes", "Monat", "Mês"],
    monthly_table => ["Monthly results", "Resultados mensuales", "Monatliche Ergebnisse", "Resultados mensais"],
    assumptions => ["Assumptions and methods", "Supuestos y métodos", "Annahmen und Methoden", "Premissas e métodos"],
    soiling => ["Soiling", "Suciedad", "Verschmutzung", "Sujidade"],
    lid => ["Light-induced degradation", "Degradación inducida por luz", "Lichtinduzierte Degradation", "Degradação induzida pela luz"],
    mismatch => ["Mismatch", "Desajuste", "Mismatch", "Descasamento"],
    dc_wiring => ["DC wiring", "Cableado CC", "DC-Verkabelung", "Cabeamento CC"],
    ac_wiring => ["AC wiring", "Cableado CA", "AC-Verkabelung", "Cabeamento CA"],
    availability => ["Availability losses", "Pérdidas por disponibilidad", "Verfügbarkeitsverluste", "Perdas por disponibilidade"],
    models => ["Models", "Modelos", "Modelle", "Modelos"],
    models_text => ["Solar position: NREL SPA. Sky diffuse: %MODEL%. Reflection: physical incidence-angle model. Module: CEC six-parameter single-diode model. Temperature: Faiman. Inverter: PVWatts efficiency curve with power limit.", "Posición solar: NREL SPA. Difusa del cielo: %MODEL%. Reflexión: modelo físico de ángulo de incidencia. Módulo: modelo de un diodo de seis parámetros CEC. Temperatura: Faiman. Inversor: curva de eficiencia PVWatts con límite de potencia.", "Sonnenstand: NREL SPA. Diffusstrahlung: %MODEL%. Reflexion: physikalisches Einfallswinkelmodell. Modul: CEC-Eindiodenmodell mit sechs Parametern. Temperatur: Faiman. Wechselrichter: PVWatts-Wirkungsgradkurve mit Leistungsbegrenzung.", "Posição solar: NREL SPA. Difusa do céu: %MODEL%. Reflexão: modelo físico de ângulo de incidência. Módulo: modelo de diodo único CEC de seis parâmetros. Temperatura: Faiman. Inversor: curva de eficiência PVWatts com limite de potência."],
    notes => ["Notes", "Notas", "Hinweise", "Observações"],
    disclaimer => ["This is a simulation estimate based on a typical meteorological year. Actual production varies with weather, shading, equipment and operation. It is not a guarantee of yield.", "Esta es una estimación por simulación basada en un año meteorológico típico. La producción real varía según el clima, las sombras, los equipos y la operación. No constituye una garantía de rendimiento.", "Dies ist eine Simulationsschätzung auf Basis eines typischen meteorologischen Jahres. Der tatsächliche Ertrag hängt von Wetter, Verschattung, Komponenten und Betrieb ab. Er stellt keine Ertragsgarantie dar.", "Esta é uma estimativa por simulação baseada em um ano meteorológico típico. A produção real varia com o clima, o sombreamento, os equipamentos e a operação. Não é uma garantia de produção."],
    run_info => ["Calculation record", "Registro de cálculo", "Berechnungsprotokoll", "Registro de cálculo"],
    page => ["Page", "Página", "Seite", "Página"],
    generated_with => ["Generated with SolarCraft", "Generado con SolarCraft", "Erstellt mit SolarCraft", "Gerado com SolarCraft"],
    open_rack => ["Open rack", "Estructura abierta", "Freistehend", "Estrutura aberta"],
    roof_mounted => ["Roof-mounted (ventilated)", "Sobre cubierta (ventilado)", "Aufdach (hinterlüftet)", "Sobre telhado (ventilado)"],
    integrated => ["Building-integrated", "Integrado en edificio", "Gebäudeintegriert", "Integrado à edificação"],
    total => ["Total", "Total", "Summe", "Total"],
    irradiation => ["Irradiation in the module plane", "Irradiación en el plano de los módulos", "Einstrahlung in Modulebene", "Irradiação no plano dos módulos"],
    irr_ghi => ["Global horizontal irradiation", "Irradiación global horizontal", "Globalstrahlung horizontal", "Irradiação global horizontal"],
    irr_poa => ["Global incident in plane", "Global incidente en el plano", "Globalstrahlung in Modulebene", "Global incidente no plano"],
    irr_shading => ["Near shading (obstacles)", "Sombreado cercano (obstáculos)", "Nahverschattung (Hindernisse)", "Sombreamento próximo (obstáculos)"],
    irr_iam => ["Reflection (incidence angle)", "Reflexión (ángulo de incidencia)", "Reflexion (Einfallswinkel)", "Reflexão (ângulo de incidência)"],
    irr_soiling => ["Soiling", "Suciedad", "Verschmutzung", "Sujidade"],
    irr_effective => ["Effective irradiation on modules", "Irradiación efectiva en los módulos", "Effektive Einstrahlung auf Module", "Irradiação efetiva nos módulos"],
    roof_plan => ["Roof layout", "Distribución en el techo", "Dachbelegung", "Distribuição no telhado"],
    roof_caption => ["%N% modules · eave at the bottom · setback %S% m", "%N% módulos · alero abajo · retiro %S% m", "%N% Module · Traufe unten · Randabstand %S% m", "%N% módulos · beiral embaixo · recuo %S% m"],
    note_horizon => ["The weather data already include shading by the distant horizon (terrain).", "Los datos meteorológicos ya incluyen el sombreado del horizonte lejano (relieve).", "Die Wetterdaten enthalten bereits die Verschattung durch den fernen Horizont (Gelände).", "Os dados meteorológicos já incluem o sombreamento do horizonte distante (relevo)."],
    note_shading => ["%S%: shading by roof obstacles is ray-traced hourly (beam) and by sky view (diffuse); the electrical effect of partial shading on module strings is not yet modelled.", "%S%: el sombreado de los obstáculos del techo se calcula hora a hora por trazado de rayos (directa) y por vista de cielo (difusa); aún no se modela el efecto eléctrico del sombreado parcial en las cadenas.", "%S%: Die Verschattung durch Dachhindernisse wird stündlich per Raytracing (direkt) und über den Himmelssichtfaktor (diffus) berechnet; die elektrische Wirkung von Teilverschattung auf Strings ist noch nicht modelliert.", "%S%: o sombreamento dos obstáculos do telhado é calculado hora a hora por traçado de raios (direta) e por visão do céu (difusa); o efeito elétrico do sombreamento parcial nas strings ainda não é modelado."],
    note_mpp => ["%S%: %N% hours with the string voltage outside the inverter MPP range.", "%S%: %N% horas con la tensión de cadena fuera del rango MPP del inversor.", "%S%: %N% Stunden mit Stringspannung außerhalb des MPP-Bereichs des Wechselrichters.", "%S%: %N% horas com a tensão de string fora da faixa MPP do inversor."],
    weather_tmy => ["Typical meteorological year", "Año meteorológico típico", "Typisches meteorologisches Jahr", "Ano meteorológico típico"],
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_numbers() {
        assert_eq!(Lang::En.num(12345.678, 1), "12,345.7");
        assert_eq!(Lang::Es.num(12345.678, 1), "12.345,7");
        assert_eq!(Lang::De.num(-0.5, 2), "−0,50");
        assert_eq!(Lang::PtBr.num(999.0, 0), "999");
    }
}

pub fn month_abbr(lang: Lang) -> [&'static str; 12] {
    match lang {
        Lang::En => ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
        Lang::Es => ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"],
        Lang::De => ["Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"],
        Lang::PtBr => ["jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez"],
    }
}
