// SPDX-License-Identifier: GPL-3.0-or-later
//! Editable scene import for saved Visual3D objects and drawn roof planes.
use super::{children, document, member, number, path_node, value};
use crate::Result;
use roxmltree::Node;
use solarcraft_geometry::layout::Orientation;
use solarcraft_model::{derive, Project};
use solarcraft_scene::{
    mesh::{add, cross, dist, dot, norm, scale, sub},
    place::{FieldLayout, ModuleDims, ModuleField, Mount, PlacedModule, StringRoute},
    Georef, Kind, Layer, MeshFace, Object, Scene, Source, P2, V3,
};
use std::{collections::{BTreeMap, HashMap, HashSet}, io::Cursor};
use zip::ZipArchive;

mod buildings;

type Archive = ZipArchive<Cursor<Vec<u8>>>;
const GEOMETRY: &str = "Visu3D/GeometrischeDaten.xml";
const WIRING: &str = "Visu3D/Modul_Verschaltung.xml";
const STRING_PLAN: &str = "Visu3D/StrangPlan1.xml";
const MAX_MESH_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TRIANGLES: usize = 30_000;

pub(super) fn apply(project: &mut Project, root: Node<'_, '_>, archive: &mut Archive, warnings: &mut Vec<String>) -> usize {
    match read_scene(project, root, archive) {
        Ok(Some((scene, skipped_objects, skipped_fields))) => {
            let fields = scene.fields.len();
            let modules = scene.fields.iter().map(|f| f.modules.len()).sum::<usize>();
            let strings = scene.fields.iter().map(|f| f.strings.len()).sum::<usize>();
            let meshes = scene.objects.iter().filter(|o| o.id.starts_with("visual3d-mesh-")).count();
            let trees = scene.objects.iter().filter(|o| matches!(o.kind, Kind::Tree { .. })).count();
            let buildings = scene.objects.iter().filter(|o| matches!(o.kind, Kind::Building { .. })).count();
            let dormers = scene.objects.iter().filter(|o| matches!(o.kind, Kind::Dormer { .. })).count();
            warnings.push(format!("Restored {meshes} Visual3D meshes, {trees} editable trees, {buildings} buildings, {dormers} dormers, {fields} drawn roof fields, {modules} placed modules and {strings} saved string routes."));
            if buildings > 0 {
                warnings.push("Imported buildings show the exterior shell. Roof thickness, doors, windows and material data are omitted.".into());
            }
            if trees > 0 {
                warnings.push("Imported tree crowns use an ellipsoid and mean opacity for local edits. The saved monthly opacity values are retained as object properties.".into());
            }
            if fields == 0 {
                warnings.push("The imported objects have no bound module geometry. Edits to these objects leave the archived shading in use.".into());
            }
            if skipped_objects > 0 || skipped_fields > 0 {
                warnings.push(format!("Omitted {skipped_objects} unsupported Visual3D objects and {skipped_fields} drawn fields without a verified placement and wiring join."));
            }
            project.scene = Some(scene);
            fields
        }
        Ok(None) => 0,
        Err(error) => {
            warnings.push(format!("Saved Visual3D scene was omitted. {}", error.message));
            0
        }
    }
}

