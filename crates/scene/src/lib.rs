// SPDX-License-Identifier: GPL-3.0-or-later
//! The canonical SolarCraft scene (feature rows LAY-004..LAY-020, INT-001..INT-012).
//!
//! One editable model holds the buildings, roofs, obstacles, trees, terrain
//! and imported meshes of a project, plus the module fields placed on them.
//! Every object, generated face, module and string has a stable identifier
//! (`bld-3`, `bld-3/roof-1`, `fld-7/m0012`, `fld-7/s2`), so the viewport,
//! the electrical design, the shading results and the CAD/BIM exchanges all
//! name the same thing the same way.
//!
//! Coordinates are metres in a local frame: x east, y north, z up, origin at
//! the project site. [`Georef`] ties the frame to the globe.
//!
//! The geometry kernels come from IngeTrazo (`solarcraft-ingetrazo`):
//! face triangulation with holes (`facegeom`), vertex welding (`weld`),
//! closed-solid and orientation checks (`core::orient`, `parity`) and
//! hidden-line removal for drawings (`hlr`).
#![allow(clippy::too_many_arguments, clippy::type_complexity, clippy::needless_range_loop)]
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod io;
pub mod mesh;
pub mod place;
pub mod shade;
pub mod rear;
pub mod view;

pub type P2 = [f64; 2];
pub type V3 = [f64; 3];

/// Scene schema version.
pub const SCENE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Georef {
    pub latitude: f64,
    pub longitude: f64,
    /// Elevation of the local origin above sea level, metres.
    pub elevation: f64,
    /// Clockwise angle from the scene's +y axis to true north, degrees.
    /// 0 when the scene is drawn north-up (the default).
    #[serde(default)]
    pub true_north_deg: f64,
}

impl Default for Georef {
    fn default() -> Self { Georef { latitude: 0.0, longitude: 0.0, elevation: 0.0, true_north_deg: 0.0 } }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
}

fn yes() -> bool { true }
fn one() -> u32 { SCENE_VERSION }
fn is_false(value: &bool) -> bool { !value }

/// Where an object came from, when it was imported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub format: String,
    pub file: String,
    /// Identifier of the object in the source file (name, GUID, handle).
    pub external_id: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_geometry: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Roof {
    Flat,
    /// Single plane. `facing_deg`: downslope direction, clockwise from north.
    Shed { pitch_deg: f64, facing_deg: f64 },
    /// Rectangular footprint; the ridge runs parallel to the first footprint edge.
    Gable { pitch_deg: f64 },
    /// Rectangular footprint; all four sides slope at `pitch_deg`.
    Hip { pitch_deg: f64 },
    /// Rectangular footprint; the two end slopes run to the specified ridge inset.
    HipWithSetback { pitch_deg: f64, ridge_setback_m: f64 },
}

/// Shed-roof dormer whose underside follows a steeper roof plane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dormer {
    pub origin: P2,
    pub base_z: f64,
    pub width: f64,
    pub front_height: f64,
    pub facing_deg: f64,
    pub base_pitch_deg: f64,
    pub roof_pitch_deg: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gable: Option<DormerGable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_clearance_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DormerGable {
    pub rise_m: f64,
    pub ridge_width_m: f64,
}

impl Dormer {
    pub fn depth(&self) -> Option<f64> {
        if !self.origin.iter().all(|v| v.is_finite()) || !self.base_z.is_finite()
            || !(0.001..=1000.0).contains(&self.width) || !(0.001..=1000.0).contains(&self.front_height)
            || !(0.0..360.0).contains(&self.facing_deg) || !(0.0..=75.0).contains(&self.base_pitch_deg)
            || self.host_clearance_m.is_some_and(|v| !(-0.05..=0.05).contains(&v)) {
            return None;
        }
        let pitch = self.base_pitch_deg.to_radians().tan();
        let depth = if let Some(gable) = self.gable {
            if !(0.001..=1000.0).contains(&gable.rise_m) || !(0.001..self.width).contains(&gable.ridge_width_m) {
                return None;
            }
            (self.front_height + gable.rise_m) / pitch
        } else {
            if !(0.0..self.base_pitch_deg).contains(&self.roof_pitch_deg) { return None; }
            self.front_height / (pitch - self.roof_pitch_deg.to_radians().tan())
        };
        (0.001..=1000.0).contains(&depth).then_some(depth)
    }

    pub fn footprint(&self, depth: f64) -> Vec<P2> {
        let corners = if let Some(gable) = self.gable {
            let inset = (self.width - gable.ridge_width_m) * 0.5;
            let eave_depth = self.front_height / self.base_pitch_deg.to_radians().tan();
            vec![[0.0, 0.0], [self.width, 0.0], [self.width, eave_depth],
                [self.width - inset, depth], [inset, depth], [0.0, eave_depth]]
        } else {
            vec![[0.0, 0.0], [self.width, 0.0], [self.width, depth], [0.0, depth]]
        };
        corners.into_iter().map(|point| self.point(point)).collect()
    }

