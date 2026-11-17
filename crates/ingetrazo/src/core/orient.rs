// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors (original Python).
// Copyright (C) 2026 SolarCraft contributors (Rust port).
//
// Upstream: core/orient.py (pin and hash in crates/ingetrazo/upstream.toml).
// Ported functions: `ray_parity_outside`, `is_closed`, `signed_volume`,
// `_incoherent_edges`. The crossing count is the vendored `parity::Tris`
// kernel, as upstream's native path uses. Differences from upstream:
// * meshes are given as faces of vertex indices (outer ring + holes) over a
//   welded vertex array instead of `core.mesh` objects;
// * the jittered rays come from a fixed xorshift sequence instead of
//   Python's `random.Random`, so individual rays differ while the vote rule
//   (majority of even crossings, `None` on a tie) is the same.
//! Solid orientation and closure queries.

use crate::parity::Tris;
use std::collections::HashMap;

pub type V3 = [f64; 3];

/// Upstream `_RAYS` and `_JITTER`.
const RAYS: usize = 5;
const JITTER: f64 = 0.05;

/// A face: outer ring then holes, as indices into a vertex array.
pub struct Face<'a> { pub rings: Vec<&'a [u32]> }

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn norm(a: V3) -> V3 { let l = dot(a, a).sqrt(); if l > 0.0 { [a[0] / l, a[1] / l, a[2] / l] } else { a } }

/// Upstream `_basis`: two unit vectors perpendicular to `n`.
fn basis(n: V3) -> (V3, V3) {
    let helper = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let u = norm(cross(n, helper));
    (u, cross(n, u))
}

/// Triangles packed for crossing counts; `face[i]` is the owning face index.
pub fn pack(pos: &[V3], tris: &[[u32; 3]], face: &[i64]) -> Tris {
    let a: Vec<V3> = tris.iter().map(|t| pos[t[0] as usize]).collect();
    let e1 = tris.iter().map(|t| sub(pos[t[1] as usize], pos[t[0] as usize])).collect();
    let e2 = tris.iter().map(|t| sub(pos[t[2] as usize], pos[t[0] as usize])).collect();
    Tris::new(a, e1, e2, face.to_vec())
}

/// Upstream `ray_parity_outside`: is `origin` outside the volume, by a majority
/// vote of crossing parity along jittered rays around `direction`.
/// `None` on a tie.
pub fn ray_parity_outside(origin: V3, direction: V3, packed: &Tris, skip: Option<i64>) -> Option<bool> {
    let n = norm(direction);
    let (u, v) = basis(n);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut uniform = || {
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;
        ((state >> 11) as f64 / (1u64 << 53) as f64) * 2.0 * JITTER - JITTER
    };
    let rays: Vec<V3> = (0..RAYS).map(|r| if r == 0 { n } else {
        let (a, b) = (uniform(), uniform());
        norm([n[0] + u[0] * a + v[0] * b, n[1] + u[1] * a + v[1] * b, n[2] + u[2] * a + v[2] * b])
    }).collect();
    let counts = packed.crossings(origin, &rays, None, skip);
    let outside = counts.iter().filter(|c| *c % 2 == 0).count();
    let inside = counts.len() - outside;
    if outside == inside { None } else { Some(outside > inside) }
}

/// Undirected edge uses of all rings, keyed by vertex pair.
fn edge_uses(faces: &[Face]) -> HashMap<(u32, u32), Vec<(u32, u32)>> {
    let mut uses: HashMap<(u32, u32), Vec<(u32, u32)>> = HashMap::new();
    for f in faces {
        for ring in &f.rings {
            let n = ring.len();
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                if a == b { continue; }
                uses.entry((a.min(b), a.max(b))).or_default().push((a, b));
            }
        }
    }
    uses
}

/// Upstream `is_closed`: every edge borders at least two faces.
pub fn is_closed(faces: &[Face]) -> bool {
    !faces.is_empty() && edge_uses(faces).values().all(|u| u.len() >= 2)
}

/// Edges bordering only one face (the boundary of an open shell).
pub fn boundary_edges(faces: &[Face]) -> usize { edge_uses(faces).values().filter(|u| u.len() == 1).count() }

/// Upstream `_incoherent_edges`: shared edges that two faces walk the same way.
pub fn incoherent_edges(faces: &[Face]) -> usize {
    edge_uses(faces).values().filter(|u| u.len() == 2 && u[0] == u[1]).count()
}

/// Upstream `signed_volume` over triangles: positive when wound outward.
pub fn signed_volume(pos: &[V3], tris: &[[u32; 3]]) -> f64 {
    tris.iter().map(|t| dot(pos[t[0] as usize], cross(pos[t[1] as usize], pos[t[2] as usize])) / 6.0).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cube() -> (Vec<V3>, Vec<Vec<u32>>) {
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0]];
        let f = vec![vec![0, 3, 2, 1], vec![4, 5, 6, 7], vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6], vec![3, 0, 4, 7]];
        (p, f)
    }
    fn tris(f: &[Vec<u32>]) -> (Vec<[u32; 3]>, Vec<i64>) {
        let mut t = vec![]; let mut o = vec![];
        for (i, r) in f.iter().enumerate() { for k in 1..r.len() - 1 { t.push([r[0], r[k], r[k + 1]]); o.push(i as i64); } }
        (t, o)
    }
    #[test]
    fn cube_is_closed_outward_and_unit_volume() {
        let (p, f) = cube();
        let faces: Vec<Face> = f.iter().map(|r| Face { rings: vec![r.as_slice()] }).collect();
        assert!(is_closed(&faces));
        assert_eq!(incoherent_edges(&faces), 0);
        let (t, o) = tris(&f);
        assert!((signed_volume(&p, &t) - 1.0).abs() < 1e-12);
        let packed = pack(&p, &t, &o);
        assert_eq!(ray_parity_outside([0.5, 0.5, 0.5], [0.3, 0.2, 1.0], &packed, None), Some(false));
        assert_eq!(ray_parity_outside([2.0, 0.5, 0.5], [1.0, 0.0, 0.0], &packed, None), Some(true));
        // A face centroid nudged along its outward normal is outside.
        assert_eq!(ray_parity_outside([0.5, 0.5, 1.001], [0.0, 0.0, 1.0], &packed, Some(1)), Some(true));
    }
    #[test]
    fn open_box_and_flipped_face_are_reported() {
        let (_, mut f) = cube();
        f.pop();
        let faces: Vec<Face> = f.iter().map(|r| Face { rings: vec![r.as_slice()] }).collect();
        assert!(!is_closed(&faces));
        assert_eq!(boundary_edges(&faces), 4);
        let (_, mut f) = cube();
        f[1].reverse();
        let faces: Vec<Face> = f.iter().map(|r| Face { rings: vec![r.as_slice()] }).collect();
        assert_eq!(incoherent_edges(&faces), 4);
    }
}