fn read_scene(project: &Project, root: Node<'_, '_>, archive: &mut Archive) -> Result<Option<(Scene, usize, usize)>> {
    let Some(bytes) = member(archive, GEOMETRY, 8 * 1024 * 1024)? else { return Ok(None); };
    let doc = document(&bytes, GEOMETRY)?;
    let geo = doc.root_element();
    if !geo.has_tag_name("VerschDatenGesamt") { return Ok(None); }
    let Some(saved) = path_node(geo, &["Szene"]) else { return Ok(None); };
    let mut scene = Scene::new(Georef {
        latitude: project.site.latitude.value, longitude: project.site.longitude.value,
        elevation: project.site.elevation.value, true_north_deg: 0.0,
    });
    let mut skipped_objects = 0;
    let mut triangles = 0;
    let nodes: Vec<_> = children(saved, "ZeichenObjekt").collect();
    let reference_objects: Vec<_> = nodes.iter().copied().enumerate().filter(|(_, node)| {
        let ty = number::<u32>(*node, &["StandardDaten", "AnwObjTyp"]);
        ty.is_some_and(|ty| ty != 4) && number::<i32>(*node, &["StandardDaten", "IstBezugsObj"]) == Some(-1)
    }).map(|(i, _)| i + 1).collect();
    for (index, node) in nodes.iter().copied().enumerate() {
        if let Some(building) = path_node(node, &["SpezifDaten", "VarHaus"]) {
            if let Some(object) = buildings::read_building(index + 1, node, building) {
                scene.objects.push(object);
            } else {
                skipped_objects += 1;
            }
            continue;
        }
        if let Some(tree) = path_node(node, &["SpezifDaten", "Baum"]) {
            if let Some(object) = read_tree(index + 1, node, tree) {
                scene.objects.push(object);
            } else {
                skipped_objects += 1;
            }
            continue;
        }
        let Some(mesh) = path_node(node, &["SpezifDaten", "MeshObj"]) else { continue; };
        if let Some(object) = read_object(index + 1, node, mesh, archive, MAX_TRIANGLES - triangles) {
            if let Kind::Mesh { faces, .. } = &object.kind { triangles += faces.len(); }
            scene.objects.push(object);
        } else {
            skipped_objects += 1;
        }
    }
    for (index, node) in nodes.iter().copied().enumerate() {
        let Some(dormer) = path_node(node, &["SpezifDaten", "Gaube"]) else { continue; };
        let parent = number::<usize>(node, &["Zuordn_XML", "Index_BezugsObj"]).and_then(|i| reference_objects.get(i))
            .and_then(|i| scene.object(&format!("visual3d-building-{i}")));
        if let Some(object) = parent.and_then(|parent| buildings::read_dormer(index + 1, node, dormer, parent)) {
            scene.objects.push(object);
        } else {
            skipped_objects += 1;
        }
    }
    if scene.objects.iter().any(|o| o.layer == "vegetation") {
        scene.layers.push(Layer { name: "vegetation".into(), visible: true, locked: false });
    }
    let planes: Vec<_> = children(saved, "ZeichenObjekt")
        .filter(|node| path_node(*node, &["SpezifDaten", "BelegFL"]).is_some()).collect();
    let mut skipped_fields = planes.len();
    if let [only_plane] = planes.as_slice() {
        // The observed single-plane archives provide one numbered plan with a
        // bijective panel-label map. Multi-plane archives need plan binding.
        if let Some((carrier, field)) = read_field(project, root, saved, *only_plane, archive) {
            scene.objects.push(carrier);
            scene.fields.push(field);
            skipped_fields = 0;
        }
    }
    if scene.objects.is_empty() { return Ok(None); }
    Ok(Some((scene, skipped_objects, skipped_fields)))
}

fn normalized_guid(s: &str) -> String {
    s.trim().trim_matches(['{','}']).to_ascii_lowercase()
}

fn xyz(node: Node<'_, '_>, path: &[&str]) -> Option<V3> {
    let n = path_node(node, path)?;
    let out = [number::<f64>(n, &["X"])?, number::<f64>(n, &["Y"])?, number::<f64>(n, &["Z"])?];
    out.iter().all(|v| (-1e7..=1e7).contains(v)).then_some(out)
}

fn xy(node: Node<'_, '_>) -> Option<P2> {
    let out = [number::<f64>(node, &["X"])?, number::<f64>(node, &["Y"])?];
    out.iter().all(|v| (-100_000.0..=100_000.0).contains(v)).then_some(out)
}

