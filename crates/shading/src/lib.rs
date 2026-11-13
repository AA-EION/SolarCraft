// SPDX-License-Identifier: GPL-3.0-or-later
//! Ray-cast shading: binned-SAH BVH with any-hit traversal and
//! Möller–Trumbore intersection (validated in docs/spikes/C-shading.md).
//! Coordinates: x east, y north, z up, metres.
pub type V3 = [f64; 3];
pub fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
pub fn add(a: V3, b: V3) -> V3 { [a[0] + b[0], a[1] + b[1], a[2] + b[2]] }
pub fn mul(a: V3, s: f64) -> V3 { [a[0] * s, a[1] * s, a[2] * s] }
pub fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
pub fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }

#[derive(Clone, Copy, Debug)]
struct Tri { v0: V3, e1: V3, e2: V3, owner: u32 }

#[derive(Clone, Copy)]
struct Node { lo: V3, hi: V3, left: u32, count: u32 } // count>0: leaf over tris[left..left+count]

pub struct Bvh { nodes: Vec<Node>, tris: Vec<Tri> }

impl Bvh {
    fn build(mut tris: Vec<Tri>) -> Self {
        let mut nodes = Vec::with_capacity(tris.len() * 2);
        nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], left: 0, count: 0 });
        let n = tris.len();
        Self::split(&mut nodes, &mut tris, 0, 0, n);
        Bvh { nodes, tris }
    }
    fn bounds(t: &[Tri]) -> (V3, V3) {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for tr in t { for p in [tr.v0, add(tr.v0, tr.e1), add(tr.v0, tr.e2)] { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } } }
        (lo, hi)
    }
    fn split(nodes: &mut Vec<Node>, tris: &mut [Tri], idx: usize, start: usize, end: usize) {
        let (lo, hi) = Self::bounds(&tris[start..end]);
        nodes[idx].lo = lo; nodes[idx].hi = hi;
        if end - start <= 4 { nodes[idx].left = start as u32; nodes[idx].count = (end - start) as u32; return; }
        // Binned surface-area heuristic (Wald 2007), 16 bins on the centroid extent.
        let cen = |t: &Tri, k: usize| t.v0[k] + (t.e1[k] + t.e2[k]) / 3.0;
        let area = |lo: V3, hi: V3| { let d = sub(hi, lo); d[0] * d[1] + d[1] * d[2] + d[2] * d[0] };
        let mut best = (f64::INFINITY, 0usize, 0.0f64);
        for ax in 0..3 {
            let (mut cl, mut ch) = (f64::MAX, f64::MIN);
            for t in &tris[start..end] { let c = cen(t, ax); cl = cl.min(c); ch = ch.max(c); }
            if ch - cl < 1e-12 { continue; }
            const B: usize = 16;
            let mut bins = [([f64::MAX; 3], [f64::MIN; 3], 0usize); B];
            for t in &tris[start..end] {
                let b = (((cen(t, ax) - cl) / (ch - cl) * B as f64) as usize).min(B - 1);
                let (bl, bh) = Self::bounds(std::slice::from_ref(t));
                for k in 0..3 { bins[b].0[k] = bins[b].0[k].min(bl[k]); bins[b].1[k] = bins[b].1[k].max(bh[k]); }
                bins[b].2 += 1;
            }
            for split in 1..B {
                let acc = |r: std::ops::Range<usize>| { let (mut l, mut h, mut n) = ([f64::MAX; 3], [f64::MIN; 3], 0); for b in &bins[r] { if b.2 > 0 { for k in 0..3 { l[k] = l[k].min(b.0[k]); h[k] = h[k].max(b.1[k]); } n += b.2; } } (l, h, n) };
                let (l1, h1, n1) = acc(0..split); let (l2, h2, n2) = acc(split..B);
                if n1 == 0 || n2 == 0 { continue; }
                let cost = area(l1, h1) * n1 as f64 + area(l2, h2) * n2 as f64;
                if cost < best.0 { best = (cost, ax, cl + (ch - cl) * split as f64 / B as f64); }
            }
        }
        let mut mid = start;
        if best.0.is_finite() {
            let (ax, pos) = (best.1, best.2);
            for i in start..end { if cen(&tris[i], ax) < pos { tris.swap(i, mid); mid += 1; } }
        }
        if mid == start || mid == end {
            mid = (start + end) / 2;
            let ext = sub(hi, lo);
            let ax = if ext[0] >= ext[1] && ext[0] >= ext[2] { 0 } else if ext[1] >= ext[2] { 1 } else { 2 };
            tris[start..end].select_nth_unstable_by(mid - start, |a, b| cen(a, ax).partial_cmp(&cen(b, ax)).unwrap());
        }
        let l = nodes.len();
        nodes.push(Node { lo, hi, left: 0, count: 0 });
        nodes.push(Node { lo, hi, left: 0, count: 0 });
        nodes[idx].left = l as u32; nodes[idx].count = 0;
        Self::split(nodes, tris, l, start, mid);
        Self::split(nodes, tris, l + 1, mid, end);
    }
    /// True if a ray from `o` along `d` hits any triangle not owned by `skip`.
    pub fn occluded(&self, o: V3, d: V3, skip: u32) -> bool {
        let inv = [1.0 / d[0], 1.0 / d[1], 1.0 / d[2]];
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            let (mut t0, mut t1) = (0.0f64, f64::INFINITY);
            for k in 0..3 {
                let (a, b) = ((n.lo[k] - o[k]) * inv[k], (n.hi[k] - o[k]) * inv[k]);
                t0 = t0.max(a.min(b)); t1 = t1.min(a.max(b));
            }
            if t0 > t1 { continue; }
            if n.count > 0 {
                for t in &self.tris[n.left as usize..(n.left + n.count) as usize] {
                    if t.owner != skip && hit(t, o, d) { return true; }
                }
            } else {
                stack[sp] = n.left; stack[sp + 1] = n.left + 1; sp += 2;
            }
        }
        false
    }
}

