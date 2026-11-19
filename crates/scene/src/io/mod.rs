// SPDX-License-Identifier: GPL-3.0-or-later
//! CAD/BIM/mesh exchange (INT-001..INT-012) with a fidelity report.
//!
//! | Format | Export | Import | SolarCraft identities |
//! |---|---|---|---|
//! | glTF 2.0 / GLB | yes | yes | node names + `extras.solarcraft` (lossless round trip) |
//! | Wavefront OBJ | yes | yes | `o` names + `# solarcraft` comments (lossless round trip) |
//! | DXF R12 (AutoCAD) | 3D faces + hidden-line plan + modules/strings + dimension texts | 3DFACE, closed (LW)POLYLINE footprints | layer names `SC3D_<id>_<ROLE>` |
//! | IFC4 (Revit, ArchiCAD, BlenderBIM) | spatial tree, IfcRoof/IfcWall/IfcSlab/IfcSolarDevice…, Pset_SolarCraft | tessellated, faceted-brep and extruded geometry | `Tag` + deterministic `GlobalId` + `Pset_SolarCraft` |
//! | IngeTrazo `.igz` (format 1) | groups with `uid`, layers, IFC tags, datum | groups, prototypes/instances, loose faces | group `uid` + `ext.solarcraft` |
//! | SolarCraft scene JSON | yes | yes | exact |
//!
//! Every exchange returns a [`Fidelity`] report: counts, units and axes,
//! georeference handling, the largest coordinate error introduced by the
//! file's number format, the identifier map, conversions and what the
//! format could not carry.
use crate::mesh::{self, Face, Role};
use crate::place::ModuleField;
use crate::{Edit, Georef, Kind, Object, Scene, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod dxf;
pub mod gltf;
pub mod ifc;
pub mod igz;
pub mod obj;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format { Glb, Gltf, Obj, Dxf, Ifc, Igz, Json }

impl Format {
    pub fn from_name(s: &str) -> Option<Format> {
        Some(match s.to_ascii_lowercase().trim_start_matches('.') {
            "glb" => Format::Glb, "gltf" => Format::Gltf, "obj" => Format::Obj, "dxf" => Format::Dxf, "ifc" => Format::Ifc, "igz" => Format::Igz, "json" | "scene" => Format::Json,
            _ => return None,
        })
    }
    pub fn from_path(p: &std::path::Path) -> Option<Format> { p.extension().and_then(|e| e.to_str()).and_then(Format::from_name) }
    pub fn name(self) -> &'static str { match self { Format::Glb => "glb", Format::Gltf => "gltf", Format::Obj => "obj", Format::Dxf => "dxf", Format::Ifc => "ifc", Format::Igz => "igz", Format::Json => "json" } }
}

/// Formats SolarCraft cannot read or write itself, with the reason (shown to users).
pub const NOT_SUPPORTED: &[(&str, &str)] = &[
    ("skp", "SketchUp .skp needs the SketchUp C SDK or OpenSKP; export glTF/OBJ/IFC from SketchUp, or use IngeTrazo (.igz) as the bridge."),
    ("rvt", "Revit .rvt/.rfa needs Autodesk's API; export IFC from Revit (File ▸ Export ▸ IFC)."),
    ("rfa", "Revit families need Autodesk's API; export IFC from Revit."),
    ("dwg", "DWG needs LibreDWG (planned as an optional helper); save as DXF in AutoCAD or LibreCAD."),
    ("dae", "COLLADA is planned; export glTF or OBJ meanwhile."),
    ("gml", "CityGML is planned; convert to glTF/OBJ meanwhile."),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct IdMap { pub scene_id: String, pub external_id: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Fidelity {
    pub format: String,
    pub direction: String,
    pub objects: usize,
    pub fields: usize,
    pub modules: usize,
    pub faces: usize,
    pub triangles: usize,
    pub units: String,
    pub axes: String,
    pub georeference: String,
    /// Largest coordinate change caused by the file's number format, metres.
    pub max_coordinate_error_m: f64,
    /// True when SolarCraft objects and fields come back exactly on import.
    pub solarcraft_metadata: bool,
    pub ids: Vec<IdMap>,
    pub conversions: Vec<String>,
    pub unsupported: Vec<String>,
}

impl Fidelity {
    fn new(format: Format, direction: &str) -> Self { Fidelity { format: format.name().into(), direction: direction.into(), units: "m".into(), ..Default::default() } }
    pub(crate) fn note(&mut self, s: impl Into<String>) { let s = s.into(); if !self.conversions.contains(&s) { self.conversions.push(s); } }
    pub(crate) fn lost(&mut self, s: impl Into<String>) { let s = s.into(); if !self.unsupported.contains(&s) { self.unsupported.push(s); } }
    pub(crate) fn err(&mut self, a: f64, b: f64) { let d = (a - b).abs(); if d > self.max_coordinate_error_m { self.max_coordinate_error_m = d; } }
}

/// What a file contributes to a scene: objects and fields (with their
/// external identities) plus the georeference it carries.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Imported { pub objects: Vec<Object>, pub fields: Vec<ModuleField>, pub georef: Option<Georef>, pub fidelity: Fidelity }

/// One exported unit: an object or a module field, with its faces by role.
pub(crate) struct Item<'a> {
    pub id: String,
    pub name: String,
    pub layer: String,
    pub ifc_class: &'static str,
    pub faces: Vec<Face>,
    pub object: Option<&'a Object>,
    pub field: Option<&'a ModuleField>,
}

pub(crate) fn module_faces(f: &ModuleField) -> Vec<Face> {
    f.modules.iter().map(|m| Face { id: m.id.clone(), object: f.id.clone(), role: Role::Imported, outer: m.corners.to_vec(), holes: vec![], opacity: 1.0 }).collect()
}

pub(crate) fn items(s: &Scene) -> Vec<Item<'_>> {
    let mut out: Vec<Item> = s.objects.iter().map(|o| Item {
        id: o.id.clone(), name: o.name.clone(), layer: o.layer.clone(),
        ifc_class: match o.kind { Kind::Building { .. } => "IfcBuilding", Kind::Tree { .. } | Kind::Terrain { .. } => "IfcGeographicElement", _ => "IfcBuildingElementProxy" },
        faces: mesh::faces(o), object: Some(o), field: None,
    }).collect();
    out.extend(s.fields.iter().map(|f| Item { id: f.id.clone(), name: f.name.clone(), layer: "pv".into(), ifc_class: "IfcSolarDevice", faces: module_faces(f), object: None, field: Some(f) }));
    out
}

pub(crate) fn role_name(f: &Face, field: bool) -> &'static str {
    if field { return "module"; }
    match f.role { Role::Roof => "roof", Role::Wall => "wall", Role::Floor => "floor", Role::Opening => "opening", Role::Obstacle => "obstacle", Role::Crown => "crown", Role::Trunk => "trunk", Role::Terrain => "terrain", Role::Imported => "imported" }
}