// Visual3D's X and Z run west and south, with Y up. Cross-check:
// the Fabrik roof's saved basis and this mapping give a south-facing 8.53° plane.
fn enu([x, y, z]: V3) -> V3 { [-x, -z, y] }

fn read_tree(index: usize, node: Node<'_, '_>, tree: Node<'_, '_>) -> Option<Object> {
    let standard = path_node(node, &["StandardDaten"])?;
    if xyz(standard, &["Skalierung"])? != [1.0; 3] { return None; }
    let upright = ["ZenitWinkel", "RollWinkel"].into_iter().all(|axis| {
        number::<f64>(standard, &["Rotation", axis]).is_some_and(|v| v.abs() < 0.001)
    });
    let azimuth = number::<f64>(standard, &["Rotation", "AzimutWinkel"])?;
    if !upright || (azimuth.rem_euclid(360.0) - 180.0).abs() >= 0.001 { return None; }
    let diameter = |path: &[&str]| number::<f64>(tree, path).filter(|v| (0.001..=1000.0).contains(v));
    let crown_height = diameter(&["Krone", "Hoehe"])?;
    let height = diameter(&["BaumHoehe"])?;
    if height < crown_height { return None; }
    let centroid = number::<f64>(tree, &["Krone", "HoeheSchwPkt"]).filter(|v| (0.0..=1.0).contains(v))?;
    let monthly: Vec<_> = ["Jan", "Feb", "Mar", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"]
        .into_iter().map(|month| number::<f64>(tree, &["KronenAbschGrad_Monat", month])
            .filter(|v| (0.0..=1.0).contains(v))).collect::<Option<_>>()?;
    let [east, north, base_z] = enu(xyz(standard, &["Position"])?);
    let trunk_radius = diameter(&["StammDurchmesser"])? * 0.5;
    // In the saved overview plans, this pose anchors the lower trunk
    // corner. The center is one radius farther east and north.
    let kind = Kind::Tree {
        center: [east + trunk_radius, north + trunk_radius], base_z, trunk_height: height - crown_height,
        trunk_radius,
        crown_radius: diameter(&["Krone", "Durchmesser"])? * 0.5,
        crown_height, transparency: 1.0 - monthly.iter().sum::<f64>() / 12.0, segments: 16,
    };
    let properties = BTreeMap::from([
        ("pvsol_crown_opacity_jan_to_dec".into(), serde_json::to_string(&monthly).ok()?),
        ("pvsol_crown_centroid_fraction".into(), centroid.to_string()),
    ]);
    Some(Object {
        id: format!("visual3d-tree-{index}"), name: value(standard, &["Bezeichnung"])?.to_owned(),
        layer: "vegetation".into(), kind, hidden: false, properties,
        source: Some(Source { format: "pvsol_visual3d".into(), file: GEOMETRY.into(),
            external_id: normalized_guid(value(standard, &["SignaturXML"])?), local_geometry: false }),
    })
}

fn read_object(index: usize, node: Node<'_, '_>, mesh: Node<'_, '_>, archive: &mut Archive, budget: usize) -> Option<Object> {
    let file = value(mesh, &["DateiName"])?;
    if file.len() > 180 || !file.to_ascii_lowercase().ends_with(".x") || file.contains(['/', '\\', ':']) {
        return None;
    }
    let standard = path_node(node, &["StandardDaten"])?;
    let native_azimuth = number::<f64>(standard, &["Rotation", "AzimutWinkel"])?;
    let zenith = number::<f64>(standard, &["Rotation", "ZenitWinkel"])?;
    let roll = number::<f64>(standard, &["Rotation", "RollWinkel"])?;
    if !(-360.001..=360.001).contains(&native_azimuth) || zenith.abs() > 0.001 || roll.abs() > 0.001 { return None; }
    let azimuth = native_azimuth.rem_euclid(360.0);
    // Both supported mesh producers use either no rotation or a half turn.
    // Other rotations need a verified pivot and handedness.
    let invert = if azimuth.abs() < 0.001 || (azimuth - 360.0).abs() < 0.001 { false }
        else if (azimuth - 180.0).abs() < 0.001 { true } else { return None; };
    let position = xyz(standard, &["Position"])?;
    let extent = xyz(standard, &["Skalierung"])?;
    let member_name = format!("Visu3D/MeshFiles/{file}");
    let mesh_bytes = member(archive, &member_name, MAX_MESH_BYTES).ok()??;
    let (vertices, faces) = directx_mesh(&mesh_bytes, budget)?;
    let (min, max) = vertices.iter().fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(mut min, mut max), v| {
        for i in 0..3 { min[i] = min[i].min(v[i]); max[i] = max[i].max(v[i]); }
        (min, max)
    });
    // The archived dimensions match raw bounding-box extents. Position is
    // applied to the translated minimum, including the source survey datum.
    if (0..3).any(|i| (extent[i] - (max[i] - min[i])).abs() > extent[i].abs().max(1.0) * 0.0001) {
        return None;
    }
    let world: Vec<_> = vertices.iter().map(|v| {
        let mut local = sub(*v, min);
        if invert { local[0] = -local[0]; local[2] = -local[2]; }
        enu(add(position, local))
    }).collect();
    let mesh = faces.into_iter().filter_map(|[a, b, c]| {
        let (a, b, c) = (world[a], world[b], world[c]);
        let normal = cross(sub(b, a), sub(c, a));
        (dot(normal, normal) > 1e-16)
            .then_some(MeshFace { outer: vec![a, c, b], holes: vec![] })
    }).collect::<Vec<_>>();
    if mesh.is_empty() { return None; }
    let external_id = normalized_guid(value(standard, &["SignaturXML"])?);
    Some(Object {
        id: format!("visual3d-mesh-{index}"), name: value(standard, &["Bezeichnung"])?.to_owned(),
        layer: "default".into(), kind: Kind::Mesh { faces: mesh, opacity: 1.0 }, hidden: false,
        properties: BTreeMap::new(),
        source: Some(Source { format: "pvsol_visual3d".into(), file: member_name, external_id, local_geometry: false }),
    })
}