    pub(crate) fn point(&self, [x, y]: P2) -> P2 {
        let (s, c) = self.facing_deg.to_radians().sin_cos();
        [self.origin[0] - c * x - s * y, self.origin[1] + s * x - c * y]
    }

    pub fn fit_on(&self, parent: &Object) -> Option<Self> {
        if !matches!(parent.kind, Kind::Building { .. }) { return None; }
        mesh::faces(parent).into_iter().filter(|face| face.role == mesh::Role::Roof).find_map(|face| {
            let (pitch, facing) = place::tilt_azimuth(mesh::normal(&face.outer));
            let daz = ((facing - self.facing_deg + 180.0).rem_euclid(360.0) - 180.0).abs();
            if daz > 90.0 { return None; }
            let roof: Vec<_> = face.outer.iter().map(|p| [p[0], p[1]]).collect();
            let mut fitted = self.clone();
            fitted.base_z = mesh::plane_z(&face.outer)?(self.origin) + self.host_clearance_m.unwrap_or(0.0);
            fitted.base_pitch_deg = pitch;
            fitted.facing_deg = facing;
            fitted.footprint(fitted.depth()?).into_iter()
                .all(|p| solarcraft_geometry::poly::point_in_ring(p, &roof)).then_some(fitted)
        })
    }
}

