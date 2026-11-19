// SPDX-License-Identifier: GPL-3.0-or-later
//! Faces generated from scene objects, their triangulation (IngeTrazo
//! `facegeom`) and solid checks (IngeTrazo `weld` + `core::orient`).
//!
//! Every face is planar, wound counter-clockwise seen from outside, and
//! carries a stable identifier derived from its object and role.
use crate::{Kind, Object, Roof, P2, V3};
use serde::{Deserialize, Serialize};
use solarcraft_ingetrazo::core::orient;
use solarcraft_ingetrazo::facegeom::{face_geometry, Hypot, V3 as F3};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role { Roof, Wall, Floor, Opening, Obstacle, Crown, Trunk, Terrain, Imported }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub id: String,
    pub object: String,
    pub role: Role,
    pub outer: Vec<V3>,
    #[serde(default)]
    pub holes: Vec<Vec<V3>>,
    /// Beam fraction blocked (1 opaque; tree crowns 1 − transparency).
    pub opacity: f64,
}

pub fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
pub fn scale(a: V3, s: f64) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
pub fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
pub fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
pub fn norm(a: V3) -> V3 { let l = dot(a, a).sqrt(); if l > 0.0 { scale(a, 1.0 / l) } else { a } }
pub fn dist(a: V3, b: V3) -> f64 { dot(sub(a, b), sub(a, b)).sqrt() }

/// Newell normal (unit) of a ring.
pub fn normal(ring: &[V3]) -> V3 {
    let mut n = [0.0; 3];
    for k in 0..ring.len() {
        let (c, d) = (ring[k], ring[(k + 1) % ring.len()]);
        n[0] += (c[1] - d[1]) * (c[2] + d[2]);
        n[1] += (c[2] - d[2]) * (c[0] + d[0]);
        n[2] += (c[0] - d[0]) * (c[1] + d[1]);
    }
    norm(n)
}

pub fn area(face: &Face) -> f64 { triangulate(face).iter().map(|t| 0.5 * dot(cross(sub(t[1], t[0]), sub(t[2], t[0])), cross(sub(t[1], t[0]), sub(t[2], t[0]))).sqrt()).sum() }

fn plan_area(p: &[P2]) -> f64 { solarcraft_geometry::poly::ring_area(p) }
fn ccw(p: &[P2]) -> Vec<P2> { let mut v = p.to_vec(); if plan_area(&v) < 0.0 { v.reverse(); } v }

/// Plane through three non-collinear ring points, as z = f(x, y); None if vertical.
pub fn plane_z(ring: &[V3]) -> Option<impl Fn(P2) -> f64> {
    let n = normal(ring);
    if n[2].abs() < 1e-9 { return None; }
    let o = ring[0];
    Some(move |p: P2| o[2] - (n[0] * (p[0] - o[0]) + n[1] * (p[1] - o[1])) / n[2])
}

fn face(object: &str, suffix: &str, role: Role, outer: Vec<V3>, opacity: f64) -> Face {
    Face { id: format!("{object}/{suffix}"), object: object.into(), role, outer, holes: vec![], opacity }
}

/// Walls, floor and a roof over `footprint` (CCW) given a roof-height function.
fn prism(id: &str, fp: &[P2], base: f64, top: &dyn Fn(P2) -> f64, wall_role: Role, top_role: Role, out: &mut Vec<Face>) {
    let n = fp.len();
    for i in 0..n {
        let (a, b) = (fp[i], fp[(i + 1) % n]);
        out.push(face(id, &format!("wall-{i}"), wall_role, vec![[a[0], a[1], base], [b[0], b[1], base], [b[0], b[1], top(b)], [a[0], a[1], top(a)]], 1.0));
    }
    let bottom = if wall_role == Role::Wall { Role::Floor } else { wall_role };
    out.push(face(id, "floor", bottom, fp.iter().rev().map(|p| [p[0], p[1], base]).collect(), 1.0));
    out.push(face(id, "roof-0", top_role, fp.iter().map(|p| [p[0], p[1], top(*p)]).collect(), 1.0));
}