pub(crate) fn role_rgb(role: &str) -> [f64; 4] {
    match role {
        "roof" => [0.69, 0.36, 0.29, 1.0], "wall" => [0.89, 0.86, 0.82, 1.0], "floor" => [0.59, 0.59, 0.59, 1.0], "opening" => [0.59, 0.77, 0.86, 0.6],
        "obstacle" => [0.55, 0.55, 0.57, 1.0], "crown" => [0.31, 0.55, 0.28, 0.85], "trunk" => [0.43, 0.32, 0.22, 1.0], "terrain" => [0.67, 0.75, 0.55, 1.0],
        "module" => [0.09, 0.17, 0.31, 1.0], _ => [0.78, 0.78, 0.8, 1.0],
    }
}

/// Identifier safe for every format (letters, digits, `-`, `_`, `.`).
pub(crate) fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
    if t.is_empty() { "imported".into() } else { t }
}

pub(crate) fn mesh_object(id: String, name: String, file: &str, format: Format, external: String, faces: Vec<crate::MeshFace>, opacity: f64, layer: &str) -> Object {
    Object { id, name, layer: layer.into(), kind: Kind::Mesh { faces, opacity }, hidden: false, properties: Default::default(), source: Some(Source { format: format.name().into(), file: file.into(), external_id: external, local_geometry: false }) }
}

/// Write the scene in a format. Returns the file bytes and the fidelity report.
pub fn export(s: &Scene, format: Format) -> Result<(Vec<u8>, Fidelity), String> {
    let (bytes, mut fid) = match format {
        Format::Glb => gltf::export(s, true),
        Format::Gltf => gltf::export(s, false),
        Format::Obj => obj::export(s),
        Format::Dxf => dxf::export(s),
        Format::Ifc => ifc::export(s),
        Format::Igz => igz::export(s),
        Format::Json => {
            let mut f = Fidelity::new(Format::Json, "export");
            f.solarcraft_metadata = true;
            f.axes = "x east, y north, z up".into();
            f.georeference = "exact".into();
            (serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?, f)
        }
    };
    fid.objects = s.objects.len();
    fid.fields = s.fields.len();
    fid.modules = s.fields.iter().map(|f| f.modules.len()).sum();
    if fid.ids.is_empty() { fid.ids = s.objects.iter().map(|o| IdMap { scene_id: o.id.clone(), external_id: o.id.clone() }).chain(s.fields.iter().map(|f| IdMap { scene_id: f.id.clone(), external_id: f.id.clone() })).collect(); }
    Ok((bytes, fid))
}