/// A planar face of an imported mesh: outer ring and holes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshFace {
    pub outer: Vec<V3>,
    #[serde(default)]
    pub holes: Vec<Vec<V3>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    /// Extruded footprint with a roof. `openings`: roof windows and other
    /// openings, as plan polygons; each becomes a hole in its roof face and
    /// a separate opening face, so the solid stays closed.
    Building {
        footprint: Vec<P2>,
        #[serde(default)]
        base_z: f64,
        eave_height: f64,
        roof: Roof,
        #[serde(default)]
        openings: Vec<Vec<P2>>,
    },
    Dormer { #[serde(flatten)] profile: Dormer },
    /// Box standing on `base_z` (chimney, dormer block, parapet, wall).
    Block { center: P2, base_z: f64, length: f64, width: f64, height: f64, #[serde(default)] rotation_deg: f64 },
    /// Vertical cylinder (pole, vent, silo).
    Cylinder { center: P2, base_z: f64, radius: f64, height: f64, #[serde(default = "segments")] segments: u32 },
    /// Tree: opaque trunk and a crown that lets `transparency` of the beam through.
    Tree {
        center: P2,
        #[serde(default)]
        base_z: f64,
        trunk_height: f64,
        trunk_radius: f64,
        crown_radius: f64,
        crown_height: f64,
        transparency: f64,
        #[serde(default = "segments")]
        segments: u32,
    },
    /// Height grid: `heights[j * nx + i]` at `origin + (i, j) · cell`.
    Terrain { origin: P2, cell: f64, nx: usize, ny: usize, heights: Vec<f64> },
    /// Faces imported from CAD/BIM/mesh files.
    Mesh { faces: Vec<MeshFace>, #[serde(default = "opaque")] opacity: f64 },
}

fn segments() -> u32 { 16 }
fn opaque() -> f64 { 1.0 }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub layer: String,
    pub kind: Kind,
    #[serde(default)]
    pub hidden: bool,
    /// BIM-style properties (classification, material, notes); exported as a property set.
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
    #[serde(default)]
    pub source: Option<Source>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Scene {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub georef: Georef,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub objects: Vec<Object>,
    #[serde(default)]
    pub fields: Vec<place::ModuleField>,
    /// Counter behind new identifiers; never reused, so deleted IDs stay dead.
    #[serde(default)]
    pub next_id: u64,
}

impl Scene {
    pub fn new(georef: Georef) -> Self {
        Scene { version: SCENE_VERSION, georef, layers: vec![Layer { name: "default".into(), visible: true, locked: false }], objects: vec![], fields: vec![], next_id: 1 }
    }
    pub fn object(&self, id: &str) -> Option<&Object> { self.objects.iter().find(|o| o.id == id) }
    pub fn field(&self, id: &str) -> Option<&place::ModuleField> { self.fields.iter().find(|f| f.id == id) }
    pub fn has_bound_modules(&self) -> bool {
        self.fields.iter().any(|field| field.array_id.is_some() && !field.modules.is_empty())
    }
    fn mint(&mut self, prefix: &str) -> String {
        let n = self.next_id.max(1);
        self.next_id = n + 1;
        format!("{prefix}-{n}")
    }
    /// Azimuth from true north of a direction given in scene coordinates.
    pub fn true_azimuth(&self, scene_az: f64) -> f64 { (scene_az + self.georef.true_north_deg).rem_euclid(360.0) }
    /// Tilt and true azimuth of a field's modules.
    pub fn field_orientation(&self, f: &place::ModuleField) -> Option<(f64, f64)> { f.orientation().map(|(t, a)| (t, self.true_azimuth(a))) }
    pub fn layer_visible(&self, name: &str) -> bool { self.layers.iter().find(|l| l.name == name).is_none_or(|l| l.visible) }
    fn layer_locked(&self, name: &str) -> bool { self.layers.iter().any(|l| l.name == name && l.locked) }
    /// Every generated face of every object (visible or not).
    pub fn faces(&self) -> Vec<mesh::Face> { self.objects.iter().flat_map(mesh::faces).collect() }
}

fn prefix(k: &Kind) -> &'static str {
    match k { Kind::Building { .. } => "bld", Kind::Dormer { .. } => "dormer", Kind::Block { .. } => "blk", Kind::Cylinder { .. } => "cyl", Kind::Tree { .. } => "tree", Kind::Terrain { .. } => "ter", Kind::Mesh { .. } => "mesh" }
}

/// A scene edit. A list of edits is applied as one transaction: all or nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    /// `id` may be given (imports keep identities); otherwise one is minted.
    AddObject { #[serde(default)] id: Option<String>, name: String, #[serde(default)] layer: Option<String>, kind: Kind, #[serde(default)] properties: BTreeMap<String, String>, #[serde(default)] source: Option<Source> },
    UpdateObject { id: String, #[serde(default)] name: Option<String>, #[serde(default)] layer: Option<String>, #[serde(default)] kind: Option<Kind>, #[serde(default)] hidden: Option<bool>, #[serde(default)] properties: Option<BTreeMap<String, String>> },
    RelinkObject { id: String, kind: Kind, source: Source },
    Move { id: String, dx: f64, dy: f64, #[serde(default)] dz: f64 },
    RemoveObject { id: String },
    SetGeoref { georef: Georef },
    SetLayer { name: String, #[serde(default)] visible: Option<bool>, #[serde(default)] locked: Option<bool> },
    AddField { #[serde(default)] id: Option<String>, name: String, #[serde(default)] array_id: Option<String>, mount: place::Mount, #[serde(default)] module: place::ModuleDims, #[serde(default)] layout: place::FieldLayout, #[serde(default)] source: Option<Source> },
    UpdateField { id: String, #[serde(default)] name: Option<String>, #[serde(default)] array_id: Option<Option<String>>, #[serde(default)] mount: Option<place::Mount>, #[serde(default)] module: Option<place::ModuleDims>, #[serde(default)] layout: Option<place::FieldLayout> },
    RemoveField { id: String },
    /// Fill the field's mount with modules (replaces previous placement and strings).
    PlaceModules { field: String },
    /// Remove single modules from a placement (manual editing).
    RemoveModules { field: String, modules: Vec<String> },
    /// Restore an exact placement and string routing (imports, copy/paste).
    /// Module and string identifiers are re-prefixed with the field id.
    RestorePlacement { field: String, modules: Vec<place::PlacedModule>, #[serde(default)] strings: Vec<place::StringRoute>, #[serde(default)] inverter_at: Option<V3> },
    /// Serpentine string routing with `modules_per_string` modules per string.
    RouteStrings { field: String, modules_per_string: u32, #[serde(default)] inverter_at: Option<V3> },
    BindStringRoutes { array: String, bindings: Vec<place::RouteAssignment> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditError { pub code: String, pub message: String, pub index: usize }

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "edit {}: {}: {}", self.index, self.code, self.message) }
}

fn err(code: &str, message: String, index: usize) -> EditError { EditError { code: code.into(), message, index } }

fn shift(k: &mut Kind, d: V3) {
    let mv = |p: &mut P2| { p[0] += d[0]; p[1] += d[1]; };
    match k {
        Kind::Building { footprint, base_z, openings, .. } => { footprint.iter_mut().for_each(mv); openings.iter_mut().flatten().for_each(mv); *base_z += d[2]; }
        Kind::Dormer { profile } => { mv(&mut profile.origin); profile.base_z += d[2]; }
        Kind::Block { center, base_z, .. } | Kind::Cylinder { center, base_z, .. } | Kind::Tree { center, base_z, .. } => { mv(center); *base_z += d[2]; }
        Kind::Terrain { origin, heights, .. } => { mv(origin); heights.iter_mut().for_each(|h| *h += d[2]); }
        Kind::Mesh { faces, .. } => for f in faces { for p in f.outer.iter_mut().chain(f.holes.iter_mut().flatten()) { for k in 0..3 { p[k] += d[k]; } } },
    }
}

fn translate_roof_fields(s: &mut Scene, hosts: &std::collections::HashSet<&str>, delta: V3, touched: &mut Vec<String>) {
    if delta == [0.0; 3] { return; }
    for field in &mut s.fields {
        if !field.mount.face().and_then(|f| f.split_once('/')).is_some_and(|(host, _)| hosts.contains(host)) {
            continue;
        }
        for module in &mut field.modules {
            module.corners = module.corners.map(|point| mesh::add(point, delta));
        }
        if let Some(inverter) = field.inverter_at {
            let centers: std::collections::HashMap<_, _> = field.modules.iter().map(|m| (m.id.as_str(), m.center())).collect();
            for route in &mut field.strings {
                if let Some((a, b)) = route.modules.first().zip(route.modules.last())
                    .and_then(|(a,b)| centers.get(a.as_str()).zip(centers.get(b.as_str()))) {
                    route.home_run_m = mesh::dist(*a, inverter) + mesh::dist(*b, inverter);
                }
            }
        }
        if let Some(source) = &mut field.source { source.local_geometry = true; }
        if !touched.contains(&field.id) { touched.push(field.id.clone()); }
    }
}

/// Apply a transaction of edits. Returns the new scene and the identifiers it
/// created or touched, or the first error (and the scene is left unchanged).
pub fn apply(scene: &Scene, edits: &[Edit]) -> Result<(Scene, Vec<String>), EditError> {
    let mut s = scene.clone();
    if s.layers.is_empty() { s.layers.push(Layer { name: "default".into(), visible: true, locked: false }); }
    let mut touched = vec![];
    for (i, e) in edits.iter().enumerate() {
        let locked = |s: &Scene, id: &str| s.object(id).is_some_and(|o| s.layer_locked(&o.layer));
        match e {
            Edit::AddObject { id, name, layer, kind, properties, source } => {
                let id = match id {
                    Some(x) if s.object(x).is_some() || s.field(x).is_some() => return Err(err("DUPLICATE_ID", format!("{x} already exists"), i)),
                    Some(x) if x.is_empty() || x.contains('/') => return Err(err("INVALID_ID", format!("'{x}': identifiers are non-empty and contain no '/'"), i)),
                    Some(x) => x.clone(),
                    None => s.mint(prefix(kind)),
                };
                let layer = layer.clone().unwrap_or_else(|| "default".into());
                if !s.layers.iter().any(|l| l.name == layer) { s.layers.push(Layer { name: layer.clone(), visible: true, locked: false }); }
                if s.layer_locked(&layer) { return Err(err("LAYER_LOCKED", format!("layer {layer} is locked"), i)); }
                touched.push(id.clone());
                s.objects.push(Object { id, name: name.clone(), layer, kind: kind.clone(), hidden: false, properties: properties.clone(), source: source.clone() });
            }
            Edit::UpdateObject { id, name, layer, kind, hidden, properties } => {
                if locked(&s, id) { return Err(err("LAYER_LOCKED", format!("{id} is on a locked layer"), i)); }
                if let Some(l) = layer { if !s.layers.iter().any(|x| &x.name == l) { s.layers.push(Layer { name: l.clone(), visible: true, locked: false }); } }
                let o = s.objects.iter_mut().find(|o| &o.id == id).ok_or_else(|| err("NOT_FOUND", format!("no object {id}"), i))?;
                if let Some(x) = name { o.name = x.clone(); }
                if let Some(x) = layer { o.layer = x.clone(); }
                if let Some(x) = kind {
                    if std::mem::discriminant(x) != std::mem::discriminant(&o.kind) { return Err(err("KIND_CHANGE", format!("{id}: the object type cannot change; remove and add instead"), i)); }
                    if *x != o.kind { if let Some(source) = &mut o.source { source.local_geometry = true; } }
                    o.kind = x.clone();
                }
                if let Some(x) = hidden { o.hidden = *x; }
                if let Some(x) = properties { o.properties = x.clone(); }
                touched.push(id.clone());
            }
            Edit::RelinkObject { id, kind, source } => {
                if locked(&s, id) { return Err(err("LAYER_LOCKED", format!("{id} is on a locked layer"), i)); }
                let o = s.objects.iter_mut().find(|o| &o.id == id).ok_or_else(|| err("NOT_FOUND", format!("no object {id}"), i))?;
                if o.source.as_ref().is_some_and(|s| s.local_geometry) {
                    return Err(err("RELINK_CONFLICT", format!("{id} contains local geometry edits"), i));
                }
                if std::mem::discriminant(kind) != std::mem::discriminant(&o.kind) { return Err(err("KIND_CHANGE", format!("{id}: the object type cannot change during relink"), i)); }
                o.kind = kind.clone();
                o.source = Some(source.clone());
                touched.push(id.clone());
            }
            Edit::Move { id, dx, dy, dz } => {
                if locked(&s, id) { return Err(err("LAYER_LOCKED", format!("{id} is on a locked layer"), i)); }
                let children: Vec<_> = s.objects.iter().filter(|o| matches!(&o.kind, Kind::Dormer { profile }
                    if profile.parent.as_deref() == Some(id.as_str()))).map(|o| o.id.clone()).collect();
                let owners: std::collections::HashSet<_> = children.iter().map(String::as_str).chain([id.as_str()]).collect();
                if let Some(child) = children.iter().find(|child| locked(&s, child)) {
                    return Err(err("LAYER_LOCKED", format!("{child} is on a locked layer"), i));
                }
                let o = s.objects.iter_mut().find(|o| &o.id == id).ok_or_else(|| err("NOT_FOUND", format!("no object {id}"), i))?;
                shift(&mut o.kind, [*dx, *dy, *dz]);
                if let Some(source) = &mut o.source { source.local_geometry |= *dx != 0.0 || *dy != 0.0 || *dz != 0.0; }
                for child in &children {
                    let o = s.objects.iter_mut().find(|o| &o.id == child).unwrap();
                    shift(&mut o.kind, [*dx, *dy, *dz]);
                    if let Some(source) = &mut o.source { source.local_geometry |= *dx != 0.0 || *dy != 0.0 || *dz != 0.0; }
                    touched.push(child.clone());
                }
                translate_roof_fields(&mut s, &owners, [*dx, *dy, *dz], &mut touched);
                touched.push(id.clone());
            }
            Edit::RemoveObject { id } => {
                if locked(&s, id) { return Err(err("LAYER_LOCKED", format!("{id} is on a locked layer"), i)); }
                if let Some(child) = s.objects.iter().find(|o| matches!(&o.kind, Kind::Dormer { profile }
                    if profile.parent.as_deref() == Some(id.as_str()))) {
                    return Err(err("IN_USE", format!("{id} carries dormer {}; remove or detach it first", child.id), i));
                }
                if let Some(f) = s.fields.iter().find(|f| f.mount.face().is_some_and(|x| x.split('/').next() == Some(id.as_str()))) {
                    return Err(err("IN_USE", format!("{id} carries module field {}; remove the field first", f.id), i));
                }
                let n = s.objects.len();
                s.objects.retain(|o| &o.id != id);
                if s.objects.len() == n { return Err(err("NOT_FOUND", format!("no object {id}"), i)); }
                touched.push(id.clone());
            }
            Edit::SetGeoref { georef } => {
                if !(-90.0..=90.0).contains(&georef.latitude) || !(-180.0..=180.0).contains(&georef.longitude) { return Err(err("GEOREF_RANGE", "latitude must be within ±90° and longitude within ±180°".into(), i)); }
                s.georef = georef.clone();
            }
            Edit::SetLayer { name, visible, locked } => {
                if !s.layers.iter().any(|l| &l.name == name) { s.layers.push(Layer { name: name.clone(), visible: true, locked: false }); }
                let l = s.layers.iter_mut().find(|l| &l.name == name).unwrap();
                if let Some(v) = visible { l.visible = *v; }
                if let Some(v) = locked { l.locked = *v; }
            }
            Edit::AddField { id, name, array_id, mount, module, layout, source } => {
                let id = match id {
                    Some(x) if s.object(x).is_some() || s.field(x).is_some() => return Err(err("DUPLICATE_ID", format!("{x} already exists"), i)),
                    Some(x) if x.is_empty() || x.contains('/') => return Err(err("INVALID_ID", format!("'{x}': identifiers are non-empty and contain no '/'"), i)),
                    Some(x) => x.clone(),
                    None => s.mint("fld"),
                };
                touched.push(id.clone());
                s.fields.push(place::ModuleField { id, name: name.clone(), array_id: array_id.clone(), mount: mount.clone(), module: module.clone(), layout: layout.clone(), modules: vec![], strings: vec![], inverter_at: None, source: source.clone() });
            }
            Edit::UpdateField { id, name, array_id, mount, module, layout } => {
                let f = s.fields.iter_mut().find(|f| &f.id == id).ok_or_else(|| err("NOT_FOUND", format!("no field {id}"), i))?;
                if let Some(x) = name { f.name = x.clone(); }
                if let Some(x) = array_id { f.array_id = x.clone(); }
                let geometry_changed = mount.is_some() || module.is_some() || layout.is_some();
                if let Some(x) = mount { f.mount = x.clone(); }
                if let Some(x) = module { f.module = x.clone(); }
                if let Some(x) = layout { f.layout = x.clone(); }
                if geometry_changed { f.modules.clear(); f.strings.clear(); }
                touched.push(id.clone());
            }
            Edit::RemoveField { id } => {
                let n = s.fields.len();
                s.fields.retain(|f| &f.id != id);
                if s.fields.len() == n { return Err(err("NOT_FOUND", format!("no field {id}"), i)); }
                touched.push(id.clone());
            }
            Edit::PlaceModules { field } => {
                let fi = s.fields.iter().position(|f| &f.id == field).ok_or_else(|| err("NOT_FOUND", format!("no field {field}"), i))?;
                let placed = place::place(&s, &s.fields[fi]).map_err(|m| err("PLACEMENT", m, i))?;
                let f = &mut s.fields[fi];
                f.modules = placed;
                f.strings.clear();
                touched.push(field.clone());
            }
            Edit::RemoveModules { field, modules } => {
                let f = s.fields.iter_mut().find(|f| &f.id == field).ok_or_else(|| err("NOT_FOUND", format!("no field {field}"), i))?;
                for m in modules { if !f.modules.iter().any(|x| &x.id == m) { return Err(err("NOT_FOUND", format!("no module {m} in {field}"), i)); } }
                f.modules.retain(|x| !modules.contains(&x.id));
                f.strings.clear();
                touched.push(field.clone());
            }
            Edit::RestorePlacement { field, modules, strings, inverter_at } => {
                let f = s.fields.iter_mut().find(|f| &f.id == field).ok_or_else(|| err("NOT_FOUND", format!("no field {field}"), i))?;
                let re = |id: &str| format!("{field}/{}", id.rsplit('/').next().unwrap_or(id));
                f.modules = modules.iter().map(|m| place::PlacedModule { id: re(&m.id), ..m.clone() }).collect();
                f.strings = strings.iter().map(|r| place::StringRoute { id: re(&r.id), modules: r.modules.iter().map(|m| re(m)).collect(), ..r.clone() }).collect();
                if f.strings.iter().flat_map(|r| r.modules.iter()).any(|m| !f.modules.iter().any(|x| &x.id == m)) { return Err(err("INVALID_ARGUMENT", format!("{field}: strings name modules that are not in the placement"), i)); }
                f.inverter_at = *inverter_at;
                touched.push(field.clone());
            }
            Edit::RouteStrings { field, modules_per_string, inverter_at } => {
                let f = s.fields.iter_mut().find(|f| &f.id == field).ok_or_else(|| err("NOT_FOUND", format!("no field {field}"), i))?;
                if *modules_per_string == 0 { return Err(err("INVALID_ARGUMENT", "modules_per_string must be at least 1".into(), i)); }
                if f.modules.is_empty() { return Err(err("NO_MODULES", format!("{field} has no modules; place modules first"), i)); }
                f.inverter_at = *inverter_at;
                place::route_strings(f, *modules_per_string);
                touched.push(field.clone());
            }
            Edit::BindStringRoutes { array, bindings } => {
                let mut proposed = std::collections::HashMap::new();
                let mut slots = std::collections::HashSet::new();
                for assignment in bindings {
                    if proposed.insert(assignment.route.as_str(), &assignment.binding).is_some()
                        || !slots.insert(&assignment.binding) {
                        return Err(err("MPPT_BINDING", "A route or electrical string copy was assigned twice".into(), i));
                    }
                }
                let fields: Vec<_> = s.fields.iter_mut().filter(|f| f.array_id.as_deref() == Some(array)).collect();
                if fields.is_empty() {
                    return Err(err("NOT_FOUND", format!("No fields feed array {array}"), i));
                }
                for field in fields {
                    for route in &mut field.strings {
                        route.binding = proposed.remove(route.id.as_str()).cloned();
                        if !bindings.is_empty() && route.binding.is_none() {
                            return Err(err("MPPT_BINDING", format!("Missing electrical assignment for {}", route.id), i));
                        }
                    }
                    touched.push(field.id.clone());
                }
                if !proposed.is_empty() {
                    return Err(err("MPPT_BINDING", format!("A route does not feed array {array}"), i));
                }
            }
        }
    }
    let attachments: Vec<_> = s.objects.iter().filter_map(|o| {
        let Kind::Dormer { profile } = &o.kind else { return None; };
        let parent = profile.parent.as_ref()?;
        (touched.contains(&o.id) || touched.contains(parent)).then_some((&o.id, profile, parent))
    }).map(|(id, profile, parent)| {
        let fitted = s.object(parent).and_then(|host| profile.fit_on(host))
            .ok_or_else(|| err("DORMER_PARENT", format!("{id}: the dormer must fit on a face of its parent roof {parent}"), edits.len().saturating_sub(1)))?;
        Ok((id.clone(), fitted))
    }).collect::<Result<_, EditError>>()?;
    for (id, fitted) in attachments {
        let object = s.objects.iter_mut().find(|o| o.id == id).unwrap();
        let Kind::Dormer { profile } = &object.kind else { unreachable!() };
        let dz = if (fitted.base_pitch_deg - profile.base_pitch_deg).abs() < 1e-9
            && (fitted.facing_deg - profile.facing_deg).abs() < 1e-9
        {
            fitted.base_z - profile.base_z
        } else { 0.0 };
        let kind = Kind::Dormer { profile: fitted };
        if object.kind != kind {
            if s.layers.iter().any(|layer| layer.name == object.layer && layer.locked) {
                return Err(err("LAYER_LOCKED", format!("{id} is on a locked layer"), edits.len().saturating_sub(1)));
            }
            object.kind = kind;
            if let Some(source) = &mut object.source { source.local_geometry = true; }
            if !touched.contains(&id) { touched.push(id.clone()); }
            translate_roof_fields(&mut s, &std::collections::HashSet::from([id.as_str()]), [0.0, 0.0, dz], &mut touched);
        }
    }
    Ok((s, touched))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    pub severity: String,
    pub code: String,
    pub subject: String,
    pub message: String,
}

/// Every validation code the scene emits (stable API).
pub const CODES: &[&str] = &["SCENE_DUPLICATE_ID", "SCENE_LAYER", "SCENE_FOOTPRINT", "SCENE_HEIGHT", "SCENE_PITCH", "SCENE_ROOF_SHAPE", "SCENE_OPENING_OUTSIDE", "SCENE_OPENING_SPLIT",
    "SCENE_SIZE", "SCENE_TRANSPARENCY", "SCENE_TERRAIN_SIZE", "SCENE_NOT_FINITE", "SCENE_OPEN_MESH", "SCENE_OPEN_SOLID", "SCENE_WINDING", "SCENE_VOLUME",
    "SCENE_FIELD_FACE", "SCENE_FIELD_TERRAIN", "SCENE_FIELD_EMPTY", "SCENE_MODULE_COLLISION", "SCENE_STRING_INCOMPLETE"];

/// The static form of a code from [`CODES`] (for callers that keep `&'static str` codes).
pub fn static_code(code: &str) -> &'static str { CODES.iter().copied().find(|c| *c == code).unwrap_or("SCENE") }

fn issue(sev: &str, code: &str, subject: &str, message: String) -> Issue { Issue { severity: sev.into(), code: code.into(), subject: subject.into(), message } }

fn simple_polygon(p: &[P2]) -> bool {
    use solarcraft_geometry::poly::segments_intersect;
    let n = p.len();
    for i in 0..n { for j in i + 1..n {
        if j == i + 1 || (i == 0 && j == n - 1) { continue; }
        if segments_intersect(p[i], p[(i + 1) % n], p[j], p[(j + 1) % n]) { return false; }
    } }
    true
}

/// Rectangle check for gable and hip roofs (4 corners, right angles within 0.5°).
pub fn is_rectangle(p: &[P2]) -> bool {
    if p.len() != 4 { return false; }
    (0..4).all(|i| {
        let (a, b, c) = (p[i], p[(i + 1) % 4], p[(i + 2) % 4]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        let (lu, lv) = ((u[0] * u[0] + u[1] * u[1]).sqrt(), (v[0] * v[0] + v[1] * v[1]).sqrt());
        lu > 1e-6 && lv > 1e-6 && ((u[0] * v[0] + u[1] * v[1]) / (lu * lv)).abs() < 0.5f64.to_radians().sin()
    })
}

/// Geometric validity of every object and field.
pub fn validate(s: &Scene) -> Vec<Issue> {
    use solarcraft_geometry::poly::{point_in_ring, ring_area};
    let mut out = vec![];
    let mut ids = std::collections::HashSet::new();
    for id in s.objects.iter().map(|o| &o.id).chain(s.fields.iter().map(|f| &f.id)) {
        if !ids.insert(id.clone()) { out.push(issue("error", "SCENE_DUPLICATE_ID", id, format!("{id} is used twice"))); }
    }
    for o in &s.objects {
        let id = o.id.as_str();
        if !s.layers.iter().any(|l| l.name == o.layer) && !o.layer.is_empty() { out.push(issue("warning", "SCENE_LAYER", id, format!("{id} is on unknown layer {}", o.layer))); }
        let mut solid = true;
        match &o.kind {
            Kind::Building { footprint, eave_height, roof, openings, .. } => {
                if footprint.len() < 3 || ring_area(footprint).abs() < 0.5 { out.push(issue("error", "SCENE_FOOTPRINT", id, format!("{id}: the footprint needs at least 3 corners and 0.5 m² of area"))); continue; }
                if !simple_polygon(footprint) { out.push(issue("error", "SCENE_FOOTPRINT", id, format!("{id}: the footprint crosses itself"))); continue; }
                if *eave_height <= 0.0 { out.push(issue("error", "SCENE_HEIGHT", id, format!("{id}: eave height must be positive"))); continue; }
                let pitch = match roof { Roof::Flat => 0.0, Roof::Shed { pitch_deg, .. } | Roof::Gable { pitch_deg }
                    | Roof::Hip { pitch_deg } | Roof::HipWithSetback { pitch_deg, .. } => *pitch_deg };
                if !(0.0..=75.0).contains(&pitch) { out.push(issue("error", "SCENE_PITCH", id, format!("{id}: roof pitch {pitch}° is outside 0–75°"))); continue; }
                if matches!(roof, Roof::Gable { .. } | Roof::Hip { .. } | Roof::HipWithSetback { .. }) && !is_rectangle(footprint) {
                    out.push(issue("error", "SCENE_ROOF_SHAPE", id, format!("{id}: gable and hip roofs need a rectangular footprint; use a shed or flat roof, or import the roof as a mesh")));
                    continue;
                }
                if let Roof::HipWithSetback { ridge_setback_m, .. } = roof {
                    let side = |a: P2, b: P2| (a[0] - b[0]).hypot(a[1] - b[1]);
                    let half_run = 0.5 * side(footprint[0], footprint[1]).max(side(footprint[1], footprint[2]));
                    if !(0.001..half_run).contains(ridge_setback_m) {
                        out.push(issue("error", "SCENE_ROOF_SHAPE", id, format!("{id}: ridge setback must be shorter than half the longer footprint edge")));
                        continue;
                    }
                }
                for (k, op) in openings.iter().enumerate() {
                    if !op.iter().all(|&p| point_in_ring(p, footprint)) { out.push(issue("error", "SCENE_OPENING_OUTSIDE", id, format!("{id}: roof opening {k} is not inside the footprint"))); }
                }
                let roofs: Vec<_> = mesh::faces(o).into_iter().filter(|f| f.role == mesh::Role::Roof).collect();
                for (k, op) in openings.iter().enumerate() {
                    let inside = roofs.iter().any(|f| { let plan: Vec<P2> = f.outer.iter().map(|p| [p[0], p[1]]).collect(); op.iter().all(|&p| point_in_ring(p, &plan)) });
                    if !inside { out.push(issue("error", "SCENE_OPENING_SPLIT", id, format!("{id}: roof opening {k} crosses a ridge or hip line; split it per roof face"))); }
                }
            }
            Kind::Dormer { profile } => {
                if profile.depth().is_none() {
                    out.push(issue("error", "SCENE_ROOF_SHAPE", id, format!("{id}: invalid dormer dimensions or roof slope")));
                    continue;
                }
                if profile.parent.as_ref().is_some_and(|id| s.object(id).and_then(|parent| profile.fit_on(parent))
                    .is_none_or(|fitted| (fitted.base_z - profile.base_z).abs() > 0.001
                        || (fitted.base_pitch_deg - profile.base_pitch_deg).abs() > 0.001
                        || (fitted.facing_deg - profile.facing_deg).abs() > 0.001)) {
                    out.push(issue("error", "SCENE_ROOF_SHAPE", id, format!("{id}: the dormer must fit on its parent roof")));
                }
            }
            Kind::Block { length, width, height, .. } => if *length <= 0.0 || *width <= 0.0 || *height <= 0.0 { out.push(issue("error", "SCENE_SIZE", id, format!("{id}: sizes must be positive"))); continue; },
            Kind::Cylinder { radius, height, segments, .. } => if *radius <= 0.0 || *height <= 0.0 || *segments < 3 { out.push(issue("error", "SCENE_SIZE", id, format!("{id}: radius and height must be positive, segments ≥ 3"))); continue; },
            Kind::Tree { trunk_height, trunk_radius, crown_radius, crown_height, transparency, segments, .. } => {
                if *trunk_height < 0.0 || *trunk_radius <= 0.0 || *crown_radius <= 0.0 || *crown_height <= 0.0 || *segments < 3 { out.push(issue("error", "SCENE_SIZE", id, format!("{id}: tree sizes must be positive"))); continue; }
                if !(0.0..=1.0).contains(transparency) { out.push(issue("error", "SCENE_TRANSPARENCY", id, format!("{id}: transparency must be between 0 and 1"))); }
            }
            Kind::Terrain { nx, ny, heights, cell, .. } => {
                if *nx < 2 || *ny < 2 || heights.len() != nx * ny || *cell <= 0.0 { out.push(issue("error", "SCENE_TERRAIN_SIZE", id, format!("{id}: terrain needs nx·ny = {} heights (got {}) and a positive cell", nx * ny, heights.len()))); }
                if heights.iter().any(|h| !h.is_finite()) { out.push(issue("error", "SCENE_NOT_FINITE", id, format!("{id}: terrain heights must be finite"))); }
                continue;
            }
            Kind::Mesh { faces, opacity } => {
                solid = false; // imported meshes may be open surfaces
                if faces.iter().flat_map(|f| f.outer.iter().chain(f.holes.iter().flatten())).any(|p| !p.iter().all(|c| c.is_finite())) { out.push(issue("error", "SCENE_NOT_FINITE", id, format!("{id}: imported coordinates must be finite"))); continue; }
                if !(0.0..=1.0).contains(opacity) { out.push(issue("error", "SCENE_TRANSPARENCY", id, format!("{id}: opacity must be between 0 and 1"))); }
                let c = mesh::check_solid(&mesh::faces(o));
                if c.boundary_edges > 0 { out.push(issue("info", "SCENE_OPEN_MESH", id, format!("{id}: imported mesh is an open surface ({} boundary edges); it shades but has no inside", c.boundary_edges))); }
            }
        }
        if solid {
            let c = mesh::check_solid(&mesh::faces(o));
            if c.boundary_edges > 0 { out.push(issue("error", "SCENE_OPEN_SOLID", id, format!("{id}: the solid is not closed ({} boundary edges)", c.boundary_edges))); }
            if c.incoherent_edges > 0 { out.push(issue("error", "SCENE_WINDING", id, format!("{id}: {} edges are wound inconsistently", c.incoherent_edges))); }
            if c.volume <= 0.0 { out.push(issue("error", "SCENE_VOLUME", id, format!("{id}: the solid has non-positive volume ({:.3} m³); faces point inward", c.volume))); }
        }
    }
    for f in &s.fields { out.extend(place::validate_field(s, f)); }
    out
}

#[cfg(test)]
mod tests;
