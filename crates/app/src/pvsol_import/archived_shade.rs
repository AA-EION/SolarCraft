// SPDX-License-Identifier: GPL-3.0-or-later
//! Area-average replay of saved Visual3D shading at the original site and orientation.
use super::{archive_path, children, document, member, number, path_node, value, HOURS};
use crate::{e, Result};
use roxmltree::Node;
use solarcraft_model::{derive, ArchivedShading, Project};
use std::{collections::{HashMap, HashSet}, io::Cursor};
use zip::ZipArchive;

const DIRECT: &str = "Visu3D/DirektAbschattungsgrade.bin";
const DIFFUSE: &str = "Visu3D/DiffusAbschattungsgrade.bin";
const WIRING: &str = "Visu3D/Modul_Verschaltung.xml";
const SETTINGS: &str = "Visu3D/ProjDaten.xml";

pub(super) fn apply(
    project: &mut Project, root: Node<'_, '_>, archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    warnings: &mut Vec<String>,
) -> Result<usize> {
    if !archive.file_names().any(|name| name.is_ok_and(|name| archive_path(&name) == DIRECT)) { return Ok(0); }
    let bindings = match read(project, root, archive) {
        Ok(bindings) => bindings,
        Err(error) => {
            warnings.push(format!("Saved Visual3D shading was omitted. {}", error.message));
            return Ok(0);
        }
    };
    let count = bindings.len();
    for (i, shading) in bindings { project.arrays[i].archived_shading = Some(shading); }
    if count > 0 {
        warnings.push(format!("Replays saved Visual3D shading for {count} module areas. For each hour, direct shade uses the average affected substring fraction and tilted diffuse shade uses the field average. Per-string I–V shading and editable scene geometry require separate migration."));
    }
    Ok(count)
}

fn read(
    project: &Project, root: Node<'_, '_>, archive: &mut ZipArchive<Cursor<Vec<u8>>>,
) -> Result<Vec<(usize, ArchivedShading)>> {
    let settings = required(archive, SETTINGS, 4 * 1024 * 1024)?;
    let settings = document(&settings, SETTINGS)?;
    let settings_root = settings.root_element();
    if !settings_root.has_tag_name("ProjektDaten_XML") {
        return Err(error("Unrecognized Visual3D project settings."));
    }
    let enabled = |key| matches!(number::<i32>(settings_root, &["Abschattung", key]), Some(-1 | 1));
    if !enabled("SimDurchgefuehrt") || !enabled("DirektAbschVorhanden") || enabled("SimUebersprungen") {
        return Err(error("The archive does not mark its direct shade simulation as completed."));
    }

    let direct = required(archive, DIRECT, 64 * 1024 * 1024)?;
    let slots = direct_slots(&direct)?;
    let rows = effective_rows(&direct[12..], slots)?;
    let diffuse = required(archive, DIFFUSE, 1024 * 1024)?;
    if diffuse.len() != 4 + 4 * slots || i32_at(&diffuse, 0) != Some(slots as i32) {
        return Err(error("Saved diffuse and direct shade slot counts do not match."));
    }
    let diffuse: Vec<_> = diffuse[4..].chunks_exact(4).map(|bytes| {
        let pct = i32::from_le_bytes(bytes.try_into().unwrap());
        (pct <= 100).then_some(f64::from(pct.max(0)) / 100.0)
    }).collect::<Option<_>>().ok_or_else(|| error("Saved tilted diffuse percentages exceed 100."))?;

    let area_ids: HashMap<_, _> = path_node(root, &["PVModule"]).into_iter()
        .flat_map(|p| children(p, "Modulfelder"))
        .enumerate()
        .filter_map(|(i, node)| value(node, &["ModulfeldID"]).map(|id| (guid(id), i)))
        .collect();
    if area_ids.len() != project.arrays.len() {
        return Err(error("PV module-area IDs cannot be joined to Visual3D wiring."));
    }
    let wiring = required(archive, WIRING, 12 * 1024 * 1024)?;
    let wiring = document(&wiring, WIRING)?;
    let by_array = shade_slots(wiring.root_element(), &area_ids, slots, project.arrays.len())?;
    let expected = derive::modules_by_array(&project.electrical);
    let site = [project.site.latitude.value, project.site.longitude.value, project.site.utc_offset()];
    let mut bound = Vec::new();
    for (index, (array, indices)) in project.arrays.iter().zip(by_array).enumerate() {
        if indices.len() != expected.get(&array.id).copied().unwrap_or(0) as usize {
            return Err(error(format!("The 3D shade wiring count does not match {}.", array.name.value)));
        }
        if indices.is_empty() { continue; }
        let module = project.library.module(&array.module_id.value)
            .ok_or_else(|| error("Missing module specification for archived shading."))?;
        if module.bypass_diodes.is_some_and(|count| count > 64) {
            return Err(error("Unsupported bypass diode count for archived shading."));
        }
        // AddPVRefModul chooses V for StandardCellConnection and H otherwise.
        let vertical_strings = module.unverified.get("standardCellConnection")
            .and_then(|v| v.as_bool()).unwrap_or(false);
        let mut beam_loss = Vec::with_capacity(HOURS);
        for row in &rows {
            let mut sum = 0.0;
            for &index in &indices {
                let [h, v] = row.axes(index);
                sum += slot_loss(h, v, module.bypass_diodes.unwrap_or(0), vertical_strings, module.half_cell);
            }
            beam_loss.push((sum / indices.len() as f64) as f32);
        }
        let diffuse_loss = indices.iter().map(|&i| diffuse[i]).sum::<f64>() / indices.len() as f64;
        bound.push((index, ArchivedShading {
            beam_loss, diffuse_loss, tilt_deg: array.tilt.value, azimuth_deg: array.azimuth.value, site,
            module_id: array.module_id.value.clone(), installed_modules: indices.len() as u32,
        }));
    }
    Ok(bound)
}