fn directx_mesh(bytes: &[u8], budget: usize) -> Option<(Vec<V3>, Vec<[usize; 3]>)> {
    const HEADER: &[u8] = b"xof 0303bin 0032";
    // NAME, length=4, Mesh, open-brace. Template markers are distinct.
    const MESH: &[u8] = b"\x01\x00\x04\x00\x00\x00Mesh\x0a\x00";
    let bytes = bytes.strip_prefix(HEADER)?;
    let mut found = bytes.windows(MESH.len()).enumerate().filter(|(_, at)| *at == MESH);
    let (start, _) = found.next()?;
    if found.next().is_some() { return None; }
    let mut reader = MeshReader(bytes.get(start + MESH.len()..)?);
    // One DWORD in the first INTEGER_LIST gives the vertex count.
    if reader.u16()? != 6 || reader.u32()? != 1 { return None; }
    let vertex_count = reader.u32()? as usize;
    if !(3..=100_000).contains(&vertex_count) || reader.u16()? != 7 || reader.u32()? as usize != vertex_count * 3 {
        return None;
    }
    let vertices = (0..vertex_count).map(|_| Some([reader.float()?, reader.float()?, reader.float()?]))
        .collect::<Option<Vec<_>>>()?;
    if reader.u16()? != 6 { return None; }
    let values = reader.u32()? as usize;
    let face_count = reader.u32()? as usize;
    if face_count == 0 || face_count > budget.min(MAX_TRIANGLES) || values != face_count * 4 + 1 { return None; }
    let faces = (0..face_count).map(|_| {
        if reader.u32()? != 3 { return None; }
        let indices = [reader.u32()? as usize, reader.u32()? as usize, reader.u32()? as usize];
        indices.iter().all(|&i| i < vertex_count).then_some(indices)
    }).collect::<Option<Vec<_>>>()?;
    Some((vertices, faces))
}