fn hit(t: &Tri, o: V3, d: V3) -> bool {
    let p = cross(d, t.e2);
    let det = dot(t.e1, p);
    if det.abs() < 1e-12 { return false; }
    let inv = 1.0 / det;
    let s = sub(o, t.v0);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) { return false; }
    let q = cross(s, t.e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 { return false; }
    dot(t.e2, q) * inv > 1e-9
}


/// A triangle for scene construction; `owner` identifies the object so a
/// surface does not shade itself.
#[derive(Clone, Copy, Debug)]
pub struct Triangle { pub a: V3, pub b: V3, pub c: V3, pub owner: u32 }

impl Bvh {
    pub fn from_triangles(t: &[Triangle]) -> Self {
        Self::build(t.iter().map(|t| Tri { v0: t.a, e1: sub(t.b, t.a), e2: sub(t.c, t.a), owner: t.owner }).collect())
    }
    pub fn is_empty(&self) -> bool { self.tris.is_empty() }
}

/// Unit vector toward the sun (elevation and azimuth in degrees, azimuth clockwise from north).
pub fn sun_dir(elev: f64, az: f64) -> V3 {
    let (e, a) = (elev.to_radians(), az.to_radians());
    [e.cos() * a.sin(), e.cos() * a.cos(), e.sin()]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wall_shadow_matches_geometry() {
        // 3 m wall along x at y=0; point 2 m north of it on the ground; sun due south.
        let w = [Triangle { a: [-50.0, 0.0, 0.0], b: [50.0, 0.0, 0.0], c: [50.0, 0.0, 3.0], owner: 1 }, Triangle { a: [-50.0, 0.0, 0.0], b: [50.0, 0.0, 3.0], c: [-50.0, 0.0, 3.0], owner: 1 }];
        let bvh = Bvh::from_triangles(&w);
        // Shadow length 3/tan(el): shaded at el=45° (3 m > 2 m), lit at el=60° (1.73 m < 2 m).
        assert!(bvh.occluded([0.0, 2.0, 0.0], sun_dir(45.0, 180.0), 0));
        assert!(!bvh.occluded([0.0, 2.0, 0.0], sun_dir(60.0, 180.0), 0));
        assert!(!bvh.occluded([0.0, 2.0, 0.0], sun_dir(45.0, 180.0), 1), "owner skip");
    }
}