fn shade_slots(
    root: Node<'_, '_>, arrays: &HashMap<String, usize>, slots: usize, count: usize,
) -> Result<Vec<Vec<usize>>> {
    if !root.has_tag_name("Modulverschaltung_PVSol") {
        return Err(error("Unrecognized Visual3D wiring document."));
    }
    let wiring = path_node(root, &["Verschaltung"]).ok_or_else(|| error("Missing Visual3D wiring."))?;
    let mut used = HashSet::new();
    let mut bindings = vec![vec![]; count];
    for segment in wiring.descendants().filter(|n| n.has_tag_name("Strang") || n.has_tag_name("SubStrang")) {
        let mut modules = children(segment, "Modul").peekable();
        if modules.peek().is_none() { continue; }
        let array = value(segment, &["AllgDaten", "GUID_XML"]).and_then(|id| arrays.get(&guid(id)))
            .ok_or_else(|| error("A wired Visual3D string has no matching module area."))?;
        for module in modules {
            let slot = number::<usize>(module, &["AbschGrad", "Index_SchattenArray"])
                .filter(|&i| i < slots && used.insert(i))
                .ok_or_else(|| error("A Visual3D shade slot is missing, duplicated, or out of range."))?;
            bindings[*array].push(slot);
        }
    }
    Ok(bindings)
}

fn required(archive: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str, limit: u64) -> Result<Vec<u8>> {
    member(archive, name, limit)?.ok_or_else(|| error(format!("Missing {name}.")))
}

fn error(message: impl std::fmt::Display) -> crate::AppError {
    e("PVSOL_SHADING", message)
}

fn guid(value: &str) -> String {
    value.trim().trim_matches(['{', '}']).to_ascii_lowercase()
}

fn i32_at(data: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(data.get(offset..offset + 4)?.try_into().ok()?))
}

fn direct_slots(data: &[u8]) -> Result<usize> {
    let slots = i32_at(data, 0).filter(|&v| v > 0).unwrap_or(0) as usize;
    let length = slots.checked_mul(HOURS * 3).and_then(|bytes| bytes.checked_add(12));
    if slots == 0 || i32_at(data, 4) != Some(60) || i32_at(data, 8) != Some(HOURS as i32)
        || length != Some(data.len()) {
        return Err(error("Expected 8,760 hourly direct shade rows with three bytes per module."));
    }
    Ok(slots)
}

#[derive(Clone, Copy)]
enum ShadeRow<'a> {
    Uniform([u8; 2]),
    Slots(&'a [u8]),
}

impl ShadeRow<'_> {
    fn axes(self, index: usize) -> [u8; 2] {
        match self {
            Self::Uniform(axes) => axes,
            Self::Slots(row) => [row[3 * index + 1].min(100), row[3 * index + 2].min(100)],
        }
    }
}