fn circle(c: P2, r: f64, n: u32) -> Vec<P2> {
    (0..n).map(|k| { let a = k as f64 / n as f64 * std::f64::consts::TAU; [c[0] + r * a.cos(), c[1] + r * a.sin()] }).collect()
}

/// Building with a gable or hip roof over a rectangle (validated before).
fn pitched(id: &str, fp: &[P2], base: f64, eave: f64, pitch: f64, hip: bool, setback: Option<f64>, out: &mut Vec<Face>) {
    let mut p = ccw(fp);
    let len = |a: P2, b: P2| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    // Ridge along the first edge for gables; along the longer side for hips.
    if hip && len(p[1], p[2]) > len(p[0], p[1]) { p.rotate_left(1); }
    let (la, lb) = (len(p[0], p[1]), len(p[1], p[2]));
    let t = pitch.to_radians().tan();
    let top = base + eave;
    let h = top + lb / 2.0 * t;
    let b3 = |q: P2, z: f64| [q[0], q[1], z];
    let mid = |a: P2, b: P2| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (m03, m12) = (mid(p[0], p[3]), mid(p[1], p[2]));
    let (bb, tt): (Vec<V3>, Vec<V3>) = (p.iter().map(|q| b3(*q, base)).collect(), p.iter().map(|q| b3(*q, top)).collect());
    for i in 0..4 {
        let j = (i + 1) % 4;
        let mut ring = vec![bb[i], bb[j], tt[j], tt[i]];
        if !hip && (i == 1 || i == 3) {
            // Gable end: the wall rises to the ridge.
            let r = if i == 1 { m12 } else { m03 };
            ring.insert(3, b3(r, h));
        }
        out.push(face(id, &format!("wall-{i}"), Role::Wall, ring, 1.0));
    }
    out.push(face(id, "floor", Role::Floor, bb.iter().rev().copied().collect(), 1.0));
    if !hip {
        let (r0, r1) = (b3(m03, h), b3(m12, h));
        out.push(face(id, "roof-0", Role::Roof, vec![tt[0], tt[1], r1, r0], 1.0));
        out.push(face(id, "roof-1", Role::Roof, vec![tt[2], tt[3], r0, r1], 1.0));
    } else {
        let d = setback.unwrap_or(lb / 2.0);
        let ua = [(p[1][0] - p[0][0]) / la, (p[1][1] - p[0][1]) / la];
        let r0 = b3([m03[0] + ua[0] * d, m03[1] + ua[1] * d], h);
        let r1 = b3([m12[0] - ua[0] * d, m12[1] - ua[1] * d], h);
        if (la - 2.0 * d).abs() < 1e-9 {
            out.push(face(id, "roof-0", Role::Roof, vec![tt[0], tt[1], r0], 1.0));
            out.push(face(id, "roof-1", Role::Roof, vec![tt[1], tt[2], r0], 1.0));
            out.push(face(id, "roof-2", Role::Roof, vec![tt[2], tt[3], r0], 1.0));
            out.push(face(id, "roof-3", Role::Roof, vec![tt[3], tt[0], r0], 1.0));
        } else {
            out.push(face(id, "roof-0", Role::Roof, vec![tt[0], tt[1], r1, r0], 1.0));
            out.push(face(id, "roof-1", Role::Roof, vec![tt[1], tt[2], r1], 1.0));
            out.push(face(id, "roof-2", Role::Roof, vec![tt[2], tt[3], r0, r1], 1.0));
            out.push(face(id, "roof-3", Role::Roof, vec![tt[3], tt[0], r0], 1.0));
        }
    }
}

