// SPDX-License-Identifier: GPL-3.0-or-later
//! Conservative import of Project.xml AllCables into the editable cable plan.
use super::{children, number, path_node, value};
use roxmltree::Node;
use solarcraft_model::{cable::{self, Binding, Circuit, Kind, Metal}, Project};

fn metal(node: Node<'_, '_>, key: &str) -> Option<Metal> {
    match value(node, &[key])? {
        "cableMaterial_Copper" => Some(Metal::Copper),
        "cableMaterial_Aluminium" => Some(Metal::Aluminium),
        _ => None,
    }
}

fn row(node: Node<'_, '_>, length: &str, area: &str, material: &str,
       kind: Kind, label: String, binding: Binding, circuits: u32) -> Option<Circuit> {
    let length = number::<f64>(node, &[length, "Value"])?;
    let area = number::<f64>(node, &[area, "Value"])?;
    let metal = metal(node, material)?;
    if !(0.0..=100_000.0).contains(&length) || length == 0.0 || !(0.01..=2000.0).contains(&area) { return None; }
    // PV*SOL's DC resistance uses the saved conductor length directly; SolarCraft's
    // extra_m represents total series conductor length, without doubling it.
    let dc = matches!(kind, Kind::DcString | Kind::DcMain);
    Some(Circuit { label, kind, metal, length_m: if dc { 0.0 } else { length },
        extra_m: if dc { length } else { 0.0 }, area_mm2: area, temperature_c: 20.0,
        voltage_v: 0.0, current_a: 0.0, circuits, target_drop_pct: 2.0,
        source_route: None, ampacity_a: None, derating: 1.0, candidates: vec![],
        binding: Some(binding), sizing_factor: 1.0, protection_a: None,
        design_current_a: None, code_protection: None, local_rule: None })
}

fn retain(project: &mut Project, draft: Option<Circuit>) -> bool {
    let Some(mut draft) = draft else { return false; };
    if !draft.is_dc() {
        draft.voltage_v = project.grid.voltage_v * if draft.kind == Kind::AcThree { 3.0_f64.sqrt() } else { 1.0 };
    }
    let Ok(circuit) = cable::resolve(project, &project.electrical, &draft) else { return false; };
    if circuit.estimate().is_err() { return false; }
    project.cable_plan.push(circuit);
    true
}

fn has_length(node: Node<'_, '_>, key: &str) -> bool {
    number::<f64>(node, &[key, "Value"]).is_some_and(|x| x != 0.0)
}