fn effective_rows(data: &[u8], slots: usize) -> Result<Vec<ShadeRow<'_>>> {
    let mut rows = Vec::with_capacity(HOURS);
    let mut active = ShadeRow::Uniform([data[1].min(100), data[2].min(100)]);
    for (hour, row) in data.chunks_exact(slots * 3).enumerate() {
        let mut complete = true;
        for (slot, bytes) in row.chunks_exact(3).enumerate() {
            match (bytes[1], bytes[2]) {
                (0xfe, 0xfe) if slot + 1 < slots => complete = false,
                (0xfe, 0xfe) | (0xfc, 0xfc) | (0xfd, 0xfd) | (0..=100, 0..=100) => {}
                _ => return Err(error("Invalid saved direct shade penetration or marker.")),
            }
        }
        // The first slot seeds every series. A later FE stops the record and
        // an incomplete record leaves every series at its previous rates.
        if hour > 0 && complete { active = ShadeRow::Slots(row); }
        rows.push(active);
    }
    Ok(rows)
}

fn slot_loss(horizontal: u8, vertical: u8, diodes: u32, vertical_strings: bool, half_cell: bool) -> f64 {
    if horizontal == 0 || vertical == 0 { return 0.0; }
    if diodes == 0 { return 1.0; }
    let (cross, along) = if vertical_strings { (horizontal, vertical) } else { (vertical, horizontal) };
    let affected = (u32::from(cross) * diodes).div_ceil(100);
    let halves = if half_cell { 2 } else { 1 };
    let affected_halves = (u32::from(along) * halves).div_ceil(100);
    f64::from(affected * affected_halves) / f64::from(diodes * halves)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_channels_apply_the_archived_axis_and_half_cell_rules() {
        assert_eq!(slot_loss(0, 100, 3, true, false), 0.0);
        assert_eq!(slot_loss(100, 100, 3, true, false), 1.0);
        assert_eq!(slot_loss(30, 60, 3, true, false), 1.0 / 3.0);
        assert_eq!(slot_loss(30, 60, 3, false, false), 2.0 / 3.0);
        assert_eq!(slot_loss(30, 60, 3, true, true), 2.0 / 6.0);
        assert_eq!(slot_loss(30, 60, 0, true, false), 1.0);
    }

    #[test]
    fn night_marker_truncates_a_row_and_preserves_the_last_complete_rate() {
        let rows = [
            254, 254, 254, 254, 254, 254,
            0, 20, 30, 0, 40, 50,
            254, 254, 254, 0, 70, 80,
            0, 70, 80, 254, 254, 254,
        ];
        let parsed = effective_rows(&rows, 2).unwrap();
        let axes: Vec<_> = parsed.iter().map(|row| [row.axes(0), row.axes(1)]).collect();
        assert_eq!(axes, [
            [[100, 100], [100, 100]], [[20, 30], [40, 50]],
            [[20, 30], [40, 50]], [[70, 80], [100, 100]],
        ]);
    }

    #[test]
    fn explicit_substrings_bind_to_their_own_areas() {
        let xml = r#"<Modulverschaltung_PVSol><Verschaltung><Strang>
            <AllgDaten><GUID_XML>{OTHER}</GUID_XML></AllgDaten>
            <SubStrang><AllgDaten><GUID_XML>{A}</GUID_XML></AllgDaten>
              <Modul><AbschGrad><Index_SchattenArray>1</Index_SchattenArray></AbschGrad></Modul>
            </SubStrang>
            <SubStrang><AllgDaten><GUID_XML>{B}</GUID_XML></AllgDaten>
              <Modul><AbschGrad><Index_SchattenArray>0</Index_SchattenArray></AbschGrad></Modul>
            </SubStrang>
            </Strang></Verschaltung></Modulverschaltung_PVSol>"#;
        let areas = HashMap::from([("a".into(), 0), ("b".into(), 1)]);
        let doc = roxmltree::Document::parse(xml).unwrap();
        assert_eq!(shade_slots(doc.root_element(), &areas, 2, 2).unwrap(), [vec![1], vec![0]]);
        assert!(shade_slots(doc.root_element(), &areas, 1, 2).is_err());
    }
}