struct MeshReader<'a>(&'a [u8]);

impl MeshReader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_at_checked(N)?;
        self.0 = rest;
        head.try_into().ok()
    }
    fn u16(&mut self) -> Option<u16> { Some(u16::from_le_bytes(self.take()?)) }
    fn u32(&mut self) -> Option<u32> { Some(u32::from_le_bytes(self.take()?)) }
    fn float(&mut self) -> Option<f64> {
        let n = f64::from(f32::from_le_bytes(self.take()?));
        (-1e7..=1e7).contains(&n).then_some(n)
    }
}

struct Plane {
    origin: V3,
    u: V3,
    v: V3,
    normal: V3,
}

impl Plane {
    fn read(node: Node<'_, '_>) -> Option<Self> {
        let origin = enu(xyz(node, &["AufVektor"])?);
        let u = enu(xyz(node, &["BasisX"])?);
        let v = enu(xyz(node, &["BasisY"])?);
        if [u, v].iter().any(|&a| (dot(a, a).sqrt() - 1.0).abs() > 0.001) || dot(u, v).abs() > 0.001 { return None; }
        let mut normal = norm(cross(u, v));
        if normal[2] < 0.0 { normal = scale(normal, -1.0); }
        (normal[2] > 0.1).then_some(Self { origin, u, v, normal })
    }
    fn point(&self, [x, y]: P2) -> V3 { add(self.origin, add(scale(self.u, x), scale(self.v, y))) }
}

struct SavedPanel {
    corners: [V3; 4],
    center: P2,
    portrait: bool,
}