/// Cut plan-polygon openings out of the roof faces that contain them.
fn cut_openings(id: &str, openings: &[Vec<P2>], out: &mut [Face]) -> Vec<Face> {
    use solarcraft_geometry::poly::point_in_ring;
    let mut extra = vec![];
    for (k, op) in openings.iter().enumerate() {
        if op.len() < 3 { continue; }
        let op = ccw(op);
        for f in out.iter_mut().filter(|f| f.role == Role::Roof) {
            let plan: Vec<P2> = f.outer.iter().map(|p| [p[0], p[1]]).collect();
            if !op.iter().all(|&p| point_in_ring(p, &plan)) { continue; }
            let Some(z) = plane_z(&f.outer) else { continue };
            let ring: Vec<V3> = op.iter().map(|p| [p[0], p[1], z(*p)]).collect();
            f.holes.push(ring.iter().rev().copied().collect());
            extra.push(Face { id: format!("{id}/opening-{k}"), object: id.into(), role: Role::Opening, outer: ring, holes: vec![], opacity: 1.0 });
            break;
        }
    }
    extra
}

/// Closed lathe (crown): rings of an ellipsoid from bottom pole to top pole.
fn ellipsoid(id: &str, c: V3, r: f64, hz: f64, seg: u32, opacity: f64, out: &mut Vec<Face>) {
    let lat = (seg / 4).max(3);
    let ring = |k: u32| -> Vec<V3> {
        let phi = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / lat as f64;
        circle([c[0], c[1]], r * phi.cos(), seg).into_iter().map(|p| [p[0], p[1], c[2] + hz * phi.sin()]).collect()
    };
    let (bottom, top) = ([c[0], c[1], c[2] - hz], [c[0], c[1], c[2] + hz]);
    for k in 1..lat {
        let lo = ring(k);
        if k == 1 { for i in 0..seg as usize { let j = (i + 1) % seg as usize; out.push(face(id, &format!("crown-b{i}"), Role::Crown, vec![bottom, lo[j], lo[i]], opacity)); } }
        if k + 1 < lat {
            let hi = ring(k + 1);
            for i in 0..seg as usize { let j = (i + 1) % seg as usize; out.push(face(id, &format!("crown-{k}-{i}"), Role::Crown, vec![lo[i], lo[j], hi[j], hi[i]], opacity)); }
        } else {
            for i in 0..seg as usize { let j = (i + 1) % seg as usize; out.push(face(id, &format!("crown-t{i}"), Role::Crown, vec![lo[i], lo[j], top], opacity)); }
        }
    }
}

