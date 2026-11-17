// SPDX-License-Identifier: GPL-3.0-or-later
//! Module fields: placement on roof faces and ground racks, manual removal,
//! serpentine string routing with cable lengths (LAY-008, LAY-012, LAY-019).
use crate::mesh::{self, add, cross, dist, dot, norm, scale, sub, Face, Role};
use crate::{Issue, Kind, Scene, P2, V3};
use serde::{Deserialize, Serialize};
use solarcraft_geometry::layout::{fill, LayoutParams, Orientation, RoofFace};
use solarcraft_geometry::poly::point_in_ring;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Mount {
    /// Flush on a roof face (`bld-1/roof-0`), `standoff` metres above it.
    Roof { face: String, #[serde(default = "standoff")] standoff: f64 },
    /// Ground racks in rows. Tables hold `modules_up` modules up the slope
    /// (portrait) by `modules_across` along the row. `terrain`: object whose
    /// heights the racks follow.
    Ground {
        origin: P2,
        azimuth_deg: f64,
        tilt_deg: f64,
        rows: u32,
        tables_per_row: u32,
        modules_across: u32,
        modules_up: u32,
        row_pitch: f64,
        #[serde(default = "table_gap")]
        table_gap: f64,
        #[serde(default = "clearance")]
        clearance: f64,
        #[serde(default)]
        terrain: Option<String>,
    },
}

fn standoff() -> f64 { 0.1 }
fn table_gap() -> f64 { 0.5 }
fn clearance() -> f64 { 0.8 }

impl Mount {
    pub fn face(&self) -> Option<&str> { match self { Mount::Roof { face, .. } => Some(face), _ => None } }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDims {
    /// Long and short side, metres.
    pub length: f64,
    pub width: f64,
    /// Bypass-diode substrings; each runs along the long side.
    #[serde(default = "three")]
    pub bypass_diodes: u32,
}
fn three() -> u32 { 3 }
impl Default for ModuleDims { fn default() -> Self { ModuleDims { length: 1.722, width: 1.134, bypass_diodes: 3 } } }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldLayout {
    pub orientation: Orientation,
    pub gap_x: f64,
    pub gap_y: f64,
    pub setback: f64,
    pub obstacle_clearance: f64,
}
impl Default for FieldLayout { fn default() -> Self { FieldLayout { orientation: Orientation::Best, gap_x: 0.02, gap_y: 0.02, setback: 0.4, obstacle_clearance: 0.3 } } }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacedModule {
    pub id: String,
    /// Corners counter-clockwise seen from the front: bottom-left, bottom-right, top-right, top-left.
    pub corners: [V3; 4],
    pub normal: V3,
    pub row: u32,
    pub col: u32,
    /// True when the long side runs up the slope (portrait).
    pub portrait: bool,
    #[serde(default)]
    pub string: Option<u32>,
}

impl PlacedModule {
    pub fn center(&self) -> V3 { scale(self.corners.iter().fold([0.0; 3], |a, &c| add(a, c)), 0.25) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StringRoute {
    pub id: String,
    pub index: u32,
    pub modules: Vec<String>,
    /// Module-to-module jumpers along the string, metres.
    pub jumper_m: f64,
    /// Both conductors from the string ends to the inverter, metres (0 without an inverter location).
    pub home_run_m: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleField {
    pub id: String,
    pub name: String,
    /// Electrical array this field feeds (its POA, shading and module count).
    #[serde(default)]
    pub array_id: Option<String>,
    pub mount: Mount,
    #[serde(default)]
    pub module: ModuleDims,
    #[serde(default)]
    pub layout: FieldLayout,
    #[serde(default)]
    pub modules: Vec<PlacedModule>,
    #[serde(default)]
    pub strings: Vec<StringRoute>,
    #[serde(default)]
    pub inverter_at: Option<V3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::Source>,
}

impl ModuleField {
    /// Mean tilt and azimuth (clockwise from north) of the placed modules.
    pub fn orientation(&self) -> Option<(f64, f64)> {
        if self.modules.is_empty() { return None; }
        let n = norm(self.modules.iter().fold([0.0; 3], |a, m| add(a, m.normal)));
        Some(tilt_azimuth(n))
    }
}

pub fn tilt_azimuth(n: V3) -> (f64, f64) {
    let tilt = n[2].clamp(-1.0, 1.0).acos().to_degrees();
    let az = if n[0].hypot(n[1]) < 1e-9 { 180.0 } else { n[0].atan2(n[1]).to_degrees().rem_euclid(360.0) };
    (tilt, az)
}

/// In-plane frame of a face: `u` horizontal along the contour, `v` up the slope.
pub struct Frame { pub o: V3, pub u: V3, pub v: V3, pub n: V3 }

impl Frame {
    pub fn of(f: &Face) -> Frame {
        let n = mesh::normal(&f.outer);
        let h = [n[0], n[1], 0.0];
        let u = if dot(h, h).sqrt() < 1e-6 { [1.0, 0.0, 0.0] } else { norm(cross([0.0, 0.0, 1.0], n)) };
        let v = cross(n, u);
        Frame { o: f.outer[0], u, v, n }
    }
    pub fn to2(&self, p: V3) -> P2 { let d = sub(p, self.o); [dot(d, self.u), dot(d, self.v)] }
    pub fn to3(&self, p: P2, lift: f64) -> V3 { add(add(self.o, add(scale(self.u, p[0]), scale(self.v, p[1]))), scale(self.n, lift)) }
}

/// Plan footprint (convex hull of the vertices) of objects that stand on a
/// face, as keep-out polygons in the face frame.
fn keepouts(scene: &Scene, roof: &Face, fr: &Frame) -> Vec<Vec<P2>> {
    let Some(z) = mesh::plane_z(&roof.outer) else { return vec![] };
    let plan: Vec<P2> = roof.outer.iter().map(|p| [p[0], p[1]]).collect();
    let mut out = vec![];
    for o in &scene.objects {
        if o.hidden || roof.object == o.id || matches!(o.kind, Kind::Terrain { .. }) { continue; }
        let pts: Vec<V3> = mesh::faces(o).iter().flat_map(|f| f.outer.clone()).collect();
        let on = pts.iter().any(|p| point_in_ring([p[0], p[1]], &plan) && p[2] > z([p[0], p[1]]) + 1e-3)
            && pts.iter().any(|p| point_in_ring([p[0], p[1]], &plan) || p[2] <= z([p[0], p[1]]) + 0.5);
        if !on { continue; }
        // Project vertically onto the roof plane, then into the face frame.
        let proj: Vec<P2> = pts.iter().map(|p| fr.to2([p[0], p[1], z([p[0], p[1]])])).collect();
        out.push(hull(proj));
    }
    out
}

/// Convex hull (monotone chain), counter-clockwise.
pub fn hull(mut p: Vec<P2>) -> Vec<P2> {
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 { return p; }
    let cr = |o: P2, a: P2, b: P2| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut h: Vec<P2> = vec![];
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &P2>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in it {
            while h.len() >= start + 2 && cr(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 { h.pop(); }
            h.push(q);
        }
        h.pop();
    }
    h
}

fn roof_face(scene: &Scene, face_id: &str) -> Result<Face, String> {
    let obj = face_id.split('/').next().unwrap_or("");
    let o = scene.object(obj).ok_or_else(|| format!("no object {obj} for face {face_id}"))?;
    mesh::faces(o).into_iter().find(|f| f.id == face_id).ok_or_else(|| format!("{obj} has no face {face_id}"))
}

/// Terrain height under a plan point (bilinear), if the terrain covers it.
pub fn terrain_z(scene: &Scene, terrain: &str, p: P2) -> Option<f64> {
    let Kind::Terrain { origin, cell, nx, ny, heights } = &scene.object(terrain)?.kind else { return None };
    let (fx, fy) = ((p[0] - origin[0]) / cell, (p[1] - origin[1]) / cell);
    if fx < 0.0 || fy < 0.0 || fx > (*nx - 1) as f64 || fy > (*ny - 1) as f64 { return None; }
    let (i, j) = ((fx.floor() as usize).min(nx - 2), (fy.floor() as usize).min(ny - 2));
    let (tx, ty) = (fx - i as f64, fy - j as f64);
    let h = |a: usize, b: usize| heights[b * nx + a];
    Some(h(i, j) * (1.0 - tx) * (1.0 - ty) + h(i + 1, j) * tx * (1.0 - ty) + h(i, j + 1) * (1.0 - tx) * ty + h(i + 1, j + 1) * tx * ty)
}

/// Place modules for a field. Identifiers are `<field>/m<NNNN>` in row-major order
/// (rows from the eave or front row, columns left to right seen from the front).
pub fn place(scene: &Scene, f: &ModuleField) -> Result<Vec<PlacedModule>, String> {
    let (l, w) = (f.module.length, f.module.width);
    if l <= 0.0 || w <= 0.0 { return Err("module length and width must be positive".into()); }
    let mut mods: Vec<(u32, u32, [V3; 4], V3, bool)> = vec![];
    match &f.mount {
        Mount::Roof { face, standoff } => {
            let roof = roof_face(scene, face)?;
            if !matches!(roof.role, Role::Roof | Role::Obstacle | Role::Imported) { return Err(format!("{face} is not a roof face")); }
            let fr = Frame::of(&roof);
            if fr.n[2] < 0.05 { return Err(format!("{face} faces down or is vertical; modules go on upward faces")); }
            let mut obstacles: Vec<Vec<P2>> = roof.holes.iter().map(|h| h.iter().map(|p| fr.to2(*p)).collect()).collect();
            obstacles.extend(keepouts(scene, &roof, &fr));
            let rf = RoofFace { outline: roof.outer.iter().map(|p| fr.to2(*p)).collect(), obstacle_heights: vec![1.0; obstacles.len()], obstacles };
            let lp = LayoutParams { module_length: l, module_width: w, orientation: f.layout.orientation, gap_x: f.layout.gap_x, gap_y: f.layout.gap_y, setback: f.layout.setback, obstacle_clearance: f.layout.obstacle_clearance, search_step: 0.05 };
            let lay = fill(&rf, &lp);
            let portrait = lay.orientation == Orientation::Portrait;
            let pitch_y = if portrait { l } else { w } + f.layout.gap_y;
            let y0 = lay.modules.iter().map(|r| r.y0).fold(f64::INFINITY, f64::min);
            for r in &lay.modules {
                let row = ((r.y0 - y0) / pitch_y).round() as u32;
                let c = [fr.to3([r.x0, r.y0], *standoff), fr.to3([r.x1, r.y0], *standoff), fr.to3([r.x1, r.y1], *standoff), fr.to3([r.x0, r.y1], *standoff)];
                mods.push((row, 0, c, fr.n, portrait));
            }
        }
        Mount::Ground { origin, azimuth_deg, tilt_deg, rows, tables_per_row, modules_across, modules_up, row_pitch, table_gap, clearance, terrain } => {
            if *rows == 0 || *tables_per_row == 0 || *modules_across == 0 || *modules_up == 0 { return Err("rows, tables and modules per table must be at least 1".into()); }
            let (b, a) = (tilt_deg.to_radians(), azimuth_deg.to_radians());
            let u = [-a.cos(), a.sin(), 0.0];
            let v = [-a.sin() * b.cos(), -a.cos() * b.cos(), b.sin()];
            let n = cross(u, v);
            let back = [-a.sin(), -a.cos(), 0.0];
            let gap = f.layout.gap_x;
            let table_w = *modules_across as f64 * (w + gap) - gap;
            let depth = *modules_up as f64 * (l + f.layout.gap_y) - f.layout.gap_y;
            if *row_pitch < depth * b.cos() { return Err(format!("row pitch {row_pitch} m is shorter than the table's plan depth {:.2} m", depth * b.cos())); }
            for r in 0..*rows {
                for t in 0..*tables_per_row {
                    let x0 = t as f64 * (table_w + table_gap);
                    let base2 = [origin[0] + u[0] * x0 + back[0] * r as f64 * row_pitch, origin[1] + u[1] * x0 + back[1] * r as f64 * row_pitch];
                    let gz = terrain.as_ref().and_then(|tid| terrain_z(scene, tid, [base2[0] + u[0] * table_w / 2.0, base2[1] + u[1] * table_w / 2.0])).unwrap_or(0.0);
                    let o = [base2[0], base2[1], gz + clearance];
                    for k in 0..*modules_up { for m in 0..*modules_across {
                        let (px, py) = (m as f64 * (w + gap), k as f64 * (l + f.layout.gap_y));
                        let at = |dx: f64, dy: f64| add(o, add(scale(u, px + dx), scale(v, py + dy)));
                        mods.push((r * modules_up + k, 0, [at(0.0, 0.0), at(w, 0.0), at(w, l), at(0.0, l)], n, true));
                    } }
                }
            }
        }
    }
    // Columns: order along the row axis within each row.
    let axis = |c: &[V3; 4]| sub(c[1], c[0]);
    let ax = mods.first().map(|m| norm(axis(&m.2))).unwrap_or([1.0, 0.0, 0.0]);
    mods.sort_by(|a, b| a.0.cmp(&b.0).then(dot(a.2[0], ax).total_cmp(&dot(b.2[0], ax))));
    let mut last_row = u32::MAX; let mut col = 0;
    Ok(mods.into_iter().enumerate().map(|(i, (row, _, corners, normal, portrait))| {
        if row != last_row { col = 0; last_row = row; } else { col += 1; }
        PlacedModule { id: format!("{}/m{:04}", f.id, i), corners, normal, row, col, portrait, string: None }
    }).collect())
}

/// Serpentine routing: rows in order, alternating direction, cut into strings
/// of `mps` modules. Jumper length follows the module centres; the home run
/// is the + conductor from the first module plus the − conductor from the
/// last module to the inverter (straight-line distances).
pub fn route_strings(f: &mut ModuleField, mps: u32) {
    let mut rows: Vec<u32> = f.modules.iter().map(|m| m.row).collect();
    rows.sort(); rows.dedup();
    let mut order: Vec<usize> = vec![];
    for (k, r) in rows.iter().enumerate() {
        let mut idx: Vec<usize> = (0..f.modules.len()).filter(|&i| f.modules[i].row == *r).collect();
        idx.sort_by_key(|&i| f.modules[i].col);
        if k % 2 == 1 { idx.reverse(); }
        order.extend(idx);
    }
    f.strings.clear();
    for m in &mut f.modules { m.string = None; }
    for (s, chunk) in order.chunks(mps as usize).enumerate() {
        let centers: Vec<V3> = chunk.iter().map(|&i| f.modules[i].center()).collect();
        let jumper = centers.windows(2).map(|w| dist(w[0], w[1])).sum();
        let home = f.inverter_at.map_or(0.0, |inv| dist(centers[0], inv) + dist(centers[centers.len() - 1], inv));
        for &i in chunk { f.modules[i].string = Some(s as u32); }
        f.strings.push(StringRoute { id: format!("{}/s{}", f.id, s), index: s as u32, modules: chunk.iter().map(|&i| f.modules[i].id.clone()).collect(), jumper_m: jumper, home_run_m: home });
    }
}

pub fn validate_field(s: &Scene, f: &ModuleField) -> Vec<Issue> {
    let mut out = vec![];
    let mk = |sev: &str, code: &str, m: String| Issue { severity: sev.into(), code: code.into(), subject: f.id.clone(), message: m };
    if let Mount::Roof { face, .. } = &f.mount {
        if let Err(e) = roof_face(s, face) { out.push(mk("error", "SCENE_FIELD_FACE", format!("{}: {e}", f.id))); }
    }
    if let Mount::Ground { terrain: Some(t), .. } = &f.mount {
        if !matches!(s.object(t).map(|o| &o.kind), Some(Kind::Terrain { .. })) { out.push(mk("error", "SCENE_FIELD_TERRAIN", format!("{}: terrain {t} not found", f.id))); }
    }
    if f.modules.is_empty() { out.push(mk("warning", "SCENE_FIELD_EMPTY", format!("{}: no modules placed", f.id))); }
    // Modules may not sit inside solids (a chimney moved after placement, an imported wall).
    let solids: Vec<(String, Vec<Face>)> = s.objects.iter().filter(|o| !o.hidden && !matches!(o.kind, Kind::Terrain { .. } | Kind::Mesh { .. })).map(|o| (o.id.clone(), mesh::faces(o))).collect();
    let mut hits = vec![];
    for m in &f.modules {
        let c = m.center();
        for (id, fs) in &solids {
            let (lo, hi) = fs.iter().flat_map(|x| x.outer.iter()).fold(([f64::MAX; 3], [f64::MIN; 3]), |(mut lo, mut hi), p| { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } (lo, hi) });
            if (0..3).any(|k| c[k] < lo[k] || c[k] > hi[k]) { continue; }
            if mesh::inside(fs, c) { hits.push(format!("{} inside {id}", m.id)); break; }
        }
    }
    if !hits.is_empty() { out.push(mk("error", "SCENE_MODULE_COLLISION", format!("{} module(s) collide with solids: {}; place the modules again", hits.len(), hits.iter().take(5).cloned().collect::<Vec<_>>().join(", ")))); }
    if let Some(last) = f.strings.last() {
        let first = f.strings.first().map_or(0, |s| s.modules.len());
        if last.modules.len() != first { out.push(mk("warning", "SCENE_STRING_INCOMPLETE", format!("{}: last string has {} modules, the others {first}", f.id, last.modules.len()))); }
    }
    out
}