fn read_field(project: &Project, root: Node<'_, '_>, scene: Node<'_, '_>, carrier: Node<'_, '_>, archive: &mut Archive) -> Option<(Object, ModuleField)> {
    let [array] = project.arrays.as_slice() else { return None; };
    let area = path_node(root, &["PVModule", "Modulfelder"])?;
    let area_id = normalized_guid(value(area, &["ModulfeldID"])?);
    let catalog_module = project.library.module(&array.module_id.value)?;
    let length = catalog_module.length_m.max(catalog_module.width_m);
    let width = catalog_module.length_m.min(catalog_module.width_m);
    if !(0.1..=5.0).contains(&width) || !(width..=5.0).contains(&length) { return None; }
    let installed = *derive::modules_by_array(&project.electrical).get(&array.id)? as usize;
    if !(1..=1200).contains(&installed) { return None; }
    let carrier_geometry = path_node(carrier, &["SpezifDaten","BelegFL"])?;
    let plane = Plane::read(carrier_geometry)?;
    let mut outline = children(carrier_geometry, "EckPunkte2d").map(xy).collect::<Option<Vec<_>>>()?;
    if !(3..=512).contains(&outline.len()) || solarcraft_geometry::poly::ring_area(&outline).abs() < 0.1 { return None; }
    if dot(cross(plane.u, plane.v), plane.normal) * solarcraft_geometry::poly::ring_area(&outline) < 0.0 { outline.reverse(); }
    let wiring = member(archive, WIRING, 8 * 1024 * 1024).ok()??;
    let wiring = document(&wiring, WIRING).ok()?;
    let wiring = wiring.root_element();
    if !wiring.has_tag_name("Modulverschaltung_PVSol") { return None; }
    let refs: Vec<_> = children(wiring, "ReferenzModul").collect();
    let [reference] = refs.as_slice() else { return None; };
    if normalized_guid(value(*reference, &["GUID_XML"])?) != area_id || number::<usize>(*reference, &["ModulAnzahl"])? != installed { return None; }
    let azimuth = number::<f64>(*reference, &["AusrichtungStatisch", "Azimut"])?;
    let tilt = number::<f64>(*reference, &["AusrichtungStatisch", "Neigung"])?;
    if !(-0.01..=std::f64::consts::TAU + 0.01).contains(&azimuth) || !(0.0..=std::f64::consts::FRAC_PI_2).contains(&tilt) { return None; }
    let normal = [tilt.sin() * azimuth.sin(), tilt.sin() * azimuth.cos(), tilt.cos()];
    if dot(normal, plane.normal) < 0.85 { return None; }

    let standoff = mount_clearance(scene, archive).unwrap_or(0.02);
    let plan_bytes = member(archive, STRING_PLAN, 8 * 1024 * 1024).ok()??;
    let plan = document(&plan_bytes, STRING_PLAN).ok()?;
    let plan = plan.root_element();
    if !plan.has_tag_name("PlanData") { return None; }
    let layers: HashMap<_, _> = children(plan, "Layers").filter_map(|n| Some((value(n,&["Name"])?, value(n,&["Id"])?))).collect();
    let module_layer = *layers.get("MODULES")?;
    let label_layer = *layers.get("CONFIGURATION")?;
    let mut panels = Vec::new();
    let mut labels = BTreeMap::new();
    let mut alternate_labels = HashSet::new();
    for element in children(plan, "Elements") {
        match value(element, &["LayerId"])? {
            layer if layer == module_layer => {
                if let Some(panel) = read_panel(element, &plane, normal, width, length, standoff) { panels.push(panel); }
            }
            layer if layer == label_layer => {
                if let Some((id, position)) = read_label(element) {
                    if labels.insert(id, position).is_some() { alternate_labels.insert(id); }
                }
            }
            _ => {}
        }
    }
    if panels.len() != installed || labels.len() != installed || !alternate_labels.is_empty() { return None; }
    let saved_guids: HashMap<_, _> = scene.descendants().filter_map(|n| {
        (n.has_tag_name("Modul") || n.has_tag_name("Module")).then(|| Some((
            normalized_guid(value(n, &["SignaturXML"])?), number::<usize>(n, &["IndexInAbschGrBin"])?,
        ))).flatten()
    }).collect();

    // CONFIGNUMBER blocks use a common label origin relative to panel centers.
    // Equal-sized sets let us recover the shared offset without assuming order.
    let offset: P2 = std::array::from_fn(|i| {
        (labels.values().map(|p| p[i]).sum::<f64>() - panels.iter().map(|p| p.center[i]).sum::<f64>())
            / installed as f64
    });
    let mut nearest = HashSet::new();
    let mut modules = Vec::with_capacity(installed);
    let mut strings = vec![];
    let wire_root = path_node(wiring, &["Verschaltung"])?;
    for node in children(wire_root, "TeilAnlage").flat_map(|part| children(part, "WechselRichter")) {
        let inverter = number::<u32>(node, &["AllgDaten", "Nummer"])?;
        for tracker in children(node, "MPP_Tracker") {
            let input = number::<u32>(tracker, &["AllgDaten", "Nummer"])?;
            for (index, string) in children(tracker, "Strang").enumerate() {
                if children(string, "SubStrang").next().is_some()
                    || normalized_guid(value(string, &["AllgDaten", "GUID_XML"])?) != area_id { return None; }
                let string_number = index as u32 + 1;
                let route_index = strings.len() as u32;
                let first = modules.len();
                let mut ids = vec![];
                for (index, node) in children(string, "Modul").enumerate() {
                    let id = normalized_guid(value(node, &["Zugehoerigkeit", "GUID_Modul"])?);
                    let slot = number::<usize>(node, &["AbschGrad", "Index_SchattenArray"])?;
                    if saved_guids.get(&id) != Some(&slot) { return None; }
                    let label = labels.get(&[inverter, input, string_number, index as u32 + 1])?;
                    let candidate = match_panel([label[0] - offset[0], label[1] - offset[1]], &panels, width)?;
                    if !nearest.insert(candidate) { return None; }
                    let panel = &panels[candidate];
                    let id = format!("visual3d-field-1/{id}");
                    ids.push(id.clone());
                    modules.push(PlacedModule { id, corners: panel.corners, normal,
                        row: 0, col: slot as u32, portrait: panel.portrait, string: Some(route_index) });
                }
                if ids.is_empty() { return None; }
                let jumper_m = modules[first..].windows(2).map(|w| dist(w[0].center(), w[1].center())).sum();
                strings.push(StringRoute { id: format!("visual3d-field-1/route-{inverter}-{input}-{string_number}"),
                    index: route_index, modules: ids, jumper_m, home_run_m: 0.0 });
            }
        }
    }
    if modules.len() != installed { return None; }
    let mounted = Object { id: "visual3d-plane-1".into(),
        name: value(carrier, &["StandardDaten", "Bezeichnung"])?.to_owned(),
        layer: "default".into(), hidden: true, properties: BTreeMap::new(),
        kind: Kind::Mesh { faces: vec![MeshFace { outer: outline.into_iter().map(|p| plane.point(p)).collect(), holes: vec![] }], opacity: 0.0 },
        source: Some(Source { format: "pvsol_visual3d".into(), file: GEOMETRY.into(),
            external_id: area_id.clone(), local_geometry: false }),
    };
    let orientation = if modules.iter().filter(|m| m.portrait).count() * 2 >= installed { Orientation::Portrait } else { Orientation::Landscape };
    let field = ModuleField { id: "visual3d-field-1".into(), name: array.name.value.clone(), array_id: Some(array.id.clone()),
        mount: Mount::Roof { face: format!("{}/f0", mounted.id), standoff },
        module: ModuleDims { length, width, bypass_diodes: catalog_module.bypass_diodes.unwrap_or(3) },
        layout: FieldLayout { orientation, ..FieldLayout::default() }, modules, strings, inverter_at: None,
        source: Some(Source { format: "pvsol_visual3d".into(), file: STRING_PLAN.into(),
            external_id: area_id, local_geometry: false }),
    };
    Some((mounted, field))
}