/// Read a file. `name` is the file name (for provenance and relative resources);
/// `resource` loads companion files (glTF buffers) by relative path.
pub fn import(bytes: &[u8], format: Format, name: &str, options: &Value, resource: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Imported, String> {
    let mut imp = match format {
        Format::Glb | Format::Gltf => gltf::import(bytes, name, resource)?,
        Format::Obj => obj::import(bytes, name, options)?,
        Format::Dxf => dxf::import(bytes, name, options)?,
        Format::Ifc => ifc::import(bytes, name)?,
        Format::Igz => igz::import(bytes, name, options)?,
        Format::Json => {
            let s: Scene = serde_json::from_slice(bytes).map_err(|e| format!("scene JSON: {e}"))?;
            let mut f = Fidelity::new(Format::Json, "import");
            f.solarcraft_metadata = true;
            Imported { objects: s.objects, fields: s.fields, georef: Some(s.georef), fidelity: f }
        }
    };
    imp.fidelity.direction = "import".into();
    imp.fidelity.objects = imp.objects.len();
    imp.fidelity.fields = imp.fields.len();
    imp.fidelity.modules = imp.fields.iter().map(|f| f.modules.len()).sum();
    imp.fidelity.faces = imp.objects.iter().map(|o| mesh::faces(o).len()).sum();
    for o in &imp.objects { for c in o.kind_coords() { if !c.is_finite() { return Err(format!("{}: coordinates are not finite", o.id)); } } }
    Ok(imp)
}

impl Object {
    fn kind_coords(&self) -> Vec<f64> {
        match &self.kind {
            Kind::Mesh { faces, .. } => faces.iter().flat_map(|f| f.outer.iter().chain(f.holes.iter().flatten())).flat_map(|p| p.iter().copied()).collect(),
            Kind::Building { footprint, eave_height, base_z, .. } => footprint.iter().flatten().copied().chain([*eave_height, *base_z]).collect(),
            _ => vec![],
        }
    }
}

/// Turn an import into scene edits. New identities avoid collisions with the
/// scene. With `relink`, objects that came from the same file and external
/// identifier as an existing object update that object's geometry in place
/// (keeping its id, layer, properties and the fields placed on it), except when
/// its geometry was edited locally. Existing
/// objects from that file that are no longer in it are reported, not deleted.
pub fn plan(scene: &Scene, imp: &Imported, relink: bool) -> (Vec<Edit>, Vec<IdMap>, Vec<String>) {
    plan_with_source(scene, imp, relink, None)
}

/// Plan exchange edits with the local source document identity, when available.
pub fn plan_with_source(scene: &Scene, imp: &Imported, relink: bool, source_key: Option<&str>) -> (Vec<Edit>, Vec<IdMap>, Vec<String>) {
    let mut edits = vec![];
    let mut map = vec![];
    let mut notes = vec![];
    let mut taken: std::collections::HashSet<String> = scene.objects.iter().map(|o| o.id.clone()).chain(scene.fields.iter().map(|f| f.id.clone())).collect();
    let fresh = |want: &str, taken: &mut std::collections::HashSet<String>| -> String {
        let base = sanitize(want).replace('/', "_");
        let mut id = base.clone();
        let mut k = 2;
        while taken.contains(&id) { id = format!("{base}-{k}"); k += 1; }
        taken.insert(id.clone());
        id
    };
    let source_for = |object: &Object| source_key.map(|key| Source {
        format: imp.fidelity.format.clone(),
        file: key.to_owned(),
        external_id: object.source.as_ref().map_or_else(|| object.id.clone(), |origin| origin.external_id.clone()),
        local_geometry: false,
    }).or_else(|| object.source.clone());
    let mut seen = std::collections::HashSet::new();
    let mut host_ids = std::collections::HashMap::new();
    for o in &imp.objects {
        let origin = source_for(o);
        if relink {
            let find = |source: &Source| scene.objects.iter().find(|x| {
                x.source.as_ref().is_some_and(|s| s.file == source.file && s.external_id == source.external_id)
            });
            if let Some((source, old)) = origin.as_ref().and_then(|source| {
                find(source).or_else(|| o.source.as_ref().and_then(find)).map(|old| (source, old))
            }) {
                seen.insert(old.id.clone());
                host_ids.insert(o.id.clone(), old.id.clone());
                if old.kind != o.kind {
                    if old.source.as_ref().is_some_and(|source| source.local_geometry) {
                        notes.push(format!("{} differs from the source; retained locally edited geometry.", old.id));
                    } else {
                        edits.push(Edit::RelinkObject { id: old.id.clone(), kind: o.kind.clone(), source: source.clone() });
                        notes.push(format!("{} updated from {}", old.id, source.external_id));
                    }
                }
                map.push(IdMap { scene_id: old.id.clone(), external_id: source.external_id.clone() });
                continue;
            }
        }
        let id = fresh(&o.id, &mut taken);
        host_ids.insert(o.id.clone(), id.clone());
        map.push(IdMap { scene_id: id.clone(), external_id: o.id.clone() });
        edits.push(Edit::AddObject { id: Some(id), name: o.name.clone(), layer: Some(if o.layer.is_empty() { "imported".into() } else { o.layer.clone() }), kind: o.kind.clone(), properties: o.properties.clone(), source: origin });
    }
    if relink {
        let files: std::collections::HashSet<String> = imp.objects.iter().filter_map(|o| source_for(o).map(|s| s.file)).collect();
        for o in &scene.objects {
            if o.source.as_ref().is_some_and(|s| files.contains(&s.file)) && !seen.contains(&o.id) { notes.push(format!("{} is no longer in the source file; kept unchanged", o.id)); }
        }
    }
    for edit in &mut edits {
        let kind = match edit {
            Edit::AddObject { kind, .. } | Edit::RelinkObject { kind, .. } => kind,
            _ => continue,
        };
        if let Kind::Dormer { profile } = kind {
            if let Some(parent) = &mut profile.parent {
                if let Some(new_id) = host_ids.get(parent) {
                    *parent = new_id.clone();
                } else {
                    notes.push(format!("Detached a dormer whose parent {parent} was absent from the imported objects."));
                    profile.parent = None;
                }
            }
        }
    }
    let mut bound_arrays: std::collections::HashSet<_> = scene.fields.iter().filter_map(|field| field.array_id.clone()).collect();
    for f in &imp.fields {
        let source = source_key.map(|key| Source {
            format: imp.fidelity.format.clone(), file: key.to_owned(), external_id: f.id.clone(), local_geometry: false,
        }).or_else(|| f.source.clone());
        if relink {
            if let Some(existing) = scene.fields.iter().find(|field| {
                source.is_some() && field.source == source
            }) {
                map.push(IdMap { scene_id: existing.id.clone(), external_id: f.id.clone() });
                notes.push(format!("Retained existing module field {} and its SolarCraft placement.", existing.id));
                continue;
            }
        }
        let id = fresh(&f.id, &mut taken);
        map.push(IdMap { scene_id: id.clone(), external_id: f.id.clone() });
        let mount = match &f.mount {
            crate::place::Mount::Roof { face, standoff } => {
                // Follow the renamed carrier object.
                let (obj, rest) = face.split_once('/').unwrap_or((face, ""));
                let new_obj = host_ids.get(obj).map_or(obj, String::as_str);
                crate::place::Mount::Roof { face: format!("{new_obj}/{rest}"), standoff: *standoff }
            }
            m => m.clone(),
        };
        let array_id = f.array_id.clone().filter(|array_id| bound_arrays.insert(array_id.clone()));
        if array_id.is_none() && f.array_id.is_some() {
            notes.push(format!("Field {id} arrived unbound because its array already has a field."));
        }
        edits.push(Edit::AddField { id: Some(id.clone()), name: f.name.clone(), array_id, mount, module: f.module.clone(), layout: f.layout.clone(), source });
        if !f.modules.is_empty() {
            let mut strings = f.strings.clone();
            if strings.iter().any(|route| route.binding.is_some()) {
                for route in &mut strings { route.binding = None; }
                notes.push(format!("Field {id} needs new MPPT assignments in this project."));
            }
            edits.push(Edit::RestorePlacement { field: id, modules: f.modules.clone(), strings, inverter_at: f.inverter_at });
        }
    }
    (edits, map, notes)
}

#[cfg(test)]
mod tests;
