// SPDX-License-Identifier: GPL-3.0-or-later
//! Restore checked flush module formations on supported attached shed roofs.
use super::{add, children, dist, document, enu, member, normalized_guid, number, path_node, scale, value, xy, xyz,
    Archive, FieldLayout, Kind, ModuleDims, ModuleField, Mount, Orientation, PlacedModule, Project, Scene,
    Source, StringRoute, GEOMETRY, WIRING};
use roxmltree::Node;
use solarcraft_scene::{mesh, place::MpptBinding, V3};
use std::collections::{HashMap, HashSet};

const STANDOFF: f64 = 0.02;

pub(super) fn read(
    project: &Project, root: Node<'_, '_>, objects: &[Node<'_, '_>], refs: &[usize], scene: &Scene, archive: &mut Archive,
) -> Option<Vec<ModuleField>> {
    let [array] = project.arrays.as_slice() else { return None; };
    if !scene.objects.iter().any(|o| matches!(o.kind, Kind::Dormer { .. })) { return None; }
    let area = normalized_guid(value(root, &["PVModule", "Modulfelder", "ModulfeldID"])?);
    let part = member(archive, WIRING, 8 * 1024 * 1024).ok()??;
    let doc = document(&part, WIRING).ok()?;
    let wiring = doc.root_element();
    let [reference] = children(wiring, "ReferenzModul").collect::<Vec<_>>().try_into().ok()?;
    if normalized_guid(value(reference, &["GUID_XML"])?) != area { return None; }
    let installed = number::<usize>(reference, &["ModulAnzahl"])?;
    if !(1..=1200).contains(&installed) { return None; }
    let tilt = number::<f64>(reference, &["AusrichtungStatisch", "Neigung"])?.to_degrees();
    let azimuth = number::<f64>(reference, &["AusrichtungStatisch", "Azimut"])?.to_degrees();
    // The project rounds the Visual3D orientation to whole degrees.
    if (tilt - array.tilt.value).abs() > 0.51 || (azimuth - array.azimuth.value).abs() > 0.51 { return None; }
    let native_ids: HashSet<_> = children(reference, "FormsList").map(|n| n.text().map(normalized_guid)).collect::<Option<_>>()?;
    if native_ids.len() < 2 { return None; }
    let catalog = project.library.module(&array.module_id.value)?;
    let module = ModuleDims { length: catalog.length_m.max(catalog.width_m), width: catalog.length_m.min(catalog.width_m),
        bypass_diodes: catalog.bypass_diodes.unwrap_or(3) };
    let mut fields = Vec::new();
    let mut slots = HashMap::new();
    let mut removed = native_ids;
    for node in objects {
        let Some(formation) = path_node(*node, &["SpezifDaten", "ModulFormation"]) else { continue; };
        let id = normalized_guid(value(*node, &["StandardDaten", "SignaturXML"])?);
        if !removed.remove(&id) { continue; }
        let geometry = path_node(*node, &["Zuordn_XML"])?;
        let index = *refs.get(number::<usize>(geometry, &["Index_BezugsObj"])?)?;
        if number::<u32>(geometry, &["Index_BezugsUnterObj"]) != Some(1)
            || number::<u32>(geometry, &["Index_BezugsFlaeche"]) != Some(0) { return None; }
        let parent = scene.object(&format!("visual3d-dormer-{index}"))?;
        let Kind::Dormer { profile } = &parent.kind else { return None; };
        if profile.gable.is_some() { return None; }
        if (profile.roof_pitch_deg - tilt).abs() > 0.01 || (profile.facing_deg - azimuth).abs() > 0.01 { return None; }
        let shape = mesh::faces(parent).into_iter().find(|face| face.role == mesh::Role::Roof)?;
        let [a, b, _, d] = shape.outer.try_into().ok()?;
        let u = mesh::norm(mesh::sub(b, a));
        let v = mesh::norm(mesh::sub(d, a));
        let normal = mesh::norm(mesh::cross(u, v));
        let [x, y] = xy(path_node(*node, &["StandardDaten", "PosAufBezugsFL"])?)?;
        let origin = add(a, add(scale(u, x), scale(v, y)));
        if dist(origin, enu(xyz(*node, &["StandardDaten", "Position"])?)) > 0.002 { return None; }
        let reference_id = normalized_guid(value(formation, &["SignaturRefModul"])?);
        let reference_module = objects.iter().find(|n| value(**n, &["StandardDaten", "SignaturXML"])
            .is_some_and(|v| normalized_guid(v) == reference_id))?;
        let reference_pose = path_node(*reference_module, &["StandardDaten", "ZweifacheSkalierung"])?;
        let width = number::<f64>(reference_pose, &["Ebene1", "BreiteR"])?;
        let height = number::<f64>(reference_pose, &["Ebene2", "Hoehe"])?;
        if (width - module.length).abs() > 0.003 || (height - module.width).abs() > 0.003 { return None; }
        let gap_x = number::<f64>(formation, &["AbstandModule", "Horizontal"])?;
        let gap_y = number::<f64>(formation, &["AbstandModule", "Vertikal"])?;
        if ![gap_x, gap_y].into_iter().all(|v| (0.0..=1.0).contains(&v)) { return None; }
        // The supported landscape formation stores the two counts in the
        // module's axes, transposed relative to Zeile/Spalte on its roof.
        let cols = number::<u32>(formation, &["AnzModuleZe"])?;
        let rows = number::<u32>(formation, &["AnzModuleSp"])?;
        let local_id = format!("visual3d-field-{}", fields.len() + 1);
        let mut modules = Vec::new();
        let mut positions = HashSet::new();
        let by_index = fields.len();
        for panel in children(formation, "Module") {
            let native = normalized_guid(value(panel, &["SignaturXML"])?);
            let slot = number::<usize>(panel, &["IndexInAbschGrBin"])?;
            let row = number::<u32>(panel, &["Zeile"])?;
            let col = number::<u32>(panel, &["Spalte"])?;
            if row >= rows || col >= cols || !positions.insert((row, col)) { return None; }
            let x0 = f64::from(col) * (width + gap_x);
            let y0 = f64::from(row) * (height + gap_y);
            if x < 0.0 || y < 0.0 || x + x0 + width > dist(a,b) + 0.001
                || y + y0 + height > dist(a,d) + 0.001 { return None; }
            let corners: [V3; 4] = [[x0, y0], [x0 + width, y0], [x0 + width, y0 + height], [x0, y0 + height]]
                .map(|[x, y]| add(add(origin, scale(normal, STANDOFF)), add(scale(u, x), scale(v, y))));
            if slots.insert(slot, (by_index, modules.len(), native.clone())).is_some() { return None; }
            modules.push(PlacedModule { id: format!("{local_id}/{native}"), corners, normal, row, col, portrait: false, string: None });
        }
        if modules.is_empty() { return None; }
        fields.push(ModuleField { id: local_id,
            name: parent.name.clone(), array_id: Some(array.id.clone()), mount: Mount::Roof { face: shape.id, standoff: STANDOFF },
            module: module.clone(), layout: FieldLayout { orientation: Orientation::Landscape, gap_x, gap_y, ..Default::default() },
            modules, strings: vec![], inverter_at: None,
            source: Some(Source { format: "pvsol_visual3d".into(), file: GEOMETRY.into(), external_id: id, local_geometry: false }) });
    }
    if !removed.is_empty() || slots.len() != installed { return None; }
    let bindings = super::super::archived_routes::bindings(project, wiring, &HashMap::from([(area.clone(), 0)]))?;
    for input in path_node(wiring, &["Verschaltung"])?.descendants().filter(|n| n.has_tag_name("MPP_Tracker")) {
        for string in children(input, "Strang") {
            if children(string, "SubStrang").next().is_some() { return None; }
            let mut binding: Option<MpptBinding> = None;
            let mut field_index = None;
            let mut indices = vec![];
            for panel in children(string, "Modul") {
                let slot = number::<usize>(panel, &["AbschGrad", "Index_SchattenArray"])?;
                let (field, index, native) = slots.remove(&slot)?;
                let address = bindings.get(&slot)?;
                if normalized_guid(value(panel, &["Zugehoerigkeit", "GUID_Modul"])?) != native
                    || field_index.is_some_and(|ix| ix != field) || binding.as_ref().is_some_and(|b| b != address) { return None; }
                field_index = Some(field);
                binding = Some(address.clone());
                indices.push(index);
            }
            let field = &mut fields[field_index?];
            let index = field.strings.len() as u32;
            for &m in &indices { field.modules[m].string = Some(index); }
            let jumper_m = indices.windows(2).map(|pair| dist(field.modules[pair[0]].center(), field.modules[pair[1]].center())).sum();
            let modules = indices.into_iter().map(|i| field.modules[i].id.clone()).collect();
            field.strings.push(StringRoute { id: format!("{}/s{}", field.id,index), index, modules, jumper_m, home_run_m: 0.0, binding });
        }
    }
    slots.is_empty().then_some(fields)
}
