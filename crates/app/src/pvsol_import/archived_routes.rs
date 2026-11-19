// SPDX-License-Identifier: GPL-3.0-or-later
//! Match physical Visual3D strings to the expanded Project.xml inverter topology.
use super::{children, number, path_node, value};
use roxmltree::Node;
use solarcraft_model::{archived_shading::Route as ArchivedShadeRoute, plant::physical_strings, Project};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

type StringShape = Vec<(usize, usize)>;
type WireSlots = Vec<Vec<usize>>;

pub(super) fn bind(
    project: &Project, wiring: Node<'_, '_>, area_ids: &HashMap<String, usize>,
) -> Option<Vec<Vec<ArchivedShadeRoute>>> {
    if !wiring.has_tag_name("Modulverschaltung_PVSol") || !project.electrical.manual { return None; }
    let wiring = path_node(wiring, &["Verschaltung"])?;
    let array_indices: HashMap<_, _> = project.arrays.iter().enumerate().map(|(i, a)| (a.id.as_str(), i)).collect();
    let mut routes: Vec<Vec<_>> = (0..project.arrays.len()).map(|_| Vec::new()).collect();
    let mut unsupported = HashSet::new();
    let mut inverter_numbers = HashSet::new();
    let mut used_slots = HashSet::new();

    for node in children(wiring, "TeilAnlage").flat_map(|part| children(part, "WechselRichter")) {
        let unit_index = number::<usize>(node, &["AllgDaten", "Nummer"])?.checked_sub(1)?;
        if !inverter_numbers.insert(unit_index) { return None; }
        let unit = project.electrical.inverters.get(unit_index)?;
        let product = project.library.inverter(&unit.inverter_id)?.catalog_ref.as_ref()?;
        if normalized_guid(value(node, &["AllgDaten", "GUID_XML"])?) != normalized_guid(&product.id)
            || number::<i64>(node, &["AllgDaten", "Version_XML"]) != Some(product.version) {
            return None;
        }
        let has_optimizer = unit.mppts.iter().flatten().any(|group| group.optimizer.is_some());
        let saved_optimizer = match value(node, &["PowerOptimizer"])? {
            "0" => false,
            "1" | "-1" => true,
            _ => return None,
        };
        if saved_optimizer != has_optimizer { return None; }

        let mut tracker_numbers = HashSet::new();
        for tracker in children(node, "MPP_Tracker") {
            let input = number::<usize>(tracker, &["AllgDaten", "Nummer"])?.checked_sub(1)?;
            if !tracker_numbers.insert(input) { return None; }
            let groups = unit.mppts.get(input)?;
            let mut physical_routes: BTreeMap<StringShape, VecDeque<WireSlots>> = BTreeMap::new();
            for string in children(tracker, "Strang") {
                let (shape, slots) = read_string(string, area_ids, &mut used_slots)?;
                physical_routes.entry(shape).or_default().push_back(slots);
            }
            let unsupported_tracker = unit.interconnected.iter().any(|linked| linked.contains(&input))
                || groups.iter().any(|group| group.series_chain.is_some() || group.optimizer.is_some());
            if unsupported_tracker {
                for group in groups { unsupported.insert(*array_indices.get(group.array_id.as_str())?); }
            }

            for chain in physical_strings(groups) {
                let first = *chain.first()?;
                let copies = groups[first].strings as usize;
                if copies == 0 || chain.iter().any(|&group| groups[group].strings as usize != copies) { return None; }
                let shape: StringShape = chain.iter().map(|&index| {
                    let group = &groups[index];
                    Some((*array_indices.get(group.array_id.as_str())?, group.modules_per_string as usize))
                }).collect::<Option<_>>()?;
                let matches = physical_routes.get_mut(&shape)?;
                if matches.len() < copies { return None; }
                let actual: Vec<Vec<usize>> = matches.drain(..copies).map(|segments| {
                    // Ordinary strings have exactly one field segment.
                    segments.into_iter().flatten().collect()
                }).collect();
                if !unsupported_tracker {
                    let array = shape.first()?.0;
                    routes[array].push(ArchivedShadeRoute { unit_id: unit.id.clone(), input, group: first, strings: actual });
                }
            }
            if physical_routes.values().any(|strings| !strings.is_empty()) { return None; }
        }
        if unit.mppts.iter().enumerate().any(|(i, groups)| !groups.is_empty() && !tracker_numbers.contains(&i)) {
            return None;
        }
    }

    if inverter_numbers.len() != project.electrical.inverters.len() { return None; }
    for index in unsupported { routes[index].clear(); }
    Some(routes)
}

fn read_string(
    string: Node<'_, '_>, area_ids: &HashMap<String, usize>, used_slots: &mut HashSet<usize>,
) -> Option<(StringShape, WireSlots)> {
    let segments: Vec<_> = children(string, "SubStrang").collect();
    let segments = if segments.is_empty() { vec![string] } else {
        if children(string, "Modul").next().is_some() { return None; }
        segments
    };
    let mut shape = Vec::with_capacity(segments.len());
    let mut wire_slots = Vec::with_capacity(segments.len());
    for segment in segments {
        let area = normalized_guid(value(segment, &["AllgDaten", "GUID_XML"])?);
        let array_index = *area_ids.get(&area)?;
        let slots: Vec<_> = children(segment, "Modul").map(|module| {
            let slot = number::<usize>(module, &["AbschGrad", "Index_SchattenArray"])?;
            used_slots.insert(slot).then_some(slot)
        }).collect::<Option<_>>()?;
        if slots.is_empty() { return None; }
        shape.push((array_index, slots.len()));
        wire_slots.push(slots);
    }
    Some((shape, wire_slots))
}

fn normalized_guid(raw: &str) -> String {
    raw.trim().trim_matches(['{', '}']).to_ascii_lowercase()
}
