// SPDX-License-Identifier: GPL-3.0-or-later
use serde_json::{Value, json};
use solarcraft_app as app;
use std::{fs::{self, File}, io::Write, path::Path};
use zip::{ZipWriter, write::SimpleFileOptions};

fn call(method: &str, args: Value) -> Value {
    app::api::dispatch(method, &args, &mut |_| true).unwrap()
}

/// Use the saved DC generator flag for both catalog topologies to cover the stale AC flag.
fn archive_with(path: &Path, storage_enabled: bool, catalog_coupling: Option<i32>) {
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
        </Straenge><Straenge>
          <ModulfeldId>00000000-0000-0000-0000-000000000000</ModulfeldId><AnzahlInReihe>0</AnzahlInReihe><AnzahlStrings>0</AnzahlStrings>
        </Straenge></PolyStringVerschaltung></ModulfeldTeil><MPPListe>0</MPPListe></MPPListe>
      </WRObj></Verschaltung><Verschaltung><WRObj><InverterID>i</InverterID><InverterVersion>3</InverterVersion>
        <Haeufigkeit>1</Haeufigkeit><MPPListe><ModulfeldTeil><PolyStringVerschaltung><StraengeInReihe>true</StraengeInReihe>
          <Straenge><ModulfeldId>area</ModulfeldId><AnzahlInReihe>3</AnzahlInReihe><AnzahlStrings>1</AnzahlStrings></Straenge>
          <Straenge><ModulfeldId>area</ModulfeldId><AnzahlInReihe>4</AnzahlInReihe><AnzahlStrings>1</AnzahlStrings></Straenge>
        </PolyStringVerschaltung></ModulfeldTeil><MPPListe>0</MPPListe></MPPListe>
      </WRObj></Verschaltung></Wechselrichter>
    </PVData>"#;
    let mut xml = String::from_utf8(xml.to_vec()).unwrap();
    if storage_enabled {
        xml = xml.replace("OnGridPVAdvanced", "OnGridPVWithBatteries");
        xml = xml.replace("</PVData>", r#"<ACNetz><NetzSpannung><Value>120</Value></NetzSpannung>
          <IstProzentualeAbregelung>true</IstProzentualeAbregelung><AbregelungAmWR>false</AbregelungAmWR>
          <ProzentualeAbregelung><Value>0.025</Value></ProzentualeAbregelung>
          <IsClippingRelativeToInstalledDCPower>false</IsClippingRelativeToInstalledDCPower></ACNetz></PVData>"#);
    }
    // PV-only projects also save this template, with no battery record in DB.json.
    xml = xml.replace("</PVData>", r#"<Batteriesystem><BatSys>
          <AnzahlBatterieSysteme>1</AnzahlBatterieSysteme><ConnectedWrOrMppIndex>0</ConnectedWrOrMppIndex>
          <BatteryDBId>bs</BatteryDBId><IsAcKopplung>false</IsAcKopplung><IsDcKopplungGenerator>true</IsDcKopplungGenerator>
          <IsDcKopplungZwischenkreis>false</IsDcKopplungZwischenkreis><TimedChargeEnabled>false</TimedChargeEnabled>
          <TimedDischargeEnabled>false</TimedDischargeEnabled></BatSys></Batteriesystem></PVData>"#);
    add("Project.xml", xml.as_bytes());
    let mut db = json!({
        "PVModules":[{"Id":"m","Version":2,"Name":"Synthetic module","Company":{"Name":"Fixture"},
            "CellCount":60,"CellType":0,"Height":1700,"Width":1050,"OpenCircuitVoltage":39,
            "ShortCircuitCurrent":10,"MPP_Voltage":32,"MPP_Current":9.4,"CurrentCoefficient":5,
            "VoltageCoefficient":-120,"OutputpowerCoefficient":-0.004,"SpecifiedOutputPower":300}],
        "PVInverters":[{"Id":"i","Version":3,"Name":"Synthetic inverter","Company":{"Name":"Fixture"},
            "MPPT1_NrOf":1,"MPPT1_U_MPP_Min":100,"MPPT1_U_MPP_Max":500,"MPPT1_MaxCurrent":18,
            "MPPT1_MaxShortCircuitCurrent":25,"MaxInputVoltageDC":600,"MaxAC_Power":2800,"E50":0.96}]
    });
    if let Some(coupling) = catalog_coupling {
        db["BatterySystems"] = json!([{"Id":"bs","Version":1,"Coupling":coupling,"Name":"Synthetic battery system","Company":{"Name":"Fixture"},
            "NominalPower":5000,"MaxCharge":5000,"MaxDischarge":7500,"SocMin":0.0,"NumberOfBatteriesPerString":1,"NumberOfBatteryStrings":1,
            "E0":0.8,"E5":0.82,"E10":0.83,"E20":0.87,"E30":0.9,"E50":0.95,"E75":0.97,"E100":0.98,
            "Battery":{"Id":"b","Version":1,"Name":"Synthetic LFP","BatteryType":201,"RatedVoltage":3.2,"NumberOfCellsSerie":16,
                "NumberOfCellsParallel":1,"InternalResistance":0.025,"Selfdischarge":0.03,"CapacityAt1":100,"CapacityAt10":100,
                "LifeCycles80":7040}}]);
    }
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
    archive_with(&source, false, None);
    let before = fs::read(&source).unwrap();
    let preview = call("import_pvsol", json!({"source": source}));
    assert_eq!(preview["reference"]["installed_modules"], 14);
    assert_eq!(preview["reference"]["inverters"], 2);
    assert_eq!(preview["reference"]["channels"]["inverter_ac"]["annual_kwh"], 12.0);
    assert!(!destination.exists());
    call("import_pvsol", json!({"source": source,"file": destination}));
    let reopened = app::get_project(&destination).unwrap();
    assert!(reopened.storage.is_none());
    assert!(reopened.library.inverters.iter().all(|i| i.battery_port.is_none()));
    assert_eq!(reopened.site.longitude.value, -74.1);
    assert_eq!(reopened.site.utc_offset(), -5.0);
    assert_eq!(reopened.library.modules[0].catalog_ref.as_ref().unwrap().version, 2);
    assert_eq!(reopened.arrays.len(), 2);
    assert_eq!(reopened.arrays[1].strings.value, 0);
    assert_eq!(reopened.electrical.inverters[0].mppts[0][0].modules_per_string, 7);
    // PV*SOL saves an unoccupied MPPT as a 0 × 0 string; it is skipped, not rejected.
    assert_eq!(reopened.electrical.inverters[0].mppts[0].len(), 1);
    // WBV row k is centred on k:00 local standard time: the sun is evaluated there,
    // while the calendar slot stays hour k.
    {
        let store = solarcraft_store::Store::open(&destination).unwrap();
        let (weather, _) = app::load_weather(&store, &reopened).unwrap();
        for hour in &weather.hours[..48] { assert_eq!(Some(hour.unix_mid), hour.unix_start); }
    }
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

#[test]
fn dc_coupled_battery_system_and_absolute_feed_in_limit_are_migrated() {
    use solarcraft_model::plant::Coupling;
    let dir = std::env::temp_dir().join(format!("SolarCraft-PVPRJ-battery-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("battery.pvprj");
    let destination = dir.join("battery.solarcraft");
    archive_with(&source, true, Some(2));
    call("import_pvsol", json!({"source": source, "file": destination}));
    let p = app::get_project(&destination).unwrap();
    assert_eq!(p.grid.export_limit_kw, Some(2.5), "absolute limit stored as kW / 100");
    let storage = p.storage.as_ref().expect("battery system migrated");
    assert!(matches!(&storage.coupling, Coupling::DcHybrid { unit_id } if *unit_id == p.electrical.inverters[0].id));
    let battery = p.library.battery(&storage.battery_id).unwrap();
    let cells = battery.cells.as_ref().unwrap();
    assert!((cells.internal_resistance_ohm - 0.025 / 16.0).abs() < 1e-12, "InternalResistance is per catalog battery");
    assert!((cells.pack_resistance() - 0.025).abs() < 1e-12);
    assert!((battery.capacity_kwh - 5.12).abs() < 1e-9);
    assert_eq!((battery.p_charge_max_kw, battery.p_discharge_max_kw), (5.0, 7.5));
    let port = p.library.inverters[0].battery_port.as_ref().unwrap();
    assert_eq!((port.efficiency_curve.len(), port.p_rated_w), (8, Some(5000.0)));
    assert!((port.efficiency(400.0) - 0.826).abs() < 1e-9, "8 % load: between the 5 % (0.82) and 10 % (0.83) knots");
    assert!((port.efficiency(5000.0) - 0.98).abs() < 1e-12);
    app::simulate(&destination, |_| true).unwrap();
    let result = call("compare_pvsol", json!({"file": destination}));
    assert!(result["comparison"]["inverter_ac"]["solarcraft_basis"].as_str().unwrap().starts_with("PV and battery delivered"));
    for file in [source, destination] { fs::remove_file(file).unwrap(); }
    fs::remove_dir(dir).unwrap();
}

/// Catalog coupling 3 (DC link, as in a GoodWe ES hybrid system) with the saved
/// generator-side flag stays a DC battery on the hybrid inverter.
#[test]
fn dc_link_battery_system_is_migrated_to_the_hybrid_port() {
    use solarcraft_model::plant::Coupling;
    let dir = std::env::temp_dir().join(format!("SolarCraft-PVPRJ-dc-link-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("battery.pvprj");
    let destination = dir.join("battery.solarcraft");
    archive_with(&source, true, Some(3));
    let imported = call("import_pvsol", json!({"source": source, "file": destination}));
    let p = app::get_project(&destination).unwrap();
    let storage = p.storage.as_ref().expect("battery system migrated");
    assert!(matches!(&storage.coupling, Coupling::DcHybrid { unit_id } if *unit_id == p.electrical.inverters[0].id));
    assert!(p.library.inverters[0].battery_port.is_some());
    assert!(imported["reference"]["warnings"].as_array().unwrap().iter()
        .any(|warning| warning.as_str().unwrap().contains("DC link")));
    for file in [source, destination] { fs::remove_file(file).unwrap(); }
    fs::remove_dir(dir).unwrap();
}

#[test]
fn ac_battery_catalog_overrides_stale_dc_flag_and_keeps_pv_meter() {
    use solarcraft_model::plant::Coupling;
    let dir = std::env::temp_dir().join(format!("SolarCraft-PVPRJ-ac-battery-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("battery.pvprj");
    let destination = dir.join("battery.solarcraft");
    archive_with(&source, true, Some(1));
    let imported = call("import_pvsol", json!({"source": source, "file": destination}));
    let p = app::get_project(&destination).unwrap();
    let storage = p.storage.as_ref().expect("battery system migrated");
    assert_eq!(storage.coupling, Coupling::Integrated);
    assert!(p.library.inverters.iter().all(|i| i.battery_port.is_none()));
    let converter = p.library.battery(&storage.battery_id).unwrap().converter.as_ref().unwrap();
    assert_eq!((converter.p_rated_w, converter.p_charge_max_w, converter.p_discharge_max_w), (5000.0, 5000.0, 7500.0));
    assert_eq!(converter.efficiency_curve.len(), 8);
    assert!(imported["reference"]["warnings"].as_array().unwrap().iter()
        .any(|warning| warning.as_str().unwrap().contains("conflicting saved DC flag")));
    app::simulate(&destination, |_| true).unwrap();
    let result = call("compare_pvsol", json!({"file": destination}));
    assert_eq!(result["comparison"]["inverter_ac"]["solarcraft_basis"], "inverter AC output");
    for file in [source, destination] { fs::remove_file(file).unwrap(); }
    fs::remove_dir(dir).unwrap();
}

#[test]
fn missing_active_battery_revision_preserves_the_pv_design() {
    let dir = std::env::temp_dir().join(format!("SolarCraft-PVPRJ-missing-battery-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("battery.pvprj");
    let destination = dir.join("battery.solarcraft");
    archive_with(&source, true, None);
    let imported = call("import_pvsol", json!({"source": source, "file": destination}));
    assert_eq!(imported["reference"]["installed_modules"], 14);
    assert!(imported["reference"]["warnings"].as_array().unwrap().iter()
        .any(|warning| warning.as_str().unwrap().contains("one identifiable embedded revision")));
    let p = app::get_project(&destination).unwrap();
    assert!(p.storage.is_none());
    assert!(p.library.inverters.iter().all(|i| i.battery_port.is_none()));
    for file in [source, destination] { fs::remove_file(file).unwrap(); }
    fs::remove_dir(dir).unwrap();
}
