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
    Object { id, name, layer: layer.into(), kind: Kind::Mesh { faces, opacity }, hidden: false, properties: Default::default(), source: Some(Source { format: format.name().into(), file: file.into(), external_id: external }) }
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
/// (keeping its id, layer, properties and the fields placed on it); existing
/// objects from that file that are no longer in it are reported, not deleted.
pub fn plan(scene: &Scene, imp: &Imported, relink: bool) -> (Vec<Edit>, Vec<IdMap>, Vec<String>) {
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
    let mut seen = std::collections::HashSet::new();
    for o in &imp.objects {
        let ext = o.source.as_ref().map(|s| (s.file.clone(), s.external_id.clone()));
        if relink {
            if let Some(old) = scene.objects.iter().find(|x| x.source.as_ref().map(|s| (s.file.clone(), s.external_id.clone())) == ext && ext.is_some()) {
                seen.insert(old.id.clone());
                if old.kind != o.kind { edits.push(Edit::UpdateObject { id: old.id.clone(), name: None, layer: None, kind: Some(o.kind.clone()), hidden: None, properties: None }); notes.push(format!("{} updated from {}", old.id, ext.as_ref().unwrap().1)); }
                map.push(IdMap { scene_id: old.id.clone(), external_id: ext.unwrap().1 });
                continue;
            }
        }
        let id = fresh(&o.id, &mut taken);
        map.push(IdMap { scene_id: id.clone(), external_id: o.source.as_ref().map_or(o.id.clone(), |s| s.external_id.clone()) });
        edits.push(Edit::AddObject { id: Some(id), name: o.name.clone(), layer: Some(if o.layer.is_empty() { "imported".into() } else { o.layer.clone() }), kind: o.kind.clone(), properties: o.properties.clone(), source: o.source.clone() });
    }
    if relink {
        let files: std::collections::HashSet<String> = imp.objects.iter().filter_map(|o| o.source.as_ref().map(|s| s.file.clone())).collect();
        for o in &scene.objects {
            if o.source.as_ref().is_some_and(|s| files.contains(&s.file)) && !seen.contains(&o.id) { notes.push(format!("{} is no longer in the source file; kept unchanged", o.id)); }
        }
    }
    for f in &imp.fields {
        let id = fresh(&f.id, &mut taken);
        map.push(IdMap { scene_id: id.clone(), external_id: f.id.clone() });
        let mount = match &f.mount {
            crate::place::Mount::Roof { face, standoff } => {
                // Follow the renamed carrier object.
                let (obj, rest) = face.split_once('/').unwrap_or((face, ""));
                let new_obj = map.iter().find(|m| m.external_id == obj || m.scene_id == obj).map_or(obj.to_string(), |m| m.scene_id.clone());
                crate::place::Mount::Roof { face: format!("{new_obj}/{rest}"), standoff: *standoff }
            }
            m => m.clone(),
        };
        edits.push(Edit::AddField { id: Some(id.clone()), name: f.name.clone(), array_id: f.array_id.clone(), mount, module: f.module.clone(), layout: f.layout.clone() });
        if !f.modules.is_empty() { edits.push(Edit::RestorePlacement { field: id, modules: f.modules.clone(), strings: f.strings.clone(), inverter_at: f.inverter_at }); }
    }
    (edits, map, notes)
}

#[cfg(test)]
mod tests;
