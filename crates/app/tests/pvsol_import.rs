// SPDX-License-Identifier: GPL-3.0-or-later
use serde_json::{Value, json};
use solarcraft_app as app;
use std::{fs::{self, File}, io::Write, path::Path};
use zip::{ZipWriter, write::SimpleFileOptions};

fn call(method: &str, args: Value) -> Value {
    app::api::dispatch(method, &args, &mut |_| true).unwrap()
}

fn archive(path: &Path) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name, data: &[u8]| {
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    };
    let xml = br#"<PVData>
      <Versionsnummer>15</Versionsnummer><AnlagenTyp><SystemType>EnergySoftware.PVSOL.OnGridPVAdvanced</SystemType></AnlagenTyp>
      <Meteo><TranspositionModelType><Value>HayDavies</Value></TranspositionModelType></Meteo>
      <PVModule><Modulfelder><ModulfeldID>area</ModulfeldID><Modulname>South roof</Modulname>
        <ModulID>m</ModulID><ModulVersion><Value>2</Value></ModulVersion><Modulanzahl><Value>2</Value></Modulanzahl>
        <Modulneigung><Value>28</Value></Modulneigung><Modulausrichtung><Value>180</Value></Modulausrichtung>
      </Modulfelder><Modulfelder><ModulfeldID>spare</ModulfeldID><Modulname>Unassigned roof</Modulname>
        <ModulID>m</ModulID><ModulVersion>2</ModulVersion><Modulanzahl>7</Modulanzahl>
        <Modulneigung>35</Modulneigung><Modulausrichtung>270</Modulausrichtung>
      </Modulfelder></PVModule>
      <Wechselrichter><Verschaltung><WRObj><InverterID>i</InverterID><InverterVersion>3</InverterVersion>
        <Haeufigkeit>1</Haeufigkeit><MPPListe><ModulfeldTeil><PolyStringVerschaltung><Straenge>
          <ModulfeldId>area</ModulfeldId><AnzahlInReihe>7</AnzahlInReihe><AnzahlStrings>1</AnzahlStrings>
        </Straenge></PolyStringVerschaltung></ModulfeldTeil><MPPListe>0</MPPListe></MPPListe>
      </WRObj></Verschaltung><Verschaltung><WRObj><InverterID>i</InverterID><InverterVersion>3</InverterVersion>
        <Haeufigkeit>1</Haeufigkeit><MPPListe><ModulfeldTeil><PolyStringVerschaltung><StraengeInReihe>true</StraengeInReihe>
          <Straenge><ModulfeldId>area</ModulfeldId><AnzahlInReihe>3</AnzahlInReihe><AnzahlStrings>1</AnzahlStrings></Straenge>
          <Straenge><ModulfeldId>area</ModulfeldId><AnzahlInReihe>4</AnzahlInReihe><AnzahlStrings>1</AnzahlStrings></Straenge>
        </PolyStringVerschaltung></ModulfeldTeil><MPPListe>0</MPPListe></MPPListe>
      </WRObj></Verschaltung></Wechselrichter>
    </PVData>"#;
    add("Project.xml", xml);
    let db = json!({
        "PVModules":[{"Id":"m","Version":2,"Name":"Synthetic module","Company":{"Name":"Fixture"},
            "CellCount":60,"CellType":0,"Height":1700,"Width":1050,"OpenCircuitVoltage":39,
            "ShortCircuitCurrent":10,"MPP_Voltage":32,"MPP_Current":9.4,"CurrentCoefficient":5,
            "VoltageCoefficient":-120,"OutputpowerCoefficient":-0.004,"SpecifiedOutputPower":300}],
        "PVInverters":[{"Id":"i","Version":3,"Name":"Synthetic inverter","Company":{"Name":"Fixture"},
            "MPPT1_NrOf":1,"MPPT1_U_MPP_Min":100,"MPPT1_U_MPP_Max":500,"MPPT1_MaxCurrent":18,
            "MPPT1_MaxShortCircuitCurrent":25,"MaxInputVoltageDC":600,"MaxAC_Power":2800,"E50":0.96}]
    });
    add("DB.json", &serde_json::to_vec(&db).unwrap());
    let mut weather = vec![0; 2517 + 8760 * 12];
    weather[512..520].copy_from_slice(b"\x07T;G;WGS");
    weather[768..772].copy_from_slice(&1_i32.to_le_bytes());
    weather[772..776].copy_from_slice(&8760_i32.to_le_bytes());
    for (offset, number) in [(781, 4.7_f64), (789, 74.1), (866, 100.0), (948, 5.0)] {
        weather[offset..offset + 8].copy_from_slice(&number.to_le_bytes());
    }
    for (i, row) in weather[2517..].chunks_exact_mut(12).enumerate() {
        let irradiance: f32 = if (9..15).contains(&(i % 24)) { 500.0 } else { 0.0 };
        row[0..4].copy_from_slice(&20_f32.to_le_bytes());
        row[4..8].copy_from_slice(&irradiance.to_le_bytes());
        row[8..12].copy_from_slice(&1_f32.to_le_bytes());
    }
    add("sample.wbv", &weather);
    let months = "<MonthlyValues><value>3600000</value></MonthlyValues>".repeat(12);
    let saved = format!("<SimResultsXML><ModelString>synthetic</ModelString><EnergiebilanzResultsWr>
        <EB_WR5_PVEnergieAC><YearlyValue>43200000</YearlyValue>{months}</EB_WR5_PVEnergieAC>
        </EnergiebilanzResultsWr></SimResultsXML>");
    add("SimResults.xml", saved.as_bytes());
    let mut rows = "time\tEnergieBilanz.WRAcEnergie\tEnergieBilanz.PVErzeugteEnergie\n\tWs\tWs\n".to_owned();
    for i in 1..=8760 { rows.push_str(&format!("{}\t1,25\t2,5\n", i * 3600)); }
    add("core.results.3600", rows.as_bytes());
    zip.finish().unwrap();
}