/// All faces of an object.
pub fn faces(o: &Object) -> Vec<Face> {
    let id = o.id.as_str();
    let mut out = vec![];
    match &o.kind {
        Kind::Building { footprint, base_z, eave_height, roof, openings } => {
            if footprint.len() < 3 { return out; }
            let fp = ccw(footprint);
            let (base, top) = (*base_z, base_z + eave_height);
            match roof {
                Roof::Flat => prism(id, &fp, base, &|_| top, Role::Wall, Role::Roof, &mut out),
                Roof::Shed { pitch_deg, facing_deg } => {
                    let (s, c) = (facing_deg.to_radians().sin(), facing_deg.to_radians().cos());
                    let t = pitch_deg.to_radians().tan();
                    let maxp = fp.iter().map(|p| p[0] * s + p[1] * c).fold(f64::MIN, f64::max);
                    prism(id, &fp, base, &|p: P2| top + (maxp - (p[0] * s + p[1] * c)) * t, Role::Wall, Role::Roof, &mut out)
                }
                Roof::Gable { pitch_deg } if fp.len() == 4 => pitched(id, &fp, base, *eave_height, *pitch_deg, false, None, &mut out),
                Roof::Hip { pitch_deg } if fp.len() == 4 => pitched(id, &fp, base, *eave_height, *pitch_deg, true, None, &mut out),
                Roof::HipWithSetback { pitch_deg, ridge_setback_m } if fp.len() == 4 => {
                    pitched(id, &fp, base, *eave_height, *pitch_deg, true, Some(*ridge_setback_m), &mut out)
                }
                _ => prism(id, &fp, base, &|_| top, Role::Wall, Role::Roof, &mut out),
            }
            let extra = cut_openings(id, openings, &mut out);
            out.extend(extra);
        }
        Kind::Dormer { profile } => {
            let Some(depth) = profile.depth() else { return out; };
            let [a, b, c, d] = profile.footprint(depth);
            let low = profile.base_z;
            let front = low + profile.front_height;
            let back = low + depth * profile.base_pitch_deg.to_radians().tan();
            let (a0, b0, a1, b1, c1, d1) = (
                [a[0], a[1], low], [b[0], b[1], low], [a[0], a[1], front], [b[0], b[1], front],
                [c[0], c[1], back], [d[0], d[1], back],
            );
            out.push(face(id, "wall-0", Role::Wall, vec![a0, b0, b1, a1], 1.0));
            out.push(face(id, "wall-1", Role::Wall, vec![b0, c1, b1], 1.0));
            out.push(face(id, "wall-2", Role::Wall, vec![d1, a0, a1], 1.0));
            out.push(face(id, "floor", Role::Floor, vec![d1, c1, b0, a0], 1.0));
            out.push(face(id, "roof-0", Role::Roof, vec![a1, b1, c1, d1], 1.0));
        }
        Kind::Block { center, base_z, length, width, height, rotation_deg } => {
            let (s, c) = (rotation_deg.to_radians().sin(), rotation_deg.to_radians().cos());
            let fp: Vec<P2> = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]].iter().map(|q: &[f64; 2]| {
                let (x, y) = (q[0] * length, q[1] * width);
                [center[0] + x * c - y * s, center[1] + x * s + y * c]
            }).collect();
            prism(id, &fp, *base_z, &|_| base_z + height, Role::Obstacle, Role::Obstacle, &mut out);
        }
        Kind::Cylinder { center, base_z, radius, height, segments } => {
            prism(id, &circle(*center, *radius, (*segments).max(3)), *base_z, &|_| base_z + height, Role::Obstacle, Role::Obstacle, &mut out);
        }
        Kind::Tree { center, base_z, trunk_height, trunk_radius, crown_radius, crown_height, transparency, segments } => {
            let seg = (*segments).max(3);
            if *trunk_height > 0.0 {
                let mut trunk = vec![];
                prism(id, &circle(*center, *trunk_radius, seg), *base_z, &|_| base_z + trunk_height, Role::Trunk, Role::Trunk, &mut trunk);
                for f in &mut trunk { f.id = f.id.replacen(&format!("{id}/"), &format!("{id}/trunk-"), 1); }
                out.extend(trunk);
            }
            let c = [center[0], center[1], base_z + trunk_height + crown_height / 2.0];
            ellipsoid(id, c, *crown_radius, crown_height / 2.0, seg, 1.0 - transparency.clamp(0.0, 1.0), &mut out);
        }
        Kind::Terrain { origin, cell, nx, ny, heights } => {
            if heights.len() != nx * ny { return out; }
            let p = |i: usize, j: usize| [origin[0] + i as f64 * cell, origin[1] + j as f64 * cell, heights[j * nx + i]];
            for j in 0..ny.saturating_sub(1) { for i in 0..nx.saturating_sub(1) {
                out.push(face(id, &format!("t{i}-{j}-a"), Role::Terrain, vec![p(i, j), p(i + 1, j), p(i + 1, j + 1)], 1.0));
                out.push(face(id, &format!("t{i}-{j}-b"), Role::Terrain, vec![p(i, j), p(i + 1, j + 1), p(i, j + 1)], 1.0));
            } }
        }
        Kind::Mesh { faces, opacity } => {
            for (k, f) in faces.iter().enumerate() {
                if f.outer.len() < 3 { continue; }
                out.push(Face { id: format!("{id}/f{k}"), object: id.into(), role: Role::Imported, outer: f.outer.clone(), holes: f.holes.clone(), opacity: *opacity });
            }
        }
    }
    out
}