fn mount_clearance(scene: Node<'_, '_>, archive: &mut Archive) -> Option<f64> {
    let systems: HashSet<_> = scene.descendants().filter(|n| n.has_tag_name("ModulreihenFormation"))
        .filter_map(|n| value(n, &["MoSys_GUID"]).map(normalized_guid)).collect();
    let id = systems.iter().next().filter(|_| systems.len() == 1)?;
    let bytes = member(archive, "Visu3D/MoSys.xml", 2 * 1024 * 1024).ok()??;
    let doc = document(&bytes, "Visu3D/MoSys.xml").ok()?;
    let node = children(doc.root_element(), "MoSys").find(|n| value(*n,&["GUID"]).map(normalized_guid).as_ref() == Some(id))?;
    number::<f64>(node, &["HoeheUnterkante"]).filter(|h| (0.0..=20.0).contains(h))
}

fn read_label(element: Node<'_, '_>) -> Option<([u32; 4], P2)> {
    let block = path_node(element, &["SpecificData", "Block"])?;
    if value(block, &["BlockName"])? != "CONFIGNUMBER" { return None; }
    let inner = path_node(block, &["Elements"])?;
    let values = value(inner, &["SpecificData", "Text", "Text"])?;
    let parts = values.split('.').map(str::parse).collect::<std::result::Result<Vec<u32>, _>>().ok()?;
    let id: [u32;4] = parts.try_into().ok()?;
    if id.contains(&0) { return None; }
    let p = xy(path_node(inner, &["Position"] )?)?;
    let offset = xy(path_node(element, &["Position"] )?)?;
    Some((id, [p[0] + offset[0], p[1] + offset[1]]))
}