#[test]
fn project_import_creates_an_editable_copy_and_compares_a_separate_reference() {
    let dir = std::env::temp_dir().join(format!("SolarCraft-PVPRJ-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.pvprj");
    let destination = dir.join("copy.solarcraft");
    archive(&source);
    let before = fs::read(&source).unwrap();
    let preview = call("import_pvsol", json!({"source": source}));
    assert_eq!(preview["reference"]["installed_modules"], 14);
    assert_eq!(preview["reference"]["inverters"], 2);
    assert_eq!(preview["reference"]["channels"]["inverter_ac"]["annual_kwh"], 12.0);
    assert!(!destination.exists());
    call("import_pvsol", json!({"source": source,"file": destination}));
    let reopened = app::get_project(&destination).unwrap();
    assert_eq!(reopened.site.longitude.value, -74.1);
    assert_eq!(reopened.site.utc_offset(), -5.0);
    assert_eq!(reopened.library.modules[0].catalog_ref.as_ref().unwrap().version, 2);
    assert_eq!(reopened.arrays.len(), 2);
    assert_eq!(reopened.arrays[1].strings.value, 0);
    assert_eq!(reopened.electrical.inverters[0].mppts[0][0].modules_per_string, 7);
    let series = &reopened.electrical.inverters[1].mppts[0];
    assert_eq!(series.iter().map(|g| g.modules_per_string).collect::<Vec<_>>(), [3, 4]);
    assert!(series[0].series_chain.is_some() && series[0].series_chain == series[1].series_chain);
    assert!(app::pvsol_import::import(&source, Some(&destination)).is_err());
    assert!(call("compare_pvsol",json!({"file":destination}))["comparison"].is_null());
    app::simulate(&destination, |_| true).unwrap();
    let result = call("compare_pvsol",json!({"file":destination}));
    assert_eq!(result["imported_input_hash_matches"], true);
    assert_eq!(result["comparison"]["inverter_ac"]["monthly"].as_array().unwrap().len(), 12);
    assert_eq!(result["comparison"]["pv_dc"]["hourly"]["rows"], 8760);
    app::set_fields(&destination, &[("/arrays/0/tilt".into(), json!(31))]).unwrap();
    assert!(call("compare_pvsol",json!({"file":destination}))["comparison"].is_null());
    assert_eq!(fs::read(&source).unwrap(), before);
    for file in [source, destination] { fs::remove_file(file).unwrap(); }
    fs::remove_dir(dir).unwrap();
}