/// Triangles of a planar face with holes (IngeTrazo `facegeom::face_geometry`).
/// Coordinates are taken relative to the first vertex before the float32
/// kernel sees them, so faces far from the origin keep their precision.
pub fn triangulate(f: &Face) -> Vec<[V3; 3]> {
    if f.outer.len() < 3 { return vec![]; }
    let o = f.outer[0];
    let mut pos: Vec<V3> = f.outer.clone();
    let mut rings: Vec<Vec<u32>> = vec![(0..f.outer.len() as u32).collect()];
    for h in &f.holes {
        if h.len() < 3 { continue; }
        let s = pos.len() as u32;
        pos.extend(h.iter().copied());
        rings.push((s..s + h.len() as u32).collect());
    }
    let p32: Vec<F3> = pos.iter().map(|p| F3((p[0] - o[0]) as f32, (p[1] - o[1]) as f32, (p[2] - o[2]) as f32)).collect();
    let rr: Vec<&[u32]> = rings.iter().map(|r| r.as_slice()).collect();
    let g = face_geometry(&p32, &rr, Hypot::LibStdCxx, true);
    let n = normal(&f.outer);
    g.tris.iter().map(|t| {
        let tri = [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]];
        // Keep the face's winding (the kernel triangulates in its own plane basis).
        if dot(cross(sub(tri[1], tri[0]), sub(tri[2], tri[0])), n) < 0.0 { [tri[0], tri[2], tri[1]] } else { tri }
    }).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SolidCheck { pub boundary_edges: usize, pub incoherent_edges: usize, pub volume: f64 }

/// Weld the faces' vertices (IngeTrazo `weld`, 0.1 mm cells) and check that
/// they bound a closed, consistently wound solid of positive volume
/// (IngeTrazo `core::orient`).
pub fn check_solid(faces: &[Face]) -> SolidCheck {
    if faces.is_empty() { return SolidCheck { boundary_edges: 0, incoherent_edges: 0, volume: 0.0 }; }
    let o = faces[0].outer[0];
    let mut pts: Vec<[f32; 3]> = vec![];
    let mut rings: Vec<Vec<Vec<usize>>> = vec![];
    for f in faces {
        let mut fr = vec![];
        for r in std::iter::once(&f.outer).chain(f.holes.iter()) {
            let s = pts.len();
            pts.extend(r.iter().map(|p| [(p[0] - o[0]) as f32, (p[1] - o[1]) as f32, (p[2] - o[2]) as f32]));
            fr.push((s..s + r.len()).collect());
        }
        rings.push(fr);
    }
    let (_, map, _) = solarcraft_ingetrazo::weld::weld(&pts, 1e4, 1e-8);
    let idx: Vec<Vec<Vec<u32>>> = rings.iter().map(|fr| fr.iter().map(|r| {
        let mut v: Vec<u32> = r.iter().map(|&i| map[i] as u32).collect();
        v.dedup();
        while v.len() > 1 && v.first() == v.last() { v.pop(); }
        v
    }).collect()).collect();
    let of: Vec<orient::Face> = idx.iter().map(|fr| orient::Face { rings: fr.iter().map(|r| r.as_slice()).collect() }).collect();
    let volume: f64 = faces.iter().flat_map(triangulate).map(|t| dot(sub(t[0], o), cross(sub(t[1], o), sub(t[2], o))) / 6.0).sum();
    SolidCheck { boundary_edges: orient::boundary_edges(&of), incoherent_edges: orient::incoherent_edges(&of), volume }
}

/// Point-in-solid test by ray parity (IngeTrazo `ray_parity_outside`).
pub fn inside(faces: &[Face], p: V3) -> bool {
    let tris: Vec<[V3; 3]> = faces.iter().flat_map(triangulate).collect();
    if tris.is_empty() { return false; }
    let pos: Vec<V3> = tris.iter().flatten().copied().collect();
    let idx: Vec<[u32; 3]> = (0..tris.len() as u32).map(|k| [3 * k, 3 * k + 1, 3 * k + 2]).collect();
    let face: Vec<i64> = (0..tris.len() as i64).collect();
    let packed = orient::pack(&pos, &idx, &face);
    orient::ray_parity_outside(p, [0.31, 0.17, 0.93], &packed, None) == Some(false)
}
