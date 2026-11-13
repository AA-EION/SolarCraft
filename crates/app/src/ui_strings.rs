// SPDX-License-Identifier: GPL-3.0-or-later
//! UI strings for the native shells (single source for Windows and macOS).
use std::collections::BTreeMap;

macro_rules! ui {
    ($($k:ident => [$en:expr, $es:expr, $de:expr, $pt:expr]),* $(,)?) => {
        pub fn strings(lang: &str) -> BTreeMap<&'static str, &'static str> {
            let i = match lang.to_ascii_lowercase().get(..2) { Some("es") => 1, Some("de") => 2, Some("pt") => 3, _ => 0 };
            let mut m = BTreeMap::new();
            $( m.insert(stringify!($k), [$en, $es, $de, $pt][i]); )*
            m
        }
    };
}

ui! {
    app_title => ["SolarCraft", "SolarCraft", "SolarCraft", "SolarCraft"],
    new_project => ["New project…", "Nuevo proyecto…", "Neues Projekt…", "Novo projeto…"],
    open_project => ["Open project…", "Abrir proyecto…", "Projekt öffnen…", "Abrir projeto…"],
    undo => ["Undo", "Deshacer", "Rückgängig", "Desfazer"],
    no_project => ["Create or open a project to start.", "Cree o abra un proyecto para comenzar.", "Erstellen oder öffnen Sie ein Projekt.", "Crie ou abra um projeto para começar."],
    project => ["Project", "Proyecto", "Projekt", "Projeto"],
    site_weather => ["Site & weather", "Sitio y clima", "Standort & Wetter", "Local e clima"],
    design => ["Design", "Diseño", "Auslegung", "Projeto elétrico"],
    results => ["Results", "Resultados", "Ergebnisse", "Resultados"],
    report => ["Report", "Informe", "Bericht", "Relatório"],
    name => ["Project name", "Nombre del proyecto", "Projektname", "Nome do projeto"],
    customer => ["Customer", "Cliente", "Kunde", "Cliente"],
    address => ["Address", "Dirección", "Adresse", "Endereço"],
    latitude => ["Latitude (°N)", "Latitud (°N)", "Breite (°N)", "Latitude (°N)"],
    longitude => ["Longitude (°E)", "Longitud (°E)", "Länge (°O)", "Longitude (°L)"],
    elevation => ["Elevation (m)", "Altitud (m)", "Höhe (m)", "Altitude (m)"],
    albedo => ["Ground albedo", "Albedo del suelo", "Bodenalbedo", "Albedo do solo"],
    weather => ["Weather data", "Datos meteorológicos", "Wetterdaten", "Dados meteorológicos"],
    no_weather => ["No weather data yet.", "Aún no hay datos meteorológicos.", "Noch keine Wetterdaten.", "Ainda sem dados meteorológicos."],
    fetch_pvgis => ["Download from PVGIS", "Descargar de PVGIS", "Von PVGIS laden", "Baixar do PVGIS"],
    fetch_consent => ["This sends the site latitude and longitude to the PVGIS service of the European Commission (JRC). Continue?", "Esto envía la latitud y longitud del sitio al servicio PVGIS de la Comisión Europea (JRC). ¿Continuar?", "Dabei werden Breite und Länge des Standorts an den PVGIS-Dienst der Europäischen Kommission (JRC) gesendet. Fortfahren?", "Isto envia a latitude e longitude do local ao serviço PVGIS da Comissão Europeia (JRC). Continuar?"],
    import_weather => ["Import EPW / PVGIS file…", "Importar archivo EPW / PVGIS…", "EPW-/PVGIS-Datei importieren…", "Importar arquivo EPW / PVGIS…"],
    demo_weather => ["Use offline demo weather", "Usar clima de demostración sin conexión", "Offline-Demowetter verwenden", "Usar clima de demonstração offline"],
    demo_warning => ["Clear-sky demo data overestimate yield. Use only to try the workflow.", "Los datos de cielo despejado sobrestiman la producción. Úselos solo para probar el flujo.", "Klarhimmel-Demodaten überschätzen den Ertrag. Nur zum Ausprobieren.", "Dados de céu limpo superestimam a produção. Use apenas para testar o fluxo."],
    mode => ["Mode", "Modo", "Modus", "Modo"],
    easy => ["Easy", "Sencillo", "Einfach", "Simples"],
    detailed => ["Detailed", "Detallado", "Detailliert", "Detalhado"],
    mode_hint => ["Switching modes never deletes your values.", "Cambiar de modo nunca borra sus valores.", "Ein Moduswechsel löscht niemals Ihre Werte.", "Trocar de modo nunca apaga seus valores."],
    target_kwp => ["System size (kWp)", "Potencia del sistema (kWp)", "Anlagenleistung (kWp)", "Potência do sistema (kWp)"],
    tilt => ["Tilt (°)", "Inclinación (°)", "Neigung (°)", "Inclinação (°)"],
    azimuth => ["Azimuth (° from north)", "Azimut (° desde el norte)", "Azimut (° ab Nord)", "Azimute (° a partir do norte)"],
    mounting => ["Mounting", "Montaje", "Montageart", "Montagem"],
    open_rack => ["Open rack", "Estructura abierta", "Freistehend", "Estrutura aberta"],
    roof_mounted => ["Roof-mounted", "Sobre cubierta", "Aufdach", "Sobre telhado"],
    integrated => ["Building-integrated", "Integrado", "Gebäudeintegriert", "Integrado"],
    modules_per_string => ["Modules per string", "Módulos por cadena", "Module pro String", "Módulos por string"],
    strings => ["Strings", "Cadenas", "Strings", "Strings"],
    module => ["Module", "Módulo", "Modul", "Módulo"],
    inverter => ["Inverter", "Inversor", "Wechselrichter", "Inversor"],
    derived => ["calculated", "calculado", "berechnet", "calculado"],
    issues => ["Checks", "Verificaciones", "Prüfungen", "Verificações"],
    no_issues => ["No issues found.", "Sin observaciones.", "Keine Probleme gefunden.", "Nenhum problema encontrado."],
    simulate => ["Simulate", "Simular", "Simulieren", "Simular"],
    cancel => ["Cancel", "Cancelar", "Abbrechen", "Cancelar"],
    system_size => ["System size", "Potencia instalada", "Anlagenleistung", "Potência instalada"],
    annual_energy => ["Annual energy", "Energía anual", "Jahresertrag", "Energia anual"],
    specific_yield => ["Specific yield", "Rendimiento específico", "Spezifischer Ertrag", "Produtividade específica"],
    performance_ratio => ["Performance ratio", "Índice de rendimiento", "Performance Ratio", "Taxa de desempenho"],
    monthly_energy => ["Monthly energy to grid (kWh)", "Energía mensual a la red (kWh)", "Monatliche Netzeinspeisung (kWh)", "Energia mensal para a rede (kWh)"],
    losses => ["Energy losses", "Pérdidas de energía", "Energieverluste", "Perdas de energia"],
    no_results => ["Run the simulation to see results.", "Ejecute la simulación para ver resultados.", "Simulation starten, um Ergebnisse zu sehen.", "Execute a simulação para ver os resultados."],
    language => ["Language", "Idioma", "Sprache", "Idioma"],
    format => ["Format", "Formato", "Format", "Formato"],
    designer => ["Designer / company", "Diseñador / empresa", "Planer / Firma", "Projetista / empresa"],
    export_report => ["Export report…", "Exportar informe…", "Bericht exportieren…", "Exportar relatório…"],
    report_saved => ["Report saved", "Informe guardado", "Bericht gespeichert", "Relatório salvo"],
    error => ["Error", "Error", "Fehler", "Erro"],
}