pub(super) fn apply(project: &mut Project, root: Node<'_, '_>, warnings: &mut Vec<String>) {
    let Some(all) = path_node(root, &["AllCables"]) else { return; };
    let saved: Vec<_> = children(all, "CablesPerInverter").collect();
    let wiring: Vec<_> = children(root, "Wechselrichter").flat_map(|n| children(n, "Verschaltung"))
        .flat_map(|n| children(n, "WRObj")).collect();
    let nonzero = all.descendants().filter(|n| matches!(n.tag_name().name(), "CableLengthAc" | "CableLength_String" | "CableLength_Dc"))
        .any(|n| number::<f64>(n, &["Value"]).is_none_or(|x| x != 0.0));
    if !nonzero { return; }
    // The project loader pairs cable lists and inverter configurations by index.
    // Check the cardinalities before expanding one configuration to physical units.
    let counts = wiring.iter().map(|&node| number::<usize>(node, &["Haeufigkeit"])).collect::<Option<Vec<_>>>();
    let Some(counts) = counts.filter(|counts| saved.len() == counts.len()
        && counts.iter().sum::<usize>() == project.electrical.inverters.len()) else {
        warnings.push("Saved cable lengths could not be bound unambiguously to inverter configurations; cable losses remain unchanged.".into());
        return;
    };
    let units = project.electrical.inverters.clone();
    let start = project.cable_plan.len();
    let mut omitted = false;
    let mut first = 0;
    for (config, count) in saved.into_iter().zip(counts) {
        let trackers: Vec<_> = children(config, "CablesPerMppTrackers").collect();
        for unit in &units[first..first + count] {
            let inv = project.library.inverter(&unit.inverter_id);
            if has_length(config, "CableLengthAc") {
                let kind = match inv.and_then(|i| i.phases) {
                    Some(1) => Some(Kind::AcSingle),
                    Some(3) => Some(Kind::AcThree),
                    _ => None,
                };
                let circuit = kind.and_then(|kind| row(config, "CableLengthAc", "CrossSectionAc", "MaterialAc", kind,
                    format!("PV*SOL {} AC", unit.name), Binding::InverterAc { unit_id: unit.id.clone() }, 1));
                omitted |= !retain(project, circuit);
            }
            let inputs: Vec<_> = unit.mppts.iter().enumerate().filter_map(|(input, groups)|
                (!groups.is_empty() || trackers.len() == unit.mppts.len()).then_some(input)).collect();
            if trackers.len() != inputs.len() {
                omitted |= trackers.iter().any(|&node| has_length(node, "CableLength_String") || has_length(node, "CableLength_Dc"));
                continue;
            }
            for (&tracker, input) in trackers.iter().zip(inputs) {
                let binding = Binding::Mppt { unit_id: unit.id.clone(), input };
                let prefix = format!("PV*SOL {} tracker {}", unit.name, input + 1);
                let groups = &unit.mppts[input];
                if has_length(tracker, "CableLength_String") {
                    let group = groups.first().filter(|g| groups.len() == 1 && g.series_chain.is_none() && g.optimizer.is_none());
                    let circuit = group.and_then(|g| row(tracker, "CableLength_String", "CrossSection_String", "Material_String",
                        Kind::DcString, format!("{prefix} string"), binding.clone(), g.strings));
                    omitted |= !retain(project, circuit);
                }
                if has_length(tracker, "CableLength_Dc") {
                    match value(tracker, &["DcConnectionType"]) {
                        Some("dcConnection_none") => {}
                        Some("dcConnection_gcb" | "dcConnection_tPlug" | "dcConnection_gcbSpd" | "dcConnection_tPlugSpd") => {
                            let circuit = row(tracker, "CableLength_Dc", "CrossSection_Dc", "Material_Dc",
                                Kind::DcMain, format!("{prefix} DC main"), binding, 1);
                            omitted |= !retain(project, circuit);
                        }
                        _ => omitted = true,
                    }
                }
            }
        }
        first += count;
    }
    if project.cable_plan.len() > start {
        warnings.push("Saved cable dimensions were added to the cable plan for review; they do not change simulated losses. The plan assumes 20 °C and a 2% drop target; verify these and enter ampacity and protection before applying. PV*SOL and SolarCraft use slightly different conductor resistivities.".into());
    }
    if omitted { warnings.push("Some saved cable entries could not be mapped reliably; their losses remain unchanged.".into()); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;
    #[test]
    fn saved_dc_length_is_series_conductor_length_and_material_is_strict() {
        let xml = "<c><L><Value>12</Value></L><A><Value>6</Value></A><M>cableMaterial_Aluminium</M></c>";
        let doc = Document::parse(xml).unwrap();
        let b = Binding::Mppt { unit_id: "i".into(), input: 0 };
        let c = row(doc.root_element(), "L", "A", "M", Kind::DcString, "s".into(), b.clone(), 2).unwrap();
        assert_eq!((c.length_m, c.extra_m, c.area_mm2, c.circuits, c.metal), (0.0, 12.0, 6.0, 2, Metal::Aluminium));
        assert_eq!(c.ampacity_a, None);
        assert!(row(doc.root_element(), "L", "A", "missing", Kind::DcString, "s".into(), b, 2).is_none());
    }
}