fn match_panel(label: P2, panels: &[SavedPanel], width: f64) -> Option<usize> {
    let mut ranks: Vec<_> = panels.iter().enumerate()
        .map(|(i, p)| ((p.center[0] - label[0]).hypot(p.center[1] - label[1]), i)).collect();
    ranks.sort_unstable_by(|a,b| a.0.total_cmp(&b.0));
    let &(best, i) = ranks.first()?;
    (best < width * 0.1 && ranks.get(1).is_none_or(|other| other.0 - best > width * 0.1)).then_some(i)
}

fn read_panel(node: Node<'_, '_>, plane: &Plane, normal: V3, width: f64, length: f64, clearance: f64) -> Option<SavedPanel> {
    let polygon = path_node(node, &["SpecificData","Polygon"])?;
    let points = children(polygon, "Vectors").map(xy).collect::<Option<Vec<_>>>()?;
    let [a, b, c, d, close] = points.as_slice() else { return None; };
    if (a[0] - close[0]).hypot(a[1] - close[1]) > 0.0001 { return None; }
    let origin = xy(path_node(node, &["Position"])?)?;
    let ring = [*a, *b, *c, *d].map(|p| [p[0] + origin[0], p[1] + origin[1]]);
    let center = std::array::from_fn(|i| ring.iter().map(|p| p[i]).sum::<f64>() / 4.0);
    let center3 = plane.point(center);
    let denom = dot(plane.normal, normal);
    let deltas = ring.map(|p| -dot(sub(plane.point(p), center3), normal) / denom);
    let lowest = deltas.into_iter().fold(f64::INFINITY, f64::min);
    let mut corners = std::array::from_fn(|i| add(plane.point(ring[i]), scale(plane.normal, deltas[i] - lowest + clearance)));
    let p = sub(corners[1], corners[0]);
    let q = sub(corners[2], corners[1]);
    if dot(norm(p), norm(q)).abs() > 0.02 { return None; }
    let mut sides = [dist(corners[0],corners[1]), dist(corners[1],corners[2]),
        dist(corners[2],corners[3]), dist(corners[3],corners[0])];
    sides.sort_unstable_by(f64::total_cmp);
    // Tall polygons on the module layer also describe the complete support
    // tables. Compare against the catalog dimensions before identifying panels.
    if sides.iter().zip([width, width, length, length]).any(|(&found, expected)| (found - expected).abs() > expected * 0.025) {
        return None;
    }
    if dot(cross(p, q), normal) < 0.0 { corners.reverse(); }
    let downslope = [normal[0], normal[1], 0.0];
    let bottom = (0..4).max_by(|&a,&b| {
        let at = |i| dot(downslope, add(corners[i], corners[(i+1)%4]));
        at(a).total_cmp(&at(b))
    })?;
    corners.rotate_left(bottom);
    let portrait = dist(corners[0], corners[1]) < dist(corners[1], corners[2]);
    Some(SavedPanel { corners, center, portrait })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_indexed_binary_x_mesh_and_rejects_invalid_faces() {
        let mut bytes = b"xof 0303bin 0032\x01\x00\x04\x00\x00\x00Mesh\x0a\x00".to_vec();
        bytes.extend_from_slice(&6_u16.to_le_bytes());
        for value in [1_u32, 3] { bytes.extend_from_slice(&value.to_le_bytes()); }
        bytes.extend_from_slice(&7_u16.to_le_bytes());
        bytes.extend_from_slice(&9_u32.to_le_bytes());
        for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] { bytes.extend_from_slice(&value.to_le_bytes()); }
        bytes.extend_from_slice(&6_u16.to_le_bytes());
        for value in [5_u32, 1, 3, 0, 1, 2] { bytes.extend_from_slice(&value.to_le_bytes()); }
        assert_eq!(directx_mesh(&bytes, 1).unwrap().1, vec![[0, 1, 2]]);
        let len = bytes.len();
        bytes[len - 4] = 3;
        assert!(directx_mesh(&bytes, 1).is_none());
    }
}
